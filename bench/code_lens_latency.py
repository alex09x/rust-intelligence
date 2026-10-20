#!/usr/bin/env python3
#
# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
#

"""Measures textDocument/codeLens latency of ridx binaries on the matrix files of one corpus.

    code_lens_latency.py --repo DIR --positions matrix_<repo>_booster.positions.json \
        --ridx base=BIN [--ridx fix=BIN ...] [--repeat 2] --out FILE

The files are the `code_lens` items of the published matrix sample. For every file the client
asks twice (cold, then warm) and records both latencies, the number of lenses, the file size and
line count, and a digest of the answer serialised with sorted keys, so two binaries can be
compared answer by answer. With several binaries and --repeat K the binaries alternate
(A, B, A, B, ...) so that drift of the node affects both sides alike. The output holds every row,
per-run summaries (percentiles, the slowest files) and the answer comparison between binaries.
"""

import argparse
import hashlib
import json
from collections import Counter
import socket
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from lsp_process import SimpleLspProcess  # noqa: E402
from matrix_provenance import check_ridx_index, normalize_repo_identifier  # noqa: E402
from metrics import percentile  # noqa: E402

TOP = 10


def load_files(positions_path):
    data = json.loads(Path(positions_path).read_text(encoding="utf-8"))
    items = data["positions"]["code_lens"]
    return [{"name": it["name"], "file": it["file"]} for it in items]


def file_facts(repo_dir, rel):
    try:
        text = (Path(repo_dir) / rel).read_bytes()
    except OSError:
        return {"bytes": None, "lines": None, "text": ""}
    decoded = text.decode("utf-8", errors="replace")
    return {"bytes": len(text), "lines": len(decoded.splitlines()), "text": decoded}


def lens_key(lens):
    r = lens.get("range", {})
    s, e = r.get("start", {}), r.get("end", {})
    return (s.get("line"), s.get("character"), e.get("line"), e.get("character"), lens.get("data", {}).get("name"))


def lens_checks(result, facts):
    """Lenses that cannot belong to the file: a zero-based start line at or past the number of
    lines in the file, or a name absent from its text."""
    if not isinstance(result, list):
        return {"lenses_past_eof": None, "lens_names_missing": None}
    lines = facts["lines"] or 0
    past = sum(1 for lens in result if (lens.get("range", {}).get("start", {}).get("line", 0) or 0) >= lines)
    names = {lens.get("data", {}).get("name") for lens in result} - {None}
    missing = sum(1 for n in names if n not in facts["text"])
    return {"lenses_past_eof": past, "lens_names_missing": missing}


def digest(result):
    return hashlib.sha256(json.dumps(result, sort_keys=True, separators=(",", ":")).encode("utf-8")).hexdigest()


def version_of(binary):
    try:
        out = subprocess.run([str(binary), "--version"], capture_output=True, text=True, timeout=30)
        return (out.stdout or out.stderr).strip()
    except Exception as exc:  # pragma: no cover - provenance only
        return f"unavailable: {exc}"


def query_files(srv, files, repo_dir, facts, keep=None):
    """Asks every file twice; `keep`, when given, receives the warm answer of each file as a lens multiset."""
    rows = []
    for item in files:
        params = {"textDocument": {"uri": f"file://{Path(repo_dir) / item['file']}"}}
        res_cold, cold_us, err_cold = srv.request("textDocument/codeLens", params, timeout=30.0)
        res_warm, warm_us, err_warm = srv.request("textDocument/codeLens", params, timeout=30.0)
        res = res_warm if err_warm is None else res_cold
        if keep is not None and isinstance(res, list):
            keep[item["file"]] = Counter(lens_key(lens) for lens in res)
        rows.append({
            "name": item["name"],
            "file": item["file"],
            "bytes": facts[item["file"]]["bytes"],
            "lines": facts[item["file"]]["lines"],
            "lenses": len(res) if isinstance(res, list) else None,
            **lens_checks(res, facts[item["file"]]),
            "digest": digest(res) if err_warm is None else None,
            "error": None if (err_cold is None and err_warm is None) else str(err_warm or err_cold),
            "cold_us": round(cold_us, 1),
            "warm_us": round(warm_us, 1),
        })
    return rows


def run_binary(binary, repo_dir, files, facts, keep=None):
    srv = SimpleLspProcess([str(binary), "lsp"], repo_dir, is_ra=False)
    t0 = time.perf_counter()
    try:
        rows = query_files(srv, files, repo_dir, facts, keep)
    finally:
        srv.close()
    return rows, round((time.perf_counter() - t0) * 1000.0, 1)


def summarize(rows):
    ok = [r for r in rows if r["error"] is None]
    cold = [r["cold_us"] for r in ok]
    warm = [r["warm_us"] for r in ok]
    slowest = sorted(ok, key=lambda r: r["warm_us"], reverse=True)[:TOP]
    return {
        "files": len(rows),
        "errors": len(rows) - len(ok),
        "lenses_total": sum(r["lenses"] or 0 for r in ok),
        **{f"cold_p{p}_us": round(percentile(cold, p), 1) for p in (50, 90, 95, 99)},
        **{f"warm_p{p}_us": round(percentile(warm, p), 1) for p in (50, 90, 95, 99)},
        "slowest_warm": [
            {k: r[k] for k in ("name", "file", "lenses", "lenses_past_eof", "lens_names_missing", "bytes", "lines",
                               "cold_us", "warm_us")} for r in slowest
        ],
    }


