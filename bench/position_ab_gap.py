# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Explains, row by row, why RIDX answered empty where rust-analyzer answered (and the reverse)
in a position_ab report that paired a ridx binary with rust-analyzer, and writes a sidecar.

    position_ab_gap.py --repo DIR --report position_ab_<sha>_vs_ra_ram9_<corpus>.json \\
        --ridx BIN --ra BIN --inspect BIN --file-table <corpus>.ws.tsv --out ....gap_classes.json

Row sets, from the first answers and the rust-analyzer retry of the report: `target` (RIDX b,
rust-analyzer a, or c then a on retry), `reverse` (RIDX a, rust-analyzer b also on retry),
`ra_noise` (rust-analyzer b then a on retry), `ra_timeout` (rust-analyzer c without an answer on
retry). Target and reverse rows are asked again: rust-analyzer for the method itself and for
`definition` at the cursor, ridx for `definition` and `prepareCallHierarchy` at the cursor, and
the inspection binary's `refs --json` for the references RIDX holds at the cursor with their
`is_macro_expansion` flag. Every location is reduced to its realm (`ws:`, `registry:`, `git:`,
`sysroot:`, `external:`) and, for workspace files, to whether a workspace shard stores it (the
`FILE` lines of the file_table example). The class of a row follows from that evidence alone, by
the rules of position_ab_gap_rules; an unexplained row is `unclassified` and the exit code is 1.
"""

import argparse
import hashlib
import json
import subprocess
import sys
from collections import Counter
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from code_lens_latency import version_of  # noqa: E402
from lsp_process import SimpleLspProcess  # noqa: E402
from matrix_canary import ensure_ra_ready  # noqa: E402
from matrix_categories import LSP_SPECS, build_params  # noqa: E402
from position_ab_classify import classify_outcome, digest, scenario_from_record  # noqa: E402
from position_ab_content import load_file_table  # noqa: E402
from position_ab_content_rules import scrub_path, uri_path  # noqa: E402
from position_ab_gap_reverse import REVERSE_ORDER, cfg_context, classify_reverse  # noqa: E402
from position_ab_gap_cursor import (  # noqa: E402
    derive_above, in_attribute_block, in_impl_header, in_macro_block, in_macro_call, in_macro_rules, qualified, token_at,
    token_class,
)
from position_ab_gap_rules import (  # noqa: E402
    RULE_ORDER, body_end_line, classify_document_highlight, classify_implementation, classify_incoming_calls,
    is_call_site, names_symbol, store_kind,
)

RPC = {key: rpc for _, key, rpc, _, _ in LSP_SPECS}
GAP_METHODS = ("implementation", "incoming_calls", "document_highlight")


def ordered(items):
    """Evidence lists in a fixed order: the servers return locations and callers in an order that
    varies between runs, the sidecar must not."""
    return sorted(items, key=lambda item: json.dumps(item, sort_keys=True))


def row_set(ridx, ra):
    """Which set a row belongs to, from the first answers and the retry: `target` (RIDX b,
    rust-analyzer a, or c then a on the retry), `ra_noise` (rust-analyzer b then a, whatever RIDX
    answered), `reverse` (RIDX a, rust-analyzer b also on the retry), `ra_timeout_then_empty`
    (rust-analyzer c then b), `ra_timeout` (rust-analyzer c with no answer on the retry), else
    `other`."""
    retry = (ra.get("retry") or {}).get("outcome")
    if ridx["outcome"] == "b" and (ra["outcome"] == "a" or (ra["outcome"] == "c" and retry == "a")):
        return "target"
    if ra["outcome"] == "b" and retry == "a":
        return "ra_noise"
    if ridx["outcome"] == "a" and ra["outcome"] == "b":
        return "reverse"
    if ra["outcome"] == "c" and retry == "b":
        return "ra_timeout_then_empty"
    if ra["outcome"] == "c" and retry != "a":
        return "ra_timeout"
    return "other"


class Gap:
    def __init__(self, repo, ridx_bin, ra_bin, inspect_bin, file_table, timeout, canary):
        self.repo = str(Path(repo).resolve())
        self.timeout = timeout
        self.inspect_bin = inspect_bin
        self.file_table = file_table
        self.lines_cache = {}
        self.ridx = SimpleLspProcess([ridx_bin, "lsp"], self.repo, is_ra=False)
        self.ra = SimpleLspProcess([ra_bin], self.repo, is_ra=True)
        self.ra_ready = ensure_ra_ready(self.ra, canary, Path(self.repo), timeout=600.0, interval=1.0)

    def close(self):
        self.ridx.close()
        self.ra.close()

    def lines(self, abs_path):
        if abs_path not in self.lines_cache:
            try:
                self.lines_cache[abs_path] = Path(abs_path).read_text(encoding="utf-8", errors="replace").split("\n")
            except OSError:
                self.lines_cache[abs_path] = []
        return self.lines_cache[abs_path]

    def line_text(self, abs_path, line_1based):
        lines = self.lines(abs_path)
        return lines[line_1based - 1] if 0 < line_1based <= len(lines) else ""

    def ask(self, srv, key, row, is_ra):
        params = build_params(key, scenario_from_record(key, row, self.repo), self.repo, is_ra=is_ra)
        res, _us, err = srv.request(RPC[key], params, timeout=self.timeout)
        outcome, code = classify_outcome(key, res, err)
        return outcome, (res if outcome == "a" else None), code

    def location(self, uri, line_0based, with_macro=False):
        """A location as the sidecar records it: realm-scrubbed path, store kind, 1-based line and
        its text; with `with_macro`, whether the line sits inside a macro invocation or body."""
        abs_path = uri_path(uri)
        out = {"path": scrub_path(abs_path, self.repo), "store": store_kind(abs_path, self.repo, self.file_table),
               "line": line_0based + 1, "line_text": self.line_text(abs_path, line_0based + 1).strip()[:120]}
        if with_macro:
            lines = self.lines(abs_path)
            out["in_macro"] = bool(lines) and (in_macro_block(lines, line_0based) or in_macro_rules(lines, line_0based))
            out["in_attribute"] = bool(lines) and in_attribute_block(lines, line_0based)
            out["in_impl_header"] = bool(lines) and in_impl_header(lines, line_0based)
            out["derive_above"] = bool(lines) and derive_above(lines, line_0based)
        return out

    def locations_of(self, res, with_macro=False):
        items = res if isinstance(res, list) else [res]
        out = []
        for it in items:
            if not isinstance(it, dict):
                continue
            uri = it.get("uri") or it.get("targetUri")
            rng = it.get("range") or it.get("targetSelectionRange") or it.get("targetRange") or {}
            if uri:
                out.append(self.location(uri, int(rng.get("start", {}).get("line", 0)), with_macro))
        return ordered(out)

    def inspect_refs(self, row):
        """RIDX's references at the cursor, from the inspection binary's `refs --json`."""
        loc = f"{Path(self.repo, row['file'])}:{row['line']}:{row['character']}"
        proc = subprocess.run([self.inspect_bin, "refs", loc, "--json"], cwd=self.repo, capture_output=True, text=True)
        try:
            data = json.loads(proc.stdout)
        except json.JSONDecodeError:
            return {"target_symbol": None, "references": [], "error": proc.stderr.strip()[-200:]}
        refs = []
        for r in data.get("references") or []:
            text = self.line_text(r["file"], int(r["end_line"]))
            refs.append({"path": scrub_path(r["file"], self.repo), "line": r["start_line"], "end_line": r["end_line"],
                         "end_col": r["end_col"], "is_macro_expansion": bool(r.get("is_macro_expansion")),
                         "call_site": is_call_site(text, int(r["end_col"])), "text_after": text[max(0, int(r["end_col"]) - 1):].strip()[:40]})
        return {"target_symbol": data.get("target_symbol"), "target_path": data.get("target_path"), "references": ordered(refs)}

    def explain_target(self, row):
        key = row["method"]
        ra_outcome, ra_res, ra_code = self.ask(self.ra, key, row, True)
        ridx_def, ridx_def_res, _ = self.ask(self.ridx, "definition", row, False)
        ev = {"ra_reask": ra_outcome, "ridx_definition": {"outcome": ridx_def, "targets": self.locations_of(ridx_def_res or [])}}
        if ra_outcome != "a":
            return "x_ra_reask_failed", f"rust-analyzer re-ask: {ra_outcome} {ra_code or ''}".strip(), ev
        if key == "implementation":
            refs = self.inspect_refs(row)["references"]
            decl = {(t["path"], t["line"]) for t in ev["ridx_definition"]["targets"]}
            ev["ra_locations"] = self.locations_of(ra_res or [], with_macro=True)
            for loc in ev["ra_locations"]:
                loc["on_decl_line"] = (loc["path"], loc["line"]) in decl
                loc["ridx_ref_at_line"] = any(r["path"] == loc["path"] and r["line"] <= loc["line"] <= r["end_line"] for r in refs)
            ev["definition_names_symbol"] = any(names_symbol(t["line_text"], row.get("symbol")) for t in ev["ridx_definition"]["targets"])
            cls, note = classify_implementation(ridx_def, ev["ra_locations"], ev["definition_names_symbol"])
        elif key == "incoming_calls":
            prep, prep_res, _ = self.ask(self.ridx, "prepare_call_hierarchy", row, False)
            ev["ridx_prepare"] = {"outcome": prep, "items": [it.get("name") for it in (prep_res or []) if isinstance(it, dict)]}
            ev["ridx_refs"] = self.inspect_refs(row)
            ev["callers"] = [self.caller(item, ev["ridx_refs"]["references"]) for item in (ra_res or []) if isinstance(item, dict)]
            for c in ev["callers"]:
                c["refs_total"] = len(ev["ridx_refs"]["references"])
            cls, note, cats = classify_incoming_calls(prep, ev["callers"])
            for c, cat in zip(ev["callers"], cats):
                c["category"] = cat
            ev["callers"] = ordered(ev["callers"])
        else:
            abs_path = str(Path(self.repo, row["file"]))
            text = self.line_text(abs_path, int(row["line"]))
            col = int(row["character"])
            token = token_at(text, col)
            ev["cursor"] = {"token": token, "token_class": token_class(token), "cursor_kind": row.get("cursor_kind"),
                            "qualified": qualified(text, col),
                            "in_macro": in_macro_call(text, col) or in_macro_rules(self.lines(abs_path), int(row["line"]) - 1),
                            "line_text": text.strip()[:120]}
            ev["ra_highlights"] = len(ra_res) if isinstance(ra_res, list) else 0
            ev["ra_definition"] = self.ra_definition(row, abs_path)
            ev["ridx_references"] = self.ridx_references(row, abs_path)
            store = ev["ridx_definition"]["targets"][0]["store"] if ev["ridx_definition"]["targets"] else None
            cls, note = classify_document_highlight(ev["cursor"], ridx_def, ev["ra_definition"], ev["ridx_references"], store)
        return cls, note, ev

    def ridx_references(self, row, abs_path):
        """RIDX `references` at the cursor: how many, how many in the cursor's file."""
        outcome, res, _ = self.ask(self.ridx, "references", row, False)
        locs = self.locations_of(res or [])
        here = scrub_path(abs_path, self.repo)
        return {"outcome": outcome, "total": len(locs), "same_file": sum(1 for loc in locs if loc["path"] == here)}

    def ra_definition(self, row, abs_path):
        """rust-analyzer's definition at the cursor: the first target in canonical order when it
        gives several, with the column and token at that target."""
        outcome, res, _ = self.ask(self.ra, "definition", row, True)
        items = [it for it in (res if isinstance(res, list) else [res]) if isinstance(it, dict) and (it.get("uri") or it.get("targetUri"))]
        if not items:
            return None
        pairs = [(self.locations_of([it], with_macro=True)[0], it) for it in items]
        loc, first = min(pairs, key=lambda pair: json.dumps(pair[0], sort_keys=True))
        start = (first.get("targetSelectionRange") or first.get("range") or {}).get("start", {})
        target_abs = uri_path(first.get("targetUri") or first.get("uri"))
        text = self.line_text(target_abs, int(start.get("line", 0)) + 1)
        col = int(start.get("character", 0)) + 1
        return {**loc, "col": col, "token": token_at(text, col), "same_file": target_abs == abs_path, "count": len(items)}

    def caller(self, item, refs, probe=True):
        """One `from` item of an incomingCalls answer with RIDX's evidence about it: the references
        RIDX holds at its call lines, whether those lines sit in macro invocations, whether RIDX's
        `definition` at the caller's name resolves to its line, and the handler's body end."""
        frm = item.get("from") or {}
        sel = (frm.get("selectionRange") or frm.get("range") or {}).get("start", {})
        abs_path = uri_path(frm.get("uri", ""))
        loc = self.location(frm.get("uri", ""), int(sel.get("line", 0)))
        from_lines = sorted({int(r.get("start", {}).get("line", 0)) + 1 for r in item.get("fromRanges") or []})
        lines = self.lines(abs_path)
        out = {**loc, "name": frm.get("name"), "kind": frm.get("kind"), "from_lines": from_lines,
               "refs": [r for r in refs if r["path"] == loc["path"] and r["line"] in from_lines],
               "from_in_macro": bool(from_lines) and all(in_macro_call(self.line_text(abs_path, ln), len(self.line_text(abs_path, ln)) + 1)
                                                         or in_macro_rules(lines, ln - 1) for ln in from_lines)}
        if probe and loc["store"] == "ws":
            rel = str(Path(abs_path).relative_to(self.repo)) if abs_path.startswith(self.repo) else abs_path
            probe_row = {"file": rel, "line": loc["line"], "character": int(sel.get("character", 0)) + 1, "name": "", "symbol": frm.get("name")}
            _o, res, _ = self.ask(self.ridx, "definition", probe_row, False)
            out["ridx_def_at_caller"] = any(t["path"] == loc["path"] and t["line"] == loc["line"] for t in self.locations_of(res or []))
            out["body_end"] = body_end_line(lines, loc["line"]) if lines else None
            out["call_line"] = min(from_lines) if from_lines else None
        return out

    def explain_reverse(self, row):
        key = row["method"]
        ridx_outcome, ridx_res, _ = self.ask(self.ridx, key, row, False)
        ra_outcome, _ra_res, ra_code = self.ask(self.ra, key, row, True)
        abs_path = str(Path(self.repo, row["file"]))
        text = self.line_text(abs_path, int(row["line"]))
        col, idx = int(row["character"]), int(row["line"]) - 1
        token = token_at(text, col)
        lines = self.lines(abs_path)
        ev = {"ridx_reask": ridx_outcome, "ra_reask": ra_outcome, "ra_error": ra_code,
              "cursor": {"token": token, "token_class": token_class(token), "cursor_kind": row.get("cursor_kind"), "line_text": text.strip()[:120],
                         "in_macro": in_macro_call(text, col) or in_macro_rules(lines, idx) or in_macro_block(lines, idx),
                         "cfg_context": cfg_context(lines, idx) if lines else None},
              "ridx_answer": self.locations_of(ridx_res or []) if key != "document_highlight" else {"highlights": len(ridx_res or [])},
              "ra_definition": self.ra_definition(row, abs_path)}
        if key == "incoming_calls":
            ev["ridx_answer"] = ordered([self.caller(item, [], probe=False) for item in (ridx_res or []) if isinstance(item, dict)])
        ev["symbol"] = row.get("symbol")
        cls, note = classify_reverse(key, ev)
        return cls, note, ev


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--repo", required=True)
    ap.add_argument("--report", required=True, type=Path, help="full position_ab report (ridx and rust-analyzer servers)")
    ap.add_argument("--ridx", required=True, help="the ridx binary of the report, for definition and prepareCallHierarchy")
    ap.add_argument("--ra", required=True, help="the rust-analyzer binary of the report")
    ap.add_argument("--inspect", required=True, help="ridx binary whose `refs --json` reports is_macro_expansion")
    ap.add_argument("--file-table", action="append", default=[], type=Path, help="file_table output of the workspace shards")
    ap.add_argument("--timeout", type=float, default=2.0)
    ap.add_argument("--limit", type=int, default=None, help="rows per set and method, for smoke runs")
    ap.add_argument("--out", required=True, type=Path)
    args = ap.parse_args(argv)

    text = args.report.read_bytes()
    report = json.loads(text)
    ridx_name = next(s["name"] for s in report["servers"] if s["kind"] == "ridx")
    ra_name = next(s["name"] for s in report["servers"] if s["kind"] == "rust-analyzer")
    sets = {}
    for row in report["rows"]:
        if row["method"] in GAP_METHODS:
            sets.setdefault(row_set(row["results"][ridx_name], row["results"][ra_name]), []).append(row)
    set_counts = {name: dict(Counter(r["method"] for r in rows)) for name, rows in sorted(sets.items())}
    print("sets:", json.dumps(set_counts), flush=True)

    work = {name: sets.get(name, []) for name in ("target", "reverse")}
    if args.limit:
        for name, rows in work.items():
            per = Counter()
            kept = []
            for r in rows:
                if per[r["method"]] < args.limit:
                    kept.append(r)
                    per[r["method"]] += 1
            work[name] = kept
    canary = [scenario_from_record("definition", r, args.repo) for r in (work["target"] + work["reverse"])[:3]]
    file_table = load_file_table(args.file_table)
    gap = Gap(args.repo, args.ridx, args.ra, args.inspect, file_table, args.timeout, canary)
    out_rows = []
    try:
        for set_name, rows in work.items():
            for i, row in enumerate(rows):
                cls, note, ev = gap.explain_target(row) if set_name == "target" else gap.explain_reverse(row)
                out_rows.append({"set": set_name, "method": row["method"], "name": row["name"], "file": row["file"],
                                 "line": row["line"], "character": row["character"], "symbol": row.get("symbol"),
                                 "target_kind": row.get("target_kind"), "cfg_test": row.get("cfg_test"),
                                 "first": {"ridx": row["results"][ridx_name]["outcome"], "ra": row["results"][ra_name]["outcome"],
                                           "ra_retry": (row["results"][ra_name].get("retry") or {}).get("outcome")},
                                 "class": cls, "note": note, "evidence": ev})
                if (i + 1) % 25 == 0:
                    print(f"  {set_name}: {i + 1}/{len(rows)}", flush=True)
    finally:
        gap.close()
    classes = {}
    for r in out_rows:
        classes.setdefault(r["set"], {}).setdefault(r["method"], Counter())[r["class"]] += 1
    classes = {s: {m: dict(c) for m, c in ms.items()} for s, ms in classes.items()}
    unclassified = sum(1 for r in out_rows if r["class"] == "unclassified")
    reask_failed = sum(1 for r in out_rows if r["class"] == "x_ra_reask_failed")
    sidecar = {
        "repo": report.get("repo"), "repo_head": report.get("repo_head"),
        "report": {"name": args.report.name, "sha256": hashlib.sha256(text).hexdigest(), "rows_total": len(report["rows"])},
        "servers": {"ridx": version_of(args.ridx), "rust_analyzer": version_of(args.ra), "inspect": version_of(args.inspect)},
        "ra_ready": gap.ra_ready, "file_table_entries": len(file_table), "limit": args.limit,
        "sets": set_counts, "rule_order": {**RULE_ORDER, "reverse": REVERSE_ORDER}, "classes": classes,
        "unclassified": unclassified, "ra_reask_failed": reask_failed,
        "rows": out_rows,
        "definitions": {
            "sets": "target: RIDX b and rust-analyzer a (or c, then a on retry); ra_noise: rust-analyzer b then a on retry, "
                    "whatever RIDX answered; reverse: RIDX a and rust-analyzer b, also on retry; ra_timeout_then_empty: "
                    "rust-analyzer c then b on retry; ra_timeout: rust-analyzer c with no answer on retry; other: the rest",
            "store": "ws: a workspace shard stores the file (FILE lines of file_table); ws_unstored: workspace file no shard "
                     "stores; registry/git/sysroot/external: realm of a file outside the workspace",
            "refs": "references RIDX holds at the cursor (inspection binary, refs --json): line, is_macro_expansion, and whether "
                    "the text after the span is a call site by the handler's rule",
            "class": "first rule of rule_order that holds for the row's evidence; unclassified when none does; "
                     "x_ra_reask_failed when rust-analyzer did not answer the re-ask, so the row has no content to classify",
        },
    }
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(sidecar, indent=1), encoding="utf-8")
    for s, ms in classes.items():
        for m, c in ms.items():
            print(f"{s:8s} {m:20s} " + ", ".join(f"{k} {v}" for k, v in sorted(c.items())), flush=True)
    print(f"wrote {args.out}; unclassified {unclassified}; ra re-ask failed {reask_failed}", flush=True)
    return 1 if unclassified else 0


if __name__ == "__main__":
    sys.exit(main())
