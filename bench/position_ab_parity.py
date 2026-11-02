# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Normalized content-parity benchmark and gap classifier between RIDX and rust-analyzer.

    position_ab_parity.py --repo DIR --positions bench/published/matrix_<corpus>_booster.positions.json \\
        --ridx NAME=BIN --ra NAME=BIN [--methods definition,references,hover,completion] \\
        [--limit N] [--timeout 2.0] --out FILE [--out-differing FILE]

For each method and position, asks both servers, normalizes their responses into a canonical
method-specific representation (`position_ab_parity_norm.py`), and classifies each row into
`exact_match`, `core_match`, `partial`, `ridx_only`, `both_empty`, or `mismatch` with a deterministic
gap class (`position_ab_parity_rules.py`). Exits with code 1 if any row is `unclassified`.
"""

import argparse
import hashlib
import json
import pickle
import socket
import subprocess
import sys
import time
from collections import Counter
from datetime import datetime, timezone
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from code_lens_latency import version_of  # noqa: E402
from code_lens_open_vs_closed import parse_version  # noqa: E402
from lsp_barrier import barrier  # noqa: E402
from lsp_process import SimpleLspProcess  # noqa: E402
from matrix_canary import ensure_ra_ready  # noqa: E402
from matrix_categories import LSP_SPECS, build_params  # noqa: E402
from matrix_provenance import check_ridx_index, normalize_repo_identifier  # noqa: E402
from position_ab import binary_label, build_rows, parse_servers  # noqa: E402
from position_ab_classify import (  # noqa: E402
    REQUEST_KEYS, classify_outcome, load_target_entries, scenario_from_record,
)
from position_ab_content import load_file_table  # noqa: E402
from position_ab_parity_exec import execute_parity_request, prepare_parity_rows  # noqa: E402
from position_ab_parity_norm import normalize_answer  # noqa: E402
from position_ab_parity_rules import evaluate_row  # noqa: E402

RPC_OF = {key: rpc for _, key, rpc, _, _ in LSP_SPECS}
DEFAULT_METHODS = ("definition", "references", "hover", "completion")
ALL_PARITY_METHODS = (
    "definition", "declaration", "type_definition", "implementation", "references", "document_highlight",
    "hover", "signature_help", "inlay_hint", "document_symbol", "workspace_symbol", "folding_range",
    "selection_range", "document_link", "semantic_tokens_full", "semantic_tokens_delta", "semantic_tokens_range",
    "prepare_call_hierarchy", "incoming_calls", "outgoing_calls", "prepare_type_hierarchy", "supertypes", "subtypes",
    "completion", "completion_resolve", "prepare_rename", "rename", "code_action", "formatting",
    "range_formatting", "on_type_formatting", "document_diagnostic", "workspace_diagnostic",
    "code_lens", "code_lens_resolve", "linked_editing_range", "moniker",
)


def _pct(vals, q):
    if not vals:
        return 0.0
    s = sorted(vals)
    idx = min(len(s) - 1, max(0, int(round((len(s) - 1) * q))))
    return round(s[idx], 1)


def ask_server(name, cmd, is_ra, rows, repo_dir, timeout, canary, read_lines_fn=None):
    """Queries one server process across all `rows` and stores raw responses in `row['_raw'][name]`."""
    srv = SimpleLspProcess(cmd, str(repo_dir), is_ra=is_ra)
    t0 = time.perf_counter()
    ready = {"initial_wait_ms": round(srv.initial_wait_ms, 1), "initial_quiescent": srv.initial_quiescent}
    try:
        if is_ra:
            ready.update(ensure_ra_ready(srv, canary, Path(repo_dir), timeout=600.0, interval=1.0))
        current = None
        for row in rows:
            key = row["method"]
            if key != current:
                current = key
                srv.drain(timeout=600.0)
                barrier(srv, timeout=600.0)
            outcome, res, code, us, extra = execute_parity_request(srv, key, row, repo_dir, is_ra, timeout, read_lines_fn or (lambda _: []))
            row.setdefault("_raw", {})[name] = {
                "outcome": outcome, "res": res, "error": code, "us": round(us, 1), **extra,
            }
    finally:
        srv.close()
    ready["cancelled_on_timeout"] = srv.cancelled_on_timeout
    return ready, round(time.perf_counter() - t0, 1)


def evaluate_all_rows(rows, ridx_name, ra_name, repo_dir, read_lines_fn, ws_table=None, ridx2_name=None):
    """Evaluates normalized content parity for all rows and builds per-method and overall summaries."""
    per_method = {}
    classes_overall = Counter()
    parity_overall = Counter()
    for row in rows:
        raw = row.pop("_raw", {})
        row.pop("scenario", None)
        r_ans, a_ans = raw[ridx_name], raw[ra_name]
        parity, cls, ev = evaluate_row(row, r_ans, a_ans, str(repo_dir), read_lines_fn, ws_table)
        row["results"] = {
            k: {
                "outcome": v["outcome"], "us": v["us"], "error": v["error"],
                **({"error_message": v["error_message"]} if v.get("error_message") else {}),
            }
            for k, v in raw.items()
        }
        row["parity"] = parity
        row["class"] = cls
        row["evidence"] = ev
        parity_overall[parity] += 1
        if cls:
            classes_overall[cls] += 1
        m = per_method.setdefault(row["method"], {
            "n": 0, "ra_non_empty": 0, "matched_on_ra_non_empty": 0,
            "exact_match": 0, "core_match": 0, "partial": 0, "ridx_only": 0,
            "both_empty": 0, "mismatch": 0, "classes": Counter(),
            "ra_recalls": [], "ridx_outsides": [],
            "target_in_ra": 0, "target_in_ridx": 0, "target_in_top20": 0, "target_eval": 0,
            "ref_precisions": [], "ref_recalls": [],
            "ref_ridx_ws": 0, "ref_ra_ws": 0, "ref_common_ws": 0,
            "sem_span_recalls": [], "sem_raw_span_recalls": [], "sem_type_agreements": [],
            "sem_ra_sym": 0, "sem_ra_lex": 0,
            "ridx_us": [], "ra_us": [], "self_diff_raw": 0, "self_diff_norm": 0,
        })
        m["n"] += 1
        m["ridx_us"].append(float(r_ans.get("us") or 0.0))
        if a_ans.get("us"):
            m["ra_us"].append(float(a_ans.get("us") or 0.0))
        if ridx2_name and ridx2_name in raw:
            r2_ans = raw[ridx2_name]
            if r_ans.get("res") != r2_ans.get("res") or r_ans.get("outcome") != r2_ans.get("outcome"):
                m["self_diff_raw"] += 1
                if normalize_answer(row["method"], r_ans.get("res"), str(repo_dir), legend=r_ans.get("legend")) != normalize_answer(row["method"], r2_ans.get("res"), str(repo_dir), legend=r2_ans.get("legend")):
                    m["self_diff_norm"] += 1
        if a_ans["outcome"] == "a":
            m["ra_non_empty"] += 1
            if parity in ("exact_match", "core_match"):
                m["matched_on_ra_non_empty"] += 1
            if "row_precision" in ev and "row_recall" in ev:
                m["ref_precisions"].append(ev["row_precision"])
                m["ref_recalls"].append(ev["row_recall"])
                m["ref_ridx_ws"] += int(ev.get("ridx_ws", 0))
                m["ref_ra_ws"] += int(ev.get("ra_ws", 0))
                m["ref_common_ws"] += int(ev.get("common_ws", 0))
            if row["method"].startswith("semantic_tokens_") and "span_recall" in ev:
                if ev.get("ra_sym", 0) > 0:
                    m["sem_span_recalls"].append(ev["span_recall"])
                if ev.get("ra_all", 0) > 0:
                    m["sem_raw_span_recalls"].append(ev["raw_span_recall"])
                if ev.get("shared_spans", 0) > 0:
                    m["sem_type_agreements"].append(ev["type_agreement"])
                m["sem_ra_sym"] += int(ev.get("ra_sym", 0))
                m["sem_ra_lex"] += int(ev.get("ra_lexical_only", 0))
            if row["method"] == "completion" and "ra_label_recall" in ev:
                m["ra_recalls"].append(ev["ra_label_recall"])
                m["ridx_outsides"].append(ev["ridx_outside_ra_ratio"])
                if ev.get("target_in_ra") is not None:
                    m["target_eval"] += 1
                    m["target_in_ra"] += int(bool(ev.get("target_in_ra")))
                    m["target_in_ridx"] += int(bool(ev.get("target_in_ridx")))
                    m["target_in_top20"] += int(bool(ev.get("target_in_top20")))
        m[parity] += 1
        if cls:
            m["classes"][cls] += 1

    methods_summary = {}
    for key, m in per_method.items():
        denom = m["ra_non_empty"] or 1
        ref_just = int(m["classes"].get("ridx_refusal_justified", 0))
        adj_matched = m["matched_on_ra_non_empty"] + ref_just
        entry = {
            "n": m["n"],
            "ra_non_empty": m["ra_non_empty"],
            "matched_on_ra_non_empty": m["matched_on_ra_non_empty"],
            "ridx_refusal_justified": ref_just,
            "adjusted_matched_on_ra_non_empty": adj_matched,
            "exact_match": m["exact_match"],
            "core_match": m["core_match"],
            "partial": m["partial"],
            "ridx_only": m["ridx_only"],
            "both_empty": m["both_empty"],
            "mismatch": m["mismatch"],
            "parity_pct_of_total": round(
                100.0 * (m["exact_match"] + m["core_match"] + m["both_empty"]) / max(1, m["n"]), 1
            ),
            "matched_on_ra_non_empty_pct": round(100.0 * m["matched_on_ra_non_empty"] / denom, 1),
            "adjusted_on_ra_non_empty_pct": round(100.0 * adj_matched / denom, 1),
            "ridx_p50_us": _pct(m["ridx_us"], 0.50),
            "ridx_p95_us": _pct(m["ridx_us"], 0.95),
            "ridx_p99_us": _pct(m["ridx_us"], 0.99),
            "ra_p50_us": _pct(m["ra_us"], 0.50),
            "ra_p95_us": _pct(m["ra_us"], 0.95),
            "ra_p99_us": _pct(m["ra_us"], 0.99),
            "self_diff_raw": m["self_diff_raw"],
            "self_diff_norm": m["self_diff_norm"],
            "classes": dict(sorted(m["classes"].items())),
        }
        if m["ref_precisions"]:
            entry.update({
                "mean_row_precision_pct": round(100.0 * sum(m["ref_precisions"]) / len(m["ref_precisions"]), 1),
                "mean_row_recall_pct": round(100.0 * sum(m["ref_recalls"]) / len(m["ref_recalls"]), 1),
                "line_precision_pct": round(100.0 * m["ref_common_ws"] / max(1, m["ref_ridx_ws"]), 1),
                "line_recall_pct": round(100.0 * m["ref_common_ws"] / max(1, m["ref_ra_ws"]), 1),
                "total_ridx_ws_lines": m["ref_ridx_ws"],
                "total_ra_ws_lines": m["ref_ra_ws"],
                "total_common_ws_lines": m["ref_common_ws"],
            })
        if key.startswith("semantic_tokens_") and (m["sem_span_recalls"] or m["sem_raw_span_recalls"]):
            entry.update({
                "mean_span_recall_pct": round(100.0 * sum(m["sem_span_recalls"]) / max(1, len(m["sem_span_recalls"])), 1),
                "mean_raw_span_recall_pct": round(100.0 * sum(m["sem_raw_span_recalls"]) / max(1, len(m["sem_raw_span_recalls"])), 1),
                "mean_type_agreement_pct": round(100.0 * sum(m["sem_type_agreements"]) / max(1, len(m["sem_type_agreements"])), 1),
                "total_ra_sym_tokens": m["sem_ra_sym"],
                "total_ra_lexical_only_tokens": m["sem_ra_lex"],
            })
        if key == "completion" and m["ra_recalls"]:
            teval = m["target_eval"] or 1
            entry.update({
                "mean_ra_label_recall_pct": round(100.0 * sum(m["ra_recalls"]) / len(m["ra_recalls"]), 1),
                "mean_ridx_outside_ra_pct": round(100.0 * sum(m["ridx_outsides"]) / len(m["ridx_outsides"]), 1),
                "target_in_ra_count": m["target_in_ra"],
                "target_in_ridx_count": m["target_in_ridx"],
                "target_in_ridx_pct": round(100.0 * m["target_in_ridx"] / teval, 1),
                "target_in_top20_count": m["target_in_top20"],
                "target_in_top20_pct": round(100.0 * m["target_in_top20"] / teval, 1),
            })
        methods_summary[key] = entry
    return {
        "parity_totals": dict(sorted(parity_overall.items())),
        "classes_totals": dict(sorted(classes_overall.items())),
        "unclassified": classes_overall.get("unclassified", 0),
        "self_diff_raw_total": sum(m["self_diff_raw"] for m in methods_summary.values()),
        "self_diff_norm_total": sum(m["self_diff_norm"] for m in methods_summary.values()),
        "methods": methods_summary,
    }


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--repo", required=True, type=Path)
    ap.add_argument("--positions", required=True, type=Path)
    ap.add_argument("--ridx", action="append", required=True, metavar="NAME=BIN")
    ap.add_argument("--ra", action="append", required=True, metavar="NAME=BIN")
    ap.add_argument("--methods", default=",".join(DEFAULT_METHODS))
    ap.add_argument("--limit", type=int, default=None)
    ap.add_argument("--timeout", type=float, default=2.0)
    ap.add_argument("--file-table", action="append", default=[], type=Path)
    ap.add_argument("--out", action="append", required=True, type=Path)
    ap.add_argument("--out-differing", action="append", default=[], type=Path)
    ap.add_argument("--save-raw-pkl", action="store_true", help="Also write <out>.raw.pkl with raw server responses")
    args = ap.parse_args(argv)

    servers = parse_servers(args.ridx, args.ra)
    ridx_srvs = [s for s in servers if not s[2]]
    ra_srv = next(s for s in servers if s[2])
    repo_dir = args.repo.resolve()
    shards = check_ridx_index(repo_dir)
    payload = json.loads(args.positions.read_text(encoding="utf-8"))
    positions = payload["positions"]
    if args.methods == "all":
        methods = list(ALL_PARITY_METHODS)
    elif args.methods == "full_matrix":
        methods = list(REQUEST_KEYS)
    else:
        methods = [m.strip() for m in args.methods.split(",") if m.strip()]
    unknown = [m for m in methods if m not in REQUEST_KEYS]
    if unknown:
        ap.error(f"unknown matrix request methods: {unknown}")

    ws_table = load_file_table(args.file_table) if args.file_table else None
    entries = load_target_entries(repo_dir)
    cache = {}

    def read_lines(rel):
        if rel not in cache:
            try:
                cache[rel] = (repo_dir / rel).read_text(encoding="utf-8", errors="replace").split("\n")
            except OSError:
                cache[rel] = []
        return cache[rel]

    raw_rows = build_rows(positions, methods, args.limit, repo_dir, entries, read_lines)
    prepare_parity_rows(raw_rows, positions, repo_dir, entries, read_lines)
    canary = [scenario_from_record("definition", rec, repo_dir) for rec in (positions.get("definition") or [])[:3]]
    repo_head = subprocess.run(["git", "-C", str(repo_dir), "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip()
    started = datetime.now(timezone.utc).isoformat()
    runs_by_name = {}
    for name, cmd, is_ra, binary in [*ridx_srvs, ra_srv]:
        print(f"{repo_dir.name}: {'rust-analyzer' if is_ra else 'ridx'} {name} ({binary}), {len(raw_rows)} requests ...", flush=True)
        ready, seconds = ask_server(name, cmd, is_ra, raw_rows, repo_dir, args.timeout, canary, read_lines)
        runs_by_name[name] = {
            "name": name, "kind": "rust-analyzer" if is_ra else "ridx", "binary": binary_label(binary),
            **parse_version(version_of(binary)), "ready": ready, "duration_s": seconds,
        }

    rc = 0
    for idx, out_path in enumerate(args.out):
        ridx_srv = ridx_srvs[min(idx, len(ridx_srvs) - 1)]
        ridx2_name = next((s[0] for s in ridx_srvs if s[0] != ridx_srv[0]), None)
        if args.save_raw_pkl:
            out_path.parent.mkdir(parents=True, exist_ok=True)
            pkl_path = out_path.with_name(out_path.name + ".raw.pkl")
            with pkl_path.open("wb") as fh:
                pickle.dump(raw_rows, fh)
        rows = [dict(r, _raw=dict(r["_raw"])) for r in raw_rows]
        summary = evaluate_all_rows(rows, ridx_srv[0], ra_srv[0], repo_dir, read_lines, ws_table, ridx2_name)
        print(f"[{ridx_srv[0]} vs {ra_srv[0]}]", flush=True)
        for key, m in summary["methods"].items():
            adj_str = (
                f" (adj={m['adjusted_on_ra_non_empty_pct']:5.1f}%, ref_just={m['ridx_refusal_justified']})"
                if m.get("ridx_refusal_justified") else ""
            )
            print(
                f"  {key:24s} exact={m['exact_match']:3d} core={m['core_match']:3d} "
                f"partial={m['partial']:3d} ridx_only={m['ridx_only']:2d} "
                f"empty={m['both_empty']:3d} mismatch={m['mismatch']:3d} "
                f"parity_on_ra={m['matched_on_ra_non_empty_pct']:5.1f}%{adj_str} "
                f"p50={m['ridx_p50_us']:.0f}/{m['ra_p50_us']:.0f}us "
                f"self_diff={m['self_diff_raw']}/{m['self_diff_norm']} classes={m['classes']}",
                flush=True,
            )
        runs = [runs_by_name[ridx_srv[0]], runs_by_name[ra_srv[0]]] + [
            runs_by_name[s[0]] for s in ridx_srvs if s[0] != ridx_srv[0]
        ]
        report = {
            "schema": "position_ab_parity/1",
            "repo": normalize_repo_identifier(repo_dir),
            "repo_dir_name": repo_dir.name,
            "repo_head": repo_head,
            "node": socket.gethostname(),
            "python": sys.version.split()[0],
            "index_shards": shards,
            "positions_file": args.positions.name,
            "positions_seed": payload.get("seed"),
            "methods": methods,
            "limit": args.limit,
            "timeout_s": args.timeout,
            "timestamp_start": started,
            "timestamp_end": datetime.now(timezone.utc).isoformat(),
            "servers": runs,
            "summary": summary,
            "rows": rows,
        }
        out_path.parent.mkdir(parents=True, exist_ok=True)
        full_text = json.dumps(report, indent=1) + "\n"
        out_path.write_text(full_text, encoding="utf-8")
        print(f"wrote {out_path} (unclassified={summary['unclassified']})")
        if idx < len(args.out_differing) and args.out_differing[idx]:
            diff_path = args.out_differing[idx]
            full_sha = hashlib.sha256(full_text.encode("utf-8")).hexdigest()
            diff_rows = [r for r in rows if r["parity"] not in ("exact_match", "core_match", "both_empty")]
            diff_rep = {k: v for k, v in report.items() if k != "rows"}
            diff_rep.update({
                "rows": diff_rows, "rows_total": len(rows), "rows_kept": len(diff_rows),
                "full_file": {"name": out_path.name, "sha256": full_sha},
            })
            diff_path.parent.mkdir(parents=True, exist_ok=True)
            diff_path.write_text(json.dumps(diff_rep, indent=1) + "\n", encoding="utf-8")
            print(f"wrote {diff_path} ({len(diff_rows)} differing rows)")
        if summary["unclassified"]:
            rc = 1
    return rc


if __name__ == "__main__":
    sys.exit(main())
