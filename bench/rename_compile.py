# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Side-by-side `textDocument/rename` apply+compile checker for `ridx` and `rust-analyzer` across all 7 corpora.

Evaluates:
1. Answered compile rate (`ridx` vs `rust-analyzer` on the same positions, optionally sampled with `--limit N`).
2. Full-corpus refusal audit across ALL refusals in the corpus (never capped by `--limit`):
   - `refused_where_ra_external`: RA's edit touches files outside `workspace_root` (std/registry).
   - `refused_where_ra_broke_build`: RA's edit stays in-repo, but applying it breaks `mitts check`
     (reported as a standalone counter and never mixed into `position_ab_parity` rules).
   - `refused_where_ra_compiled`: RA's edit stays in-repo AND compiles cleanly under `mitts check`.

Usage:
  1. Single corpus from `.raw.pkl`:
       python3 bench/rename_compile.py CORPUS PKL [N]
  2. All 7 corpora from a directory of `.raw.pkl` files:
       python3 bench/rename_compile.py --all-corpora --pkl-dir DIR [--limit N] [--out JSON]
"""

import argparse
import hashlib
import json
import os
import pickle
import random
import shutil
import subprocess
import sys
import urllib.parse
from pathlib import Path
from typing import Dict, List, Optional, Tuple

sys.path.insert(0, str(Path(__file__).resolve().parent))
from position_ab_parity_refusal import workspace_edit_external_paths  # noqa: E402

CORPORA = ("serde", "ripgrep", "tokio", "rust-analyzer", "bevy", "clap", "regex")


def resolve_mitts_binary(override: Optional[str] = None) -> Tuple[str, str]:
    cand = override or os.environ.get("MITTS") or shutil.which("mitts")
    if not cand:
        raise RuntimeError("mitts binary not found. Specify --mitts PATH or set MITTS=/path/to/mitts")
    cand_path = os.path.expanduser(cand)
    if not os.path.isfile(cand_path) or not os.access(cand_path, os.X_OK):
        raise RuntimeError(f"Configured mitts binary is not executable: {cand_path}")
    r = subprocess.run([cand_path, "--version"], capture_output=True, text=True, check=True)
    version_line = r.stdout.strip() or r.stderr.strip()
    return cand_path, version_line


def edits_of(res: Optional[dict]) -> Optional[Dict[str, list] | str]:
    if not isinstance(res, dict):
        return None
    out: Dict[str, list] = {}
    for uri, es in (res.get("changes") or {}).items():
        if es:
            out.setdefault(uri, []).extend(es)
    for dc in res.get("documentChanges") or []:
        if not isinstance(dc, dict):
            continue
        if "textDocument" in dc:
            uri = (dc.get("textDocument") or {}).get("uri", "")
            if uri and uri not in out and dc.get("edits"):
                out.setdefault(uri, []).extend(dc.get("edits") or [])
        elif dc.get("kind") == "rename":
            return "fileop"
    return out


def _utf16_col_to_char_idx(line_str: str, u16_target: int) -> int:
    u16_cur = 0
    c_idx = 0
    for ch in line_str:
        if u16_cur >= u16_target:
            break
        u16_cur += 2 if ord(ch) > 0xFFFF else 1
        c_idx += 1
    return c_idx


def apply_edits_in_worktree(eds: Dict[str, list], src_dir: str, wt_dir: str) -> bool:
    src_real = os.path.realpath(src_dir).rstrip("/")
    wt_real = os.path.realpath(wt_dir).rstrip("/")
    for uri, es in eds.items():
        if not uri.startswith("file://"):
            return False
        raw_p = urllib.parse.unquote(uri[7:])
        real_p = os.path.realpath(raw_p)
        if real_p.startswith(src_real + "/"):
            target_p = wt_real + real_p[len(src_real):]
        elif raw_p.startswith(src_dir.rstrip("/") + "/"):
            target_p = wt_real + raw_p[len(src_dir.rstrip("/")):]
        elif real_p.startswith(wt_real + "/"):
            target_p = real_p
        else:
            return False
        if not os.path.isfile(target_p):
            return False
        with open(target_p, encoding="utf-8") as fh:
            txt = fh.read()
        lines = txt.split("\n")
        starts = [0]
        for ln in lines:
            starts.append(starts[-1] + len(ln) + 1)

        def off(pos: dict) -> int:
            l_idx = min(max(0, int(pos.get("line", 0))), len(lines) - 1)
            c_idx = _utf16_col_to_char_idx(lines[l_idx], int(pos.get("character", 0)))
            return starts[l_idx] + c_idx

        seen_spans = set()
        unique_es = []
        for e in es:
            a, b = off(e["range"]["start"]), off(e["range"]["end"])
            if (a, b) not in seen_spans:
                seen_spans.add((a, b))
                unique_es.append((a, b, str(e.get("newText", ""))))
        for a, b, new_text in sorted(unique_es, key=lambda x: x[0], reverse=True):
            txt = txt[:a] + new_text + txt[b:]
        with open(target_p, "w", encoding="utf-8") as fh:
            fh.write(txt)
    return True


def _edit_signature(eds: Dict[str, list]) -> str:
    h = hashlib.sha256()
    for uri in sorted(eds.keys()):
        h.update(uri.encode("utf-8"))
        for e in sorted(
            eds[uri],
            key=lambda x: (int(x["range"]["start"]["line"]), int(x["range"]["start"]["character"])),
        ):
            h.update(json.dumps(e, sort_keys=True).encode("utf-8"))
    return h.hexdigest()


def _refusal_category(entry: Dict[str, object]) -> str:
    err_val = entry.get("error_message") or entry.get("error")
    msg = err_val.get("message", "") if isinstance(err_val, dict) else str(err_val or "")
    if "associated item" in msg and "external trait" in msg:
        return "external_trait_item"
    if "external symbol" in msg or "external or standard library symbol" in msg:
        return "external_or_stdlib_symbol"
    if ("binary entrypoint" in msg or "entrypoint" in msg) and "'main'" in msg:
        return "entrypoint_main"
    if "macro_rules!" in msg or "proc-macro" in msg or "macro expansion" in msg or "macro" in msg.lower():
        return "macro_expansion_coupled"
    if "unindexed Cargo target" in msg or "multiple plausible" in msg or "ambiguous unindexed reference" in msg:
        return "unindexed_target_ambiguous"
    if "re-exported" in msg or "re-export" in msg:
        return "cross_crate_reexport"
    if "No renameable symbol" in msg or "Cannot rename keyword" in msg or "no identifier found" in msg:
        return "not_renameable_at_cursor"
    return msg[:80] if msg else "unspecified"


def run_corpus_pkl(
    corpus: str,
    pkl_path: str,
    limit: int = 0,
    repos_root: str = "~/prod-code-storage/bench_repos",
    target_root: str = "~/ridx-bench/review",
    seed: int = 7,
    mitts_bin: Optional[str] = None,
) -> dict:
    mitts_path, checker_version = resolve_mitts_binary(mitts_bin)
    src = os.path.expanduser(f"{repos_root}/{corpus}")
    wt = f"/tmp/rcwt_{corpus}"
    tgt = os.path.expanduser(f"{target_root}/rc_target_{corpus}")
    if not os.path.isdir(wt):
        subprocess.run(["git", "-C", src, "worktree", "add", "-f", "--detach", wt, "HEAD"], check=True, capture_output=True)

    def reset() -> None:
        subprocess.run(["git", "checkout", "--", "."], cwd=wt, capture_output=True, text=True)
        subprocess.run(["git", "clean", "-fdq"], cwd=wt, capture_output=True, text=True)

    mode: List[str] = ["--all-targets"]

    def check() -> Tuple[bool, List[str]]:
        cmd = [mitts_path, "check", "--workspace"] + mode
        r = subprocess.run(
            cmd,
            cwd=wt,
            capture_output=True,
            text=True,
            # A rustc wrapper (sccache from a cargo config) makes Mitts treat the build as uncacheable: every check
            # recompiled everything and left new objects in its store. Clear it so repeated checks hit the cache.
            env=dict(os.environ, CARGO_TARGET_DIR=tgt, CARGO_BUILD_RUSTC_WRAPPER="", RUSTC_WRAPPER=""),
        )
        output = r.stderr + "\n" + r.stdout
        errs = [ln for ln in output.split("\n") if ": error" in ln or ln.startswith("error") or "rustc::error" in ln or "× Failed to compile" in ln]
        return r.returncode == 0, errs[:3]

    with open(pkl_path, "rb") as fh:
        rows = [r for r in pickle.load(fh) if r.get("method") == "rename"]

    reset()
    ok, e = check()
    print("baseline", corpus, mode, ok, e, flush=True)
    if not ok:
        mode = []
        ok, e = check()
        print("baseline", corpus, mode, ok, e, flush=True)
        if not ok:
            raise RuntimeError(f"Baseline check failed for {corpus}: {e}")

    rng = random.Random(seed)
    rng.shuffle(rows)
    stats = {"ridx": [0, 0, 0, 0], "ra": [0, 0, 0, 0]}  # [answered_checked, compiled, fileop, failed]
    total_positions = len(rows)
    total_ridx_answered = 0
    sampled_answered = 0
    r_refused = 0
    ra_refused_total = 0
    refused_where_ra_answered = 0
    refused_where_ra_external = 0
    refused_where_ra_broke_build = 0
    refused_where_ra_compiled = 0
    refused_ra_compiled_cases = []
    refused_ra_broke_cases = []
    refusal_reasons: Dict[str, int] = {}
    fails = []
    cache: Dict[str, Tuple[bool, List[str]]] = {}

    def check_eds(eds_obj: Dict[str, list]) -> Tuple[bool, List[str]]:
        sig = _edit_signature(eds_obj)
        if sig in cache:
            return cache[sig]
        reset()
        if not apply_edits_in_worktree(eds_obj, src, wt):
            res_pair = (False, ["external path outside workspace"])
        else:
            res_pair = check()
        cache[sig] = res_pair
        return res_pair

    for r in rows:
        raw = r["_raw"]
        ridx_key = "ridx1" if "ridx1" in raw else ("after" if "after" in raw else next((k for k in raw if k.startswith("ridx")), "before"))
        r_entry = raw[ridx_key]
        ra_entry = raw["ra"]
        r_out = r_entry["outcome"]
        ra_out = ra_entry["outcome"]
        if ra_out != "a":
            ra_refused_total += 1

        if r_out != "a":
            # Full-corpus refusal audit: NEVER skipped or truncated by `limit`.
            r_refused += 1
            cat = _refusal_category(r_entry)
            refusal_reasons[cat] = refusal_reasons.get(cat, 0) + 1
            if ra_out == "a":
                refused_where_ra_answered += 1
                ra_res = ra_entry.get("res")
                has_edits, ext_paths = workspace_edit_external_paths(ra_res, src)
                if has_edits and ext_paths:
                    refused_where_ra_external += 1
                else:
                    eds = edits_of(ra_res)
                    if eds == "fileop":
                        refused_where_ra_compiled += 1
                        refused_ra_compiled_cases.append((r.get("name"), r.get("symbol"), "fileop"))
                    elif isinstance(eds, dict) and eds:
                        ok_c, err_c = check_eds(eds)
                        if ok_c:
                            refused_where_ra_compiled += 1
                            refused_ra_compiled_cases.append((r.get("name"), r.get("symbol"), r_entry.get("error_message") or r_entry.get("error")))
                        else:
                            refused_where_ra_broke_build += 1
                            refused_ra_broke_cases.append((r.get("name"), r.get("symbol"), err_c))
            continue

        total_ridx_answered += 1
        if limit > 0 and sampled_answered >= limit:
            continue
        sampled_answered += 1
        for who, a in (("ridx", r_entry), ("ra", ra_entry)):
            if a["outcome"] != "a":
                continue
            eds = edits_of(a.get("res"))
            s = stats[who]
            s[0] += 1
            if eds == "fileop":
                s[2] += 1
                continue
            if not eds or not isinstance(eds, dict):
                continue
            ok_c, err_c = check_eds(eds)
            if ok_c:
                s[1] += 1
            else:
                s[3] += 1
                fails.append((who, r.get("name"), r.get("symbol"), err_c))
        print(corpus, sampled_answered, stats, flush=True)

    reset()
    print(
        "FINAL", corpus, json.dumps(stats),
        f"refused_total={r_refused}",
        f"ref_ra_ext={refused_where_ra_external}",
        f"ref_ra_broke={refused_where_ra_broke_build}",
        f"refused_where_ra_compiled={refused_where_ra_compiled}",
        "reasons=", refusal_reasons,
        flush=True,
    )
    if fails:
        ridx_fails = [f for f in fails if f[0] == "ridx"]
        print("RIDX_FAILS", corpus, len(ridx_fails), json.dumps(ridx_fails, indent=0), flush=True)
    r_ans, r_ok, r_fop, r_fail = stats["ridx"]
    ra_ans, ra_ok, ra_fop, ra_fail = stats["ra"]
    r_denom = max(1, r_ans - r_fop)
    ra_denom = max(1, ra_ans - ra_fop)
    return {
        "corpus": corpus,
        "checker": checker_version,
        "total_positions": total_positions,
        "ridx": {
            "total_answered": total_ridx_answered,
            "answered": r_ans,
            "refused": r_refused,
            "refused_where_ra_answered": refused_where_ra_answered,
            "refused_where_ra_external": refused_where_ra_external,
            "refused_where_ra_broke_build": refused_where_ra_broke_build,
            "refused_where_ra_compiled": refused_where_ra_compiled,
            "refused_ra_compiled_cases": refused_ra_compiled_cases,
            "refused_ra_broke_cases": refused_ra_broke_cases,
            "refusal_reasons": refusal_reasons,
            "compiled": r_ok,
            "fileop": r_fop,
            "failed": r_fail,
            "compile_pct": round(100.0 * r_ok / r_denom, 1),
        },
        "ra": {
            "answered": ra_ans,
            "refused_total": ra_refused_total,
            "compiled": ra_ok,
            "fileop": ra_fop,
            "failed": ra_fail,
            "compile_pct": round(100.0 * ra_ok / ra_denom, 1),
        },
        "failures": fails,
    }


def main() -> int:
    if len(sys.argv) >= 3 and not sys.argv[1].startswith("-"):
        corpus = sys.argv[1]
        pkl_path = sys.argv[2]
        limit = int(sys.argv[3]) if len(sys.argv) >= 4 else 0
        res = run_corpus_pkl(corpus, pkl_path, limit=limit)
        return 0 if res["ridx"]["failed"] == 0 else 1

    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--all-corpora", action="store_true", help="Run across all 7 benchmark corpora")
    ap.add_argument("--corpus", default=None, help="Single corpus name")
    ap.add_argument("--pkl", default=None, help="Path to single .raw.pkl file")
    ap.add_argument("--pkl-dir", default="~/ridx-bench/out_b1_clean", help="Directory containing <corpus>.json.raw.pkl")
    ap.add_argument("--repos-root", default="~/prod-code-storage/bench_repos", help="Directory containing benchmark repos")
    ap.add_argument("--target-root", default="~/ridx-bench/review", help="Directory containing rc_target_<corpus>")
    ap.add_argument("--limit", type=int, default=0, help="Max answered positions to compile-check per corpus (0 = all; refusals are ALWAYS checked in full)")
    ap.add_argument("--mitts", default=None, help="Path to mitts binary (defaults to MITTS env or PATH)")
    ap.add_argument("--out", default=None, help="Optional JSON output path")
    args = ap.parse_args()

    _, checker_version = resolve_mitts_binary(args.mitts)

    corpora = list(CORPORA) if args.all_corpora else ([args.corpus] if args.corpus else [])
    if not corpora:
        ap.error("Specify positional CORPUS PKL [N], --all-corpora, or --corpus CORPUS --pkl PKL")

    pkl_dir = Path(os.path.expanduser(args.pkl_dir))
    results = []
    any_ridx_fail = False
    for c in corpora:
        pkl_p = Path(args.pkl) if args.pkl else (pkl_dir / f"{c}.json.raw.pkl")
        res = run_corpus_pkl(c, str(pkl_p), limit=args.limit, repos_root=args.repos_root, target_root=args.target_root, mitts_bin=args.mitts)
        results.append(res)
        if res["ridx"]["failed"] > 0:
            any_ridx_fail = True

    print("\n=== Side-by-Side Rename Compile & Full-Corpus Refusal Audit ===")
    print(f"{'Corpus':<15} {'tot':>3} {'ref':>3} {'ra_ext':>6} {'ra_brk':>6} {'ra_ok':>5} {'ridx compiled (%)':<22} {'ra compiled (%)':<22}")
    print("-" * 92)
    t_pos = t_ref = t_ext = t_brk = t_rok = t_r_ok = t_r_chk = t_ra_ok = t_ra_chk = 0
    tot_reasons: Dict[str, int] = {}
    for r in results:
        rc, rac = r["ridx"], r["ra"]
        r_chk, ra_chk = rc["answered"] - rc["fileop"], rac["answered"] - rac["fileop"]
        t_pos += r["total_positions"]
        t_ref += rc["refused"]
        t_ext += rc["refused_where_ra_external"]
        t_brk += rc["refused_where_ra_broke_build"]
        t_rok += rc["refused_where_ra_compiled"]
        t_r_ok += rc["compiled"]
        t_r_chk += r_chk
        t_ra_ok += rac["compiled"]
        t_ra_chk += ra_chk
        for k, v in (rc.get("refusal_reasons") or {}).items():
            tot_reasons[k] = tot_reasons.get(k, 0) + v
        r_s = f"{rc['compiled']}/{r_chk} ({rc['compile_pct']}%)"
        ra_s = f"{rac['compiled']}/{ra_chk} ({rac['compile_pct']}%)"
        print(f"{r['corpus']:<15} {r['total_positions']:>3} {rc['refused']:>3} {rc['refused_where_ra_external']:>6} {rc['refused_where_ra_broke_build']:>6} {rc['refused_where_ra_compiled']:>5} {r_s:<22} {ra_s:<22}")
    print("-" * 92)
    tr_pct = round(100.0 * t_r_ok / max(1, t_r_chk), 1)
    tra_pct = round(100.0 * t_ra_ok / max(1, t_ra_chk), 1)
    print(f"{'TOTAL':<15} {t_pos:>3} {t_ref:>3} {t_ext:>6} {t_brk:>6} {t_rok:>5} {f'{t_r_ok}/{t_r_chk} ({tr_pct}%)':<22} {f'{t_ra_ok}/{t_ra_chk} ({tra_pct}%)':<22}")
    print("Refusal reason distribution:", json.dumps(tot_reasons))
    if args.out:
        Path(args.out).write_text(json.dumps({"checker": checker_version, "results": results, "total_refusal_reasons": tot_reasons}, indent=2) + "\n", encoding="utf-8")
    return 1 if any_ridx_fail else 0


if __name__ == "__main__":
    sys.exit(main())