def compare_answers(runs, answers):
    """Per pair of binaries: files whose warm answers differ (by digest) across any round, and for
    each of them whether the second binary's answer (first round) is a sub-multiset of the first's."""
    by_binary, past_eof = {}, {}
    for run in runs:
        for row in run["rows"]:
            by_binary.setdefault(run["name"], {}).setdefault(row["file"], set()).add(row["digest"])
            if run["round"] == 1:
                past_eof.setdefault(run["name"], {})[row["file"]] = row["lenses_past_eof"]
    names = list(by_binary)
    out = {"binaries": names, "pairs": []}
    for i, a in enumerate(names):
        for b in names[i + 1:]:
            files = sorted(set(by_binary[a]) | set(by_binary[b]))
            differing = [f for f in files if by_binary[a].get(f) != by_binary[b].get(f)]
            details = []
            for f in differing:
                ca, cb = answers.get(a, {}).get(f, Counter()), answers.get(b, {}).get(f, Counter())
                details.append({
                    "file": f, "lenses_a": sum(ca.values()), "lenses_b": sum(cb.values()),
                    "distinct_a": len(ca), "distinct_b": len(cb),
                    "removed": sum((ca - cb).values()), "added": sum((cb - ca).values()),
                    "b_subset_of_a": not (cb - ca), "b_is_set_of_a": cb == Counter(set(ca)),
                    "b_lenses_past_eof": past_eof.get(b, {}).get(f),
                })
            out["pairs"].append({
                "a": a, "b": b, "files": len(files), "identical": len(files) - len(differing),
                "differing": len(differing),
                "subset_violations": sum(1 for d in details if not d["b_subset_of_a"]),
                "set_of_a_violations": sum(1 for d in details if not d["b_is_set_of_a"]),
                "b_past_eof_remaining": sum(1 for d in details if d["b_lenses_past_eof"]),
                "differing_details": details,
            })
    unstable = {name: sorted(f for f, d in files.items() if len(d) > 1) for name, files in by_binary.items()}
    out["unstable_within_binary"] = {k: v for k, v in unstable.items() if v}
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--repo", required=True, type=Path)
    ap.add_argument("--positions", required=True, type=Path, help="published matrix positions file of the corpus")
    ap.add_argument("--ridx", action="append", required=True, metavar="NAME=BIN", help="ridx binary to measure; repeatable")
    ap.add_argument("--repeat", type=int, default=1, help="rounds; binaries alternate within every round")
    ap.add_argument("--limit", type=int, default=0, help="measure only the first N files (smoke runs)")
    ap.add_argument("--out", required=True, type=Path)
    args = ap.parse_args()

    repo_dir = args.repo.resolve()
    shards = check_ridx_index(repo_dir)
    ridx_bins = [(name, Path(binary)) for name, binary in (spec.split("=", 1) for spec in args.ridx)]
    files = load_files(args.positions)
    if args.limit:
        files = files[: args.limit]
    facts = {item["file"]: file_facts(repo_dir, item["file"]) for item in files}
    started = datetime.now(timezone.utc).isoformat()
    print(f"{repo_dir.name}: {len(files)} files, {shards} index shards", flush=True)

    runs, answers = [], {}
    for round_no in range(1, args.repeat + 1):
        for name, binary in ridx_bins:
            print(f"round {round_no}: ridx {name} ({binary}) ...", flush=True)
            keep = answers.setdefault(name, {}) if round_no == 1 else None
            rows, duration_ms = run_binary(binary, repo_dir, files, facts, keep)
            summary = summarize(rows)
            runs.append({"name": name, "round": round_no, "binary": "/".join(binary.parts[-3:]),
                         "version": version_of(binary), "duration_ms": duration_ms, "summary": summary, "rows": rows})
            print("  " + json.dumps({k: v for k, v in summary.items() if k != "slowest_warm"}), flush=True)

    report = {
        "repo": normalize_repo_identifier(repo_dir),
        "repo_dir_name": repo_dir.name,
        "repo_head": subprocess.run(["git", "-C", str(repo_dir), "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip(),
        "node": socket.gethostname(),
        "python": sys.version.split()[0],
        "positions_file": args.positions.name,
        "positions_sha256": hashlib.sha256(args.positions.read_bytes()).hexdigest(),
        "count": len(files),
        "index_shards": shards,
        "timestamp_start": started,
        "timestamp_end": datetime.now(timezone.utc).isoformat(),
        "runs": runs,
        "answers": compare_answers(runs, answers),
        "metric_definitions": {
            "cold_us": "first textDocument/codeLens round trip for the file, measured by the client",
            "warm_us": "second round trip for the same file, immediately after the first",
            "lenses": "number of items in the warm answer",
            "lenses_past_eof": "lenses whose zero-based start line is at or past the number of lines in the file",
            "lens_names_missing": "distinct lens names that do not occur in the file text",
            "digest": "sha256 of the warm answer serialised with sorted keys and no whitespace",
            "answers.pairs": "per pair of binaries, files with a different warm digest in any round; for each, "
                             "the lens multisets of the first round compared (removed, added, b_subset_of_a; "
                             "b_is_set_of_a: the second answer holds every distinct lens of the first exactly once)",
            "slowest_warm": f"the {TOP} files with the highest warm_us in the run",
        },
    }
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(report, indent=1), encoding="utf-8")
    print(f"wrote {args.out}")


if __name__ == "__main__":
    main()
