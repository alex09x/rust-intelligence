# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""
Shared pieces of the matrix-based publications (the comparison report and the site export): the
headline methods, the loader for published runs, latency formatting and the coverage sets derived
from the a/b/c counters. Everything is read from bench/published/matrix_<repo>_booster.json.
"""

import json
from pathlib import Path
from typing import Any, Callable, Dict, List, Tuple

from matrix_categories import LSP_SPECS, SYNTHETIC_ITEM_KEYS
from matrix_report import REPO_ORDER

HEADLINE_METHODS: List[Tuple[str, str]] = [
    ("definition", "Go to definition"),
    ("type_definition", "Go to type definition"),
    ("references", "Find references"),
    ("hover", "Hover"),
    ("completion", "Completion"),
    ("signature_help", "Signature help"),
    ("document_symbol", "Document symbols"),
    ("workspace_symbol", "Workspace symbol search"),
    ("rename", "Rename"),
    ("code_lens", "Code lens"),
    ("semantic_tokens_full", "Semantic tokens (full)"),
    ("folding_range", "Folding ranges"),
]
SITE_METHODS = ["definition", "references", "hover", "completion", "document_symbol", "rename"]

# Request rows whose item comes from the corpus. Statements about a/b/c are made only about these:
# notifications, lifecycle, server-to-client and mutation rows are out, and so are the rows that send
# an item built by the harness (SYNTHETIC_ITEM_KEYS).
# Rows the published runs measured with a defective harness: excluded from the counts, the coverage
# statements and the headline tables, and named in the notes (known harness defects in
# docs/BENCHMARK_MATRIX.md). Each entry is (rpc method, reason); the reason is the clause after
# "is not counted:" in the notes, with code spans in backticks.
# - `textDocument/content` was sent as {"textDocument": {"uri"}} until harness commit d27ac0e and
#   RIDX answered an empty text for every item.
# - `textDocument/codeLens`: the client declares neither `textDocument.codeLens` nor the
#   `rust-analyzer.*` commands (`experimental.commands`), and rust-analyzer builds no lens without
#   them, so it answered `[]` on every item of every published run.
# Drop an entry once a run measured with the fixed harness is published.
HARNESS_DEFECT_ROWS = {
    "text_document_content": ("textDocument/content",
                              "the published runs were measured before harness commit `d27ac0e` fixed its parameters"),
    "code_lens": ("textDocument/codeLens",
                  "the matrix client declares neither `textDocument.codeLens` nor the `rust-analyzer.*` commands "
                  "(`experimental.commands`) that rust-analyzer requires before it builds any lens, so "
                  "rust-analyzer answered `[]` on every item"),
}
REQUEST_KEYS = frozenset(
    key for _, key, _, _, rpc_type in LSP_SPECS if rpc_type == "request"
) - SYNTHETIC_ITEM_KEYS - frozenset(HARNESS_DEFECT_ROWS)
METHOD_NOT_FOUND = "-32601"
REPO_KEYS = [key for key, _ in REPO_ORDER]
CANONICAL = dict(REPO_ORDER)


def load_published(pub_dir: Path) -> List[Tuple[str, Dict[str, Any]]]:
    """Published runs in REPO_ORDER, each with its provenance sidecar present."""
    runs = []
    for key in REPO_KEYS:
        run = pub_dir / f"matrix_{key}_booster.json"
        sidecar = pub_dir / f"matrix_{key}_booster.provenance.json"
        if run.is_file() and sidecar.is_file():
            with open(run, "r", encoding="utf-8") as fh:
                runs.append((key, json.load(fh)))
    return runs


def fmt_us(v: Any) -> str:
    if not isinstance(v, (int, float)):
        return "n/a"
    if v < 1000:
        return f"{v:.0f} µs"
    if v < 1_000_000:
        return f"{v / 1000.0:.2f} ms"
    return f"{v / 1_000_000.0:.2f} s"


def seconds(v: Any) -> float:
    """Microseconds to seconds for the site bars, rounded to the nanosecond."""
    return round(v / 1_000_000.0, 9) if isinstance(v, (int, float)) else 0.0


def abc(m: Dict[str, Any]) -> str:
    if "a_non_empty" not in m:
        return "n/a"
    return f"{m.get('a_non_empty', 0)} / {m.get('b_empty', 0)} / {m.get('c_error', 0)}"


def short(sha: Any) -> str:
    return str(sha)[:7] if isinstance(sha, str) and sha else "n/a"


def distinct(runs: List[Tuple[str, Dict[str, Any]]], field: str) -> List[str]:
    return sorted({str(data.get(field, "n/a")) for _, data in runs})


def _a(methods: Dict[str, Any], key: str) -> int:
    return int(methods.get(key, {}).get("a_non_empty", 0) or 0)


def answered(methods: Dict[str, Any]) -> int:
    """Request methods with at least one non-empty answer."""
    return sum(1 for key in REQUEST_KEYS if _a(methods, key) > 0)


def _not_implemented(m: Dict[str, Any]) -> bool:
    run = int(m.get("tests_run", 0) or 0)
    return run > 0 and int(m.get("error_codes", {}).get(METHOD_NOT_FOUND, 0)) == run


def ridx_silent(data: Dict[str, Any]) -> List[str]:
    """Methods RIDX never answered non-empty while rust-analyzer did."""
    ridx, ra = data["ridx"]["methods"], data["rust_analyzer"]["methods"]
    return sorted(ridx[k]["rpc_method"] for k in REQUEST_KEYS
                  if k in ridx and _a(ridx, k) == 0 and _a(ra, k) > 0)


def ra_not_implemented(data: Dict[str, Any]) -> List[str]:
    """Methods rust-analyzer answered with -32601 on every item."""
    ra = data["rust_analyzer"]["methods"]
    return sorted(ra[k]["rpc_method"] for k in REQUEST_KEYS if k in ra and _not_implemented(ra[k]))


def ra_silent(data: Dict[str, Any]) -> List[str]:
    """Methods rust-analyzer implements but never answered non-empty while RIDX did."""
    ridx, ra = data["ridx"]["methods"], data["rust_analyzer"]["methods"]
    return sorted(ra[k]["rpc_method"] for k in REQUEST_KEYS
                  if k in ra and _a(ra, k) == 0 and _a(ridx, k) > 0 and not _not_implemented(ra[k]))


def by_method(runs: List[Tuple[str, Dict[str, Any]]],
              select: Callable[[Dict[str, Any]], List[str]]) -> Dict[str, List[str]]:
    """method -> canonical repository names (in REPO_ORDER) where `select` lists it."""
    out: Dict[str, List[str]] = {}
    for key, data in runs:
        for method in select(data):
            out.setdefault(method, []).append(CANONICAL.get(key, key))
    return dict(sorted(out.items()))


def where(repos: List[str], total: int) -> str:
    return "all five" if total == 5 and len(repos) == total else ", ".join(repos)


def defect_notes(code: bool) -> List[str]:
    """One sentence per row excluded because of a harness defect in the published runs; without
    `code` the backticks are dropped for plain-text contexts."""
    out = []
    for _, (rpc, reason) in sorted(HARNESS_DEFECT_ROWS.items()):
        note = f"`{rpc}` is not counted: {reason} (known harness defects in `docs/BENCHMARK_MATRIX.md`)."
        out.append(note if code else note.replace("`", ""))
    return out
