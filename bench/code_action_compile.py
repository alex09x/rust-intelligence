# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Side-by-side `codeAction` / `codeAction/resolve` apply+compile checker across all 7 corpora.

Reads resolved `CodeAction` items (`code_action_resolve` rows, or eager `code_action` edits) from
`.raw.pkl` produced by `position_ab_parity.py --save-raw-pkl`, applies each action's `WorkspaceEdit`
into an isolated detached Git worktree, runs `mitts check --workspace [--all-targets]`, and reports
side-by-side compile rates, build-breaking actions by title, and `coverage_of_ra_compiling` for
`ridx` vs `rust-analyzer`.

Usage:
  1. Single corpus:
       python3 bench/code_action_compile.py CORPUS PKL [--limit N]
  2. All 7 corpora:
       python3 bench/code_action_compile.py --all-corpora --pkl-dir DIR [--jobs 7] [--out JSON]
"""

import argparse
import concurrent.futures
import hashlib
import json
import os
import pickle
import random
import shutil
import subprocess
import sys
import urllib.parse
from collections import Counter
from pathlib import Path
from typing import Dict, Iterable, List, Optional, Set, Tuple

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


def _utf16_col_to_char_idx(line_str: str, u16_target: int) -> int:
    u16_cur = 0
    c_idx = 0
    for ch in line_str:
        if u16_cur >= u16_target:
            break
        u16_cur += 2 if ord(ch) > 0xFFFF else 1
        c_idx += 1
    return c_idx


def extract_workspace_edits(edit: Optional[dict]) -> Optional[Tuple[List[str], Dict[str, list]]]:
    if not isinstance(edit, dict):
        return None
    creates: List[str] = []
    out: Dict[str, list] = {}
    for dc in edit.get("documentChanges") or []:
        if not isinstance(dc, dict):
            continue
        kind = dc.get("kind")
        if kind == "create" and dc.get("uri"):
            creates.append(str(dc["uri"]))
        elif kind in ("rename", "delete"):
            return None
        elif "textDocument" in dc:
            uri = str((dc.get("textDocument") or {}).get("uri") or "")
            if uri and dc.get("edits"):
                out.setdefault(uri, []).extend(dc.get("edits") or [])
    for uri, es in (edit.get("changes") or {}).items():
        if es and uri not in out:
            out.setdefault(uri, []).extend(es)
    if not out and not creates:
        return None
    return creates, out


def _map_uri_to_wt(uri: str, src_dir: str, wt_dir: str) -> Optional[str]:
    if not uri.startswith("file://"):
        return None
    src_real = os.path.realpath(src_dir).rstrip("/")
    wt_real = os.path.realpath(wt_dir).rstrip("/")
    raw_p = urllib.parse.unquote(uri[7:])
    real_p = os.path.realpath(raw_p)
    if real_p.startswith(src_real + "/"):
        return wt_real + real_p[len(src_real):]
    if raw_p.startswith(src_dir.rstrip("/") + "/"):
        return wt_real + raw_p[len(src_dir.rstrip("/")):]
    if real_p.startswith(wt_real + "/"):
        return real_p
    return None


def apply_action_in_worktree(creates: List[str], eds: Dict[str, list], src_dir: str, wt_dir: str) -> bool:
    for c_uri in creates:
        cp = _map_uri_to_wt(c_uri, src_dir, wt_dir)
        if not cp:
            return False
        os.makedirs(os.path.dirname(cp), exist_ok=True)
        if not os.path.exists(cp):
            with open(cp, "w", encoding="utf-8") as fh:
                fh.write("")
    for uri, es in eds.items():
        target_p = _map_uri_to_wt(uri, src_dir, wt_dir)
        if not target_p or not os.path.isfile(target_p):
            return False
        with open(target_p, encoding="utf-8") as fh:
            txt = fh.read()
        lines = txt.split("\n")
        starts = [0]
        for ln in lines:
            starts.append(starts[-1] + len(ln) + 1)

        def off(pos: dict) -> int:
            if not lines:
                return 0
            l_idx = int(pos.get("line", 0))
            if l_idx >= len(lines):
                return len(txt)
            l_idx = max(0, l_idx)
            c_idx = _utf16_col_to_char_idx(lines[l_idx], int(pos.get("character", 0)))
            return starts[l_idx] + c_idx

        indexed = []
        for seq, e in enumerate(es):
            a, b = off(e["range"]["start"]), off(e["range"]["end"])
            indexed.append((a, b, seq, str(e.get("newText", ""))))
        for a, b, _, new_text in sorted(indexed, key=lambda x: (x[0], x[1], x[2]), reverse=True):
            txt = txt[:a] + new_text + txt[b:]
        with open(target_p, "w", encoding="utf-8") as fh:
            fh.write(txt)
    return True


def _edit_sig(creates: List[str], eds: Dict[str, list]) -> str:
    h = hashlib.sha256()
    for c in sorted(creates):
        h.update(f"create:{c}".encode("utf-8"))
    for uri in sorted(eds.keys()):
        h.update(uri.encode("utf-8"))
        for e in eds[uri]:
            h.update(json.dumps(e, sort_keys=True).encode("utf-8"))
    return h.hexdigest()


def compute_ra_compiling_coverage(per_pos_ok_titles: Iterable[Tuple[Set[str], Set[str]]]) -> dict:
    ra_actions = 0
    covered_actions = 0
    ra_positions = 0
    any_covered_positions = 0
    full_covered_positions = 0
    uncovered_by_title: Counter = Counter()
    for ridx_ok, ra_ok in per_pos_ok_titles:
        if not ra_ok:
            continue
        ra_positions += 1
        shared = ridx_ok & ra_ok
        ra_actions += len(ra_ok)
        covered_actions += len(shared)
        if shared:
            any_covered_positions += 1
        if ra_ok <= ridx_ok:
            full_covered_positions += 1
        for t in ra_ok - ridx_ok:
            uncovered_by_title[t] += 1
    return {
        "ra_compiling_actions": ra_actions,
        "ridx_covering_ra_compiling": covered_actions,
        "coverage_of_ra_compiling_pct": round(100.0 * covered_actions / ra_actions, 1) if ra_actions else 100.0,
        "ra_compiling_positions": ra_positions,
        "ridx_any_covering_positions": any_covered_positions,
        "ridx_full_covering_positions": full_covered_positions,
        "position_any_coverage_pct": round(100.0 * any_covered_positions / ra_positions, 1) if ra_positions else 100.0,
        "position_full_coverage_pct": round(100.0 * full_covered_positions / ra_positions, 1) if ra_positions else 100.0,
        "uncovered_ra_compiling_by_title": dict(uncovered_by_title.most_common(15)),
    }


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
    wt = f"/tmp/cacwt_{corpus}"
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
            env=dict(os.environ, CARGO_TARGET_DIR=tgt),
        )
        output = r.stderr + "\n" + r.stdout
        errs = [ln for ln in output.split("\n") if ": error" in ln or ln.startswith("error") or "rustc::error" in ln or "× Failed to compile" in ln]
        return r.returncode == 0, errs[:3]

    with open(pkl_path, "rb") as fh:
        all_rows = pickle.load(fh)
    rows = [r for r in all_rows if r.get("method") == "code_action_resolve"]
    if not rows:
        rows = [r for r in all_rows if r.get("method") == "code_action"]

    reset()
    ok, e = check()
    if not ok:
        mode = []
        ok, e = check()
        if not ok:
            raise RuntimeError(f"Baseline check failed for {corpus}: {e}")

    rng = random.Random(seed)
    rng.shuffle(rows)
    if limit > 0:
        rows = rows[:limit]

    # Results persist across runs: rust-analyzer's edits never change between RIDX iterations, and an unchanged RIDX edit
    # needs no second build. The key pins the corpus commit and check mode, so a different checkout never reuses a verdict.
    head = subprocess.run(["git", "rev-parse", "HEAD"], cwd=wt, capture_output=True, text=True).stdout.strip()
    key_prefix = f"{head}|{' '.join(mode)}|"
    cache_path = os.path.expanduser(f"{target_root}/cac_cache_{corpus}.json")
    try:
        with open(cache_path, encoding="utf-8") as fh:
            disk_cache = {k[len(key_prefix):]: (bool(v[0]), list(v[1])) for k, v in json.load(fh).items() if k.startswith(key_prefix)}
    except (OSError, ValueError):
        disk_cache = {}
    cache: Dict[str, Tuple[bool, List[str]]] = dict(disk_cache)
    summary = {
        who: {"positions_with_actions": 0, "actions_checked": 0, "actions_compiled": 0, "actions_broke_build": 0, "broke_by_title": Counter(), "sample_failures": []}
        for who in ("ridx", "ra")
    }
    per_pos_ok: List[Tuple[Set[str], Set[str]]] = []

    for r in rows:
        raw = r.get("_raw") or {}
        ridx_key = "ridx1" if "ridx1" in raw else ("ridx" if "ridx" in raw else next((k for k in raw if k != "ra"), ""))
        pos_ok: Dict[str, Set[str]] = {"ridx": set(), "ra": set()}
        for who, key in (("ridx", ridx_key), ("ra", "ra")):
            entry = raw.get(key) or {}
            res = entry.get("res")
            if entry.get("outcome") != "a" or not isinstance(res, list) or not res:
                continue
            s = summary[who]
            pos_counted = False
            for act in res:
                if not isinstance(act, dict):
                    continue
                parsed = extract_workspace_edits(act.get("edit"))
                if parsed is None:
                    continue
                if not pos_counted:
                    s["positions_with_actions"] += 1
                    pos_counted = True
                creates, eds = parsed
                s["actions_checked"] += 1
                sig = _edit_sig(creates, eds)
                if sig not in cache:
                    reset()
                    if not apply_action_in_worktree(creates, eds, src, wt):
                        cache[sig] = (False, ["external or missing path outside workspace"])
                    else:
                        cache[sig] = check()
                ok_c, err_c = cache[sig]
                title = str(act.get("title") or "<untitled>")
                if ok_c:
                    s["actions_compiled"] += 1
                    pos_ok[who].add(title)
                else:
                    s["actions_broke_build"] += 1
                    s["broke_by_title"][title] += 1
                    if len(s["sample_failures"]) < 10:
                        s["sample_failures"].append({"pos": r.get("name"), "file": r.get("file"), "line": r.get("line"), "title": title, "errors": err_c})
        per_pos_ok.append((pos_ok["ridx"], pos_ok["ra"]))

    reset()
    try:
        with open(cache_path, encoding="utf-8") as fh:
            stored = json.load(fh)
    except (OSError, ValueError):
        stored = {}
    stored.update({key_prefix + sig: [ok_v, err_v] for sig, (ok_v, err_v) in cache.items()})
    with open(cache_path, "w", encoding="utf-8") as fh:
        json.dump(stored, fh)
    cov = compute_ra_compiling_coverage(per_pos_ok)
    out = {
        "corpus": corpus,
        "checker": checker_version,
        "positions_evaluated": len(rows),
        "coverage_of_ra_compiling": cov,
    }
    for who in ("ridx", "ra"):
        s = summary[who]
        denom = max(1, s["actions_checked"])
        out[who] = {
            "positions_with_actions": s["positions_with_actions"],
            "actions_checked": s["actions_checked"],
            "actions_compiled": s["actions_compiled"],
            "actions_broke_build": s["actions_broke_build"],
            "compile_pct": round(100.0 * s["actions_compiled"] / denom, 1) if s["actions_checked"] else 100.0,
            "broke_by_title": dict(s["broke_by_title"].most_common()),
            "sample_failures": s["sample_failures"],
        }
    print(
        f"[{corpus}] ridx={out['ridx']['actions_compiled']}/{out['ridx']['actions_checked']} ({out['ridx']['compile_pct']}%) "
        f"ra={out['ra']['actions_compiled']}/{out['ra']['actions_checked']} ({out['ra']['compile_pct']}%) "
        f"cov_ra_ok={cov['ridx_covering_ra_compiling']}/{cov['ra_compiling_actions']} ({cov['coverage_of_ra_compiling_pct']}%)",
        flush=True,
    )
    return out


def _run_job(job: Tuple[str, str, int, Optional[str]]) -> Tuple[str, dict]:
    corpus, pkl_path, limit, mitts_bin = job
    return corpus, run_corpus_pkl(corpus, pkl_path, limit=limit, mitts_bin=mitts_bin)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("corpus", nargs="?")
    ap.add_argument("pkl", nargs="?")
    ap.add_argument("--all-corpora", action="store_true")
    ap.add_argument("--pkl-dir", type=Path)
    ap.add_argument("--limit", type=int, default=0, help="Max positions per corpus (0 = all 300)")
    ap.add_argument("--jobs", type=int, default=1, help="Parallel corpus workers for --all-corpora")
    ap.add_argument("--mitts", default=None, help="Path to mitts binary (defaults to MITTS env or PATH)")
    ap.add_argument("--out", type=Path)
    args = ap.parse_args()

    _, checker_version = resolve_mitts_binary(args.mitts)

    if args.all_corpora:
        if not args.pkl_dir:
            ap.error("--pkl-dir is required with --all-corpora")
        jobs = []
        for c in CORPORA:
            cands = sorted(args.pkl_dir.glob(f"*{c}*.raw.pkl"))
            if not cands:
                raise FileNotFoundError(f"No .raw.pkl found for {c} in {args.pkl_dir}")
            jobs.append((c, str(cands[0]), args.limit, args.mitts))
        per_corpus: Dict[str, dict] = {}
        if args.jobs > 1:
            with concurrent.futures.ProcessPoolExecutor(max_workers=min(args.jobs, len(jobs))) as ex:
                for c, res in ex.map(_run_job, jobs):
                    per_corpus[c] = res
        else:
            for c, pkl_s, lim, mbin in jobs:
                per_corpus[c] = run_corpus_pkl(c, pkl_s, limit=lim, mitts_bin=mbin)
        tot = {w: [0, 0, 0] for w in ("ridx", "ra")}
        cov_tot = [0, 0, 0, 0, 0]
        uncov_tot: Counter = Counter()
        ordered = {}
        for c in CORPORA:
            res = per_corpus[c]
            ordered[c] = res
            for w in ("ridx", "ra"):
                tot[w][0] += res[w]["actions_checked"]
                tot[w][1] += res[w]["actions_compiled"]
                tot[w][2] += res[w]["actions_broke_build"]
            cv = res["coverage_of_ra_compiling"]
            cov_tot[0] += cv["ra_compiling_actions"]
            cov_tot[1] += cv["ridx_covering_ra_compiling"]
            cov_tot[2] += cv["ra_compiling_positions"]
            cov_tot[3] += cv["ridx_any_covering_positions"]
            cov_tot[4] += cv["ridx_full_covering_positions"]
            for t, cnt in cv["uncovered_ra_compiling_by_title"].items():
                uncov_tot[t] += cnt
        agg = {
            "checker": checker_version,
            "corpora": ordered,
            "totals": {
                **{
                    w: {
                        "actions_checked": tot[w][0],
                        "actions_compiled": tot[w][1],
                        "actions_broke_build": tot[w][2],
                        "compile_pct": round(100.0 * tot[w][1] / max(1, tot[w][0]), 1),
                    }
                    for w in ("ridx", "ra")
                },
                "coverage_of_ra_compiling": {
                    "ra_compiling_actions": cov_tot[0],
                    "ridx_covering_ra_compiling": cov_tot[1],
                    "coverage_of_ra_compiling_pct": round(100.0 * cov_tot[1] / max(1, cov_tot[0]), 1) if cov_tot[0] else 100.0,
                    "ra_compiling_positions": cov_tot[2],
                    "ridx_any_covering_positions": cov_tot[3],
                    "ridx_full_covering_positions": cov_tot[4],
                    "position_any_coverage_pct": round(100.0 * cov_tot[3] / max(1, cov_tot[2]), 1) if cov_tot[2] else 100.0,
                    "position_full_coverage_pct": round(100.0 * cov_tot[4] / max(1, cov_tot[2]), 1) if cov_tot[2] else 100.0,
                    "uncovered_ra_compiling_by_title": dict(uncov_tot.most_common(20)),
                },
            },
        }
        if args.out:
            args.out.write_text(json.dumps(agg, indent=2) + "\n", encoding="utf-8")
        print("TOTALS:", json.dumps(agg["totals"]))
        return 0

    if not args.corpus or not args.pkl:
        ap.error("Specify CORPUS PKL or --all-corpora --pkl-dir DIR")
    res = run_corpus_pkl(args.corpus, args.pkl, limit=args.limit, mitts_bin=args.mitts)
    if args.out:
        args.out.write_text(json.dumps(res, indent=2) + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    sys.exit(main())
