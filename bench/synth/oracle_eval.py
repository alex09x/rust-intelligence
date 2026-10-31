# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Executes synthetic ground-truth cases against an LSP server and evaluates 100% oracle parity."""

from pathlib import Path
from typing import Dict, List, Tuple
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from lsp_process import SimpleLspProcess  # noqa: E402
from position_ab_content_rules import uri_path  # noqa: E402
from position_ab_parity_norm import normalize_hover  # noqa: E402
from position_ab_parity_norm_ext import (  # noqa: E402
    normalize_code_lens,
    normalize_document_link,
    normalize_document_symbol_ext,
    normalize_folding_range,
    normalize_inlay_hint,
    normalize_linked_editing_range,
    normalize_moniker,
    normalize_selection_range,
    normalize_semantic_tokens_any,
    normalize_signature_help_ext,
    normalize_workspace_symbol_ext,
)
from .oracle_cases import build_oracle_cases  # noqa: E402


def _rel_path(uri: str, repo_dir: Path) -> str:
    p = Path(uri_path(uri) or "").resolve()
    try:
        return p.relative_to(repo_dir.resolve()).as_posix()
    except ValueError:
        return p.as_posix()


def _rng_tuple(rel: str, rng: dict) -> Tuple[str, int, int, int, int]:
    s, e = (rng or {}).get("start") or {}, (rng or {}).get("end") or {}
    return (rel, int(s.get("line", 0)), int(s.get("character", 0)), int(e.get("line", 0)), int(e.get("character", 0)))


def _extract_loc_lines(res, repo_dir: Path) -> List[Tuple[str, int]]:
    if not res:
        return []
    items = res if isinstance(res, list) else [res]
    out = set()
    for it in items:
        if isinstance(it, dict):
            uri = it.get("uri") or it.get("targetUri")
            rng = it.get("targetSelectionRange") or it.get("range") or it.get("targetRange") or {}
            if uri and "start" in rng:
                out.add((_rel_path(uri, repo_dir), int(rng["start"].get("line", 0))))
    return sorted(out)


def _extract_exact_spans(res, repo_dir: Path, default_rel: str = "") -> List[Tuple[str, int, int, int, int]]:
    if not isinstance(res, list):
        return []
    return sorted({
        _rng_tuple(_rel_path(it["uri"], repo_dir) if it.get("uri") else default_rel, it.get("range") or {})
        for it in res if isinstance(it, dict)
    })


def _extract_rename_spans(res, repo_dir: Path) -> Tuple[List[Tuple[str, int, int, int, int]], set]:
    if not isinstance(res, dict):
        return [], set()
    spans, new_texts = set(), set()
    for uri, edits in (res.get("changes") or {}).items():
        rel = _rel_path(uri, repo_dir)
        for ed in edits or []:
            spans.add(_rng_tuple(rel, ed.get("range") or {}))
            new_texts.add(str(ed.get("newText") or ""))
    for doc_ed in res.get("documentChanges") or []:
        if isinstance(doc_ed, dict) and "textDocument" in doc_ed:
            rel = _rel_path((doc_ed["textDocument"] or {}).get("uri", ""), repo_dir)
            for ed in doc_ed.get("edits") or []:
                spans.add(_rng_tuple(rel, ed.get("range") or {}))
                new_texts.add(str(ed.get("newText") or ""))
    return sorted(spans), new_texts


def _apply_text_edits(text: str, edits: list) -> str:
    if not isinstance(edits, list) or not edits:
        return text
    lines = text.splitlines(keepends=True)
    offsets = [0]
    for ln in lines:
        offsets.append(offsets[-1] + len(ln))

    def to_off(pos: dict) -> int:
        l_idx = min(max(0, int(pos.get("line", 0))), len(lines))
        return len(text) if l_idx == len(lines) else min(offsets[l_idx] + int(pos.get("character", 0)), offsets[l_idx + 1])

    parsed = [
        (to_off(ed["range"].get("start") or {}), to_off(ed["range"].get("end") or {}), str(ed.get("newText") or ""))
        for ed in edits if isinstance(ed, dict) and isinstance(ed.get("range"), dict)
    ]
    out = text
    for start, end, repl in sorted(parsed, key=lambda x: (x[0], x[1]), reverse=True):
        out = out[:start] + repl + out[end:]
    return out


def _run_single_case(srv: SimpleLspProcess, case: dict, repo_dir: Path, timeout: float) -> Tuple[bool, str, float]:
    m = case["method"]
    cur = case["cursor"]
    rel = cur["rel_path"]
    uri = (repo_dir / rel).resolve().as_uri()
    pos = {"line": cur["line"], "character": cur["col"]}
    td = {"uri": uri}
    rng_span = {
        "start": {"line": cur["line"], "character": cur["col"]},
        "end": {"line": cur["end_line"], "character": cur["end_col"]},
    }

    if m in ("definition", "declaration", "type_definition", "implementation"):
        rpc = {
            "definition": "textDocument/definition",
            "declaration": "textDocument/declaration",
            "type_definition": "textDocument/typeDefinition",
            "implementation": "textDocument/implementation",
        }[m]
        res, us, _ = srv.request(rpc, {"textDocument": td, "position": pos}, timeout=timeout)
        actual = _extract_loc_lines(res, repo_dir)
        expected = sorted({(s[0], s[1]) for s in case["expected_spans"]})
        return actual == expected, f"expected={expected}, actual={actual}", us

    if m == "references":
        params = {"textDocument": td, "position": pos, "context": {"includeDeclaration": True}}
        res, us, _ = srv.request("textDocument/references", params, timeout=timeout)
        actual = _extract_exact_spans(res, repo_dir)
        expected = case["expected_spans"]
        return actual == expected, f"expected={len(expected)} spans, actual={len(actual)} spans", us

    if m == "document_highlight":
        res, us, _ = srv.request("textDocument/documentHighlight", {"textDocument": td, "position": pos}, timeout=timeout)
        actual = _extract_exact_spans(res, repo_dir, default_rel=rel)
        expected = case["expected_spans"]
        return actual == expected, f"expected={expected}, actual={actual}", us

    if m == "rename":
        params = {"textDocument": td, "position": pos, "newName": case["new_name"]}
        res, us, _ = srv.request("textDocument/rename", params, timeout=timeout)
        actual, new_texts = _extract_rename_spans(res, repo_dir)
        expected = case["expected_spans"]
        ok = (actual == expected) and (new_texts == {case["new_name"]})
        return ok, f"expected={len(expected)} edits, actual={len(actual)} edits", us

    if m == "prepare_rename":
        res, us, _ = srv.request("textDocument/prepareRename", {"textDocument": td, "position": pos}, timeout=timeout)
        if not isinstance(res, dict):
            return False, f"non-dict response: {res!r}", us
        actual = [_rng_tuple(rel, res.get("range") or res)]
        return actual == case["expected_spans"], f"expected={case['expected_spans']}, actual={actual}", us

    if m == "linked_editing_range":
        res, us, _ = srv.request("textDocument/linkedEditingRange", {"textDocument": td, "position": pos}, timeout=timeout)
        ranges = normalize_linked_editing_range(res)
        actual = [(rel, sl - 1, sc, el - 1, ec) for sl, sc, el, ec in ranges]
        return actual == case["expected_spans"], f"expected={case['expected_spans']}, actual={actual}", us

    if m in ("prepare_call_hierarchy", "incoming_calls", "outgoing_calls"):
        items, us1, _ = srv.request("textDocument/prepareCallHierarchy", {"textDocument": td, "position": pos}, timeout=timeout)
        if not isinstance(items, list) or not items:
            return False, "empty prepareCallHierarchy", us1
        item0 = items[0]
        if m == "prepare_call_hierarchy":
            return item0.get("name") == case["expected_name"], f"got={item0.get('name')!r}", us1
        rpc = "callHierarchy/incomingCalls" if m == "incoming_calls" else "callHierarchy/outgoingCalls"
        key = "from" if m == "incoming_calls" else "to"
        res, us2, _ = srv.request(rpc, {"item": item0}, timeout=timeout)
        names = sorted({str((it.get(key) or {}).get("name") or "") for it in (res or []) if isinstance(it, dict)})
        return names == case["expected_names"], f"expected={case['expected_names']}, actual={names}", us1 + us2

    if m in ("prepare_type_hierarchy", "supertypes", "subtypes"):
        items, us1, _ = srv.request("textDocument/prepareTypeHierarchy", {"textDocument": td, "position": pos}, timeout=timeout)
        if not isinstance(items, list) or not items:
            return False, "empty prepareTypeHierarchy", us1
        item0 = items[0]
        if m == "prepare_type_hierarchy":
            return item0.get("name") == case["expected_name"], f"got={item0.get('name')!r}", us1
        rpc = "typeHierarchy/supertypes" if m == "supertypes" else "typeHierarchy/subtypes"
        res, us2, _ = srv.request(rpc, {"item": item0}, timeout=timeout)
        names = sorted({str(it.get("name") or "") for it in (res or []) if isinstance(it, dict)})
        expected = set(case["expected_names"])
        return expected.issubset(set(names)), f"expected_subset={sorted(expected)}, actual={names}", us1 + us2

    if m == "hover":
        res, us, _ = srv.request("textDocument/hover", {"textDocument": td, "position": pos}, timeout=timeout)
        nh = normalize_hover(res)
        return case["expected_substr"] in nh["text"], f"sig={nh['sig_line']!r}", us

    if m == "signature_help":
        res, us, _ = srv.request("textDocument/signatureHelp", {"textDocument": td, "position": pos}, timeout=timeout)
        ns = normalize_signature_help_ext(res)
        lbl = ns["signatures"][0] if ns["signatures"] else ""
        ok = bool(ns["signatures"]) and (case["expected_substr"] in lbl) and (ns["active_param"] == case["expected_active_param"])
        return ok, f"label={lbl!r}, active_param={ns['active_param']}", us

    if m in ("inlay_hint", "inlay_hint_resolve"):
        full_rng = {"start": {"line": 0, "character": 0}, "end": {"line": 200, "character": 0}}
        res, us, _ = srv.request("textDocument/inlayHint", {"textDocument": td, "range": full_rng}, timeout=timeout)
        nh = normalize_inlay_hint(res)
        ok = any(case["expected_substr"] in lbl for _, _, lbl, _ in nh["hints"])
        if m == "inlay_hint_resolve" and isinstance(res, list) and res:
            r2, us2, err2 = srv.request("inlayHint/resolve", res[0], timeout=timeout)
            return ok and isinstance(r2, dict) and err2 is None, f"resolved={isinstance(r2, dict)}", us + us2
        return ok, f"hints={nh['hints']}", us

    if m == "document_symbol":
        res, us, _ = srv.request("textDocument/documentSymbol", {"textDocument": td}, timeout=timeout)
        nd = normalize_document_symbol_ext(res)
        missing = sorted(set(case["expected_names"]) - set(nd["names"]))
        return len(missing) == 0, f"missing={missing}", us

    if m in ("workspace_symbol", "workspace_symbol_resolve"):
        res, us, _ = srv.request("workspace/symbol", {"query": case["query"]}, timeout=timeout)
        nw = normalize_workspace_symbol_ext(res, str(repo_dir))
        ok = case["expected_name"] in set(nw["names"])
        if m == "workspace_symbol_resolve" and isinstance(res, list) and res:
            r2, us2, err2 = srv.request("workspaceSymbol/resolve", res[0], timeout=timeout)
            return ok and isinstance(r2, dict) and err2 is None, f"resolved={isinstance(r2, dict)}", us + us2
        return ok, f"names={nw['names']}", us

    if m == "folding_range":
        res, us, _ = srv.request("textDocument/foldingRange", {"textDocument": td}, timeout=timeout)
        nf = normalize_folding_range(res)
        actual = {(s - 1, e - 1) for s, e in nf["spans"]}
        missing = [f for f in case["expected_folds"] if f not in actual]
        return len(missing) == 0, f"missing_folds={missing}", us

    if m == "selection_range":
        res, us, _ = srv.request("textDocument/selectionRange", {"textDocument": td, "positions": [pos]}, timeout=timeout)
        chains = normalize_selection_range(res)
        chain = chains[0] if chains else []
        exp = case["expected_inner"]
        ok = len(chain) >= 2 and chain[0] == (exp[1] + 1, exp[2], exp[3] + 1, exp[4])
        return ok, f"chain_head={chain[:2]}", us

    if m in ("document_link", "document_link_resolve"):
        res, us, _ = srv.request("textDocument/documentLink", {"textDocument": td}, timeout=timeout)
        urls = {str(it.get("target") or "") for it in (res or []) if isinstance(it, dict)}
        ok = case["expected_url"] in urls
        if m == "document_link_resolve" and isinstance(res, list) and res:
            r2, us2, err2 = srv.request("documentLink/resolve", res[0], timeout=timeout)
            return ok and isinstance(r2, dict) and err2 is None, f"resolved={isinstance(r2, dict)}", us + us2
        return ok, f"urls={sorted(urls)}", us

    if m in ("semantic_tokens_full", "semantic_tokens_delta", "semantic_tokens_range"):
        if m == "semantic_tokens_full":
            res, us, _ = srv.request("textDocument/semanticTokens/full", {"textDocument": td}, timeout=timeout)
        elif m == "semantic_tokens_delta":
            res, us, _ = srv.request(
                "textDocument/semanticTokens/full/delta",
                {"textDocument": td, "previousResultId": "0"},
                timeout=timeout,
            )
        else:
            full_rng = {"start": {"line": 0, "character": 0}, "end": {"line": 100, "character": 0}}
            res, us, _ = srv.request("textDocument/semanticTokens/range", {"textDocument": td, "range": full_rng}, timeout=timeout)
        nt = normalize_semantic_tokens_any(res)
        coords = {(ln - 1, col, length) for ln, col, length in nt["spans"]}
        return case["expected_token"] in coords, f"token_count={len(nt['tokens'])}", us

    if m in ("completion", "completion_resolve"):
        res, us, _ = srv.request("textDocument/completion", {"textDocument": td, "position": pos}, timeout=timeout)
        items = res.get("items", []) if isinstance(res, dict) else (res if isinstance(res, list) else [])
        labels = {str(it.get("label") or "").split("(")[0].strip() for it in items if isinstance(it, dict)}
        ok = case["expected_name"] in labels
        if m == "completion_resolve" and items:
            r2, us2, err2 = srv.request("completionItem/resolve", items[0], timeout=timeout)
            return ok and isinstance(r2, dict) and err2 is None, f"resolved={isinstance(r2, dict)}", us + us2
        return ok, f"found={ok}, total_items={len(items)}", us

    if m in ("code_action", "code_action_resolve"):
        params = {"textDocument": td, "range": rng_span, "context": {"diagnostics": []}}
        res, us, _ = srv.request("textDocument/codeAction", params, timeout=timeout)
        ok = isinstance(res, list) and len(res) > 0
        if m == "code_action_resolve" and ok:
            r2, us2, err2 = srv.request("codeAction/resolve", res[0], timeout=timeout)
            return isinstance(r2, dict) and err2 is None, f"resolved={isinstance(r2, dict)}", us + us2
        return ok, f"actions={len(res) if isinstance(res, list) else 0}", us

    if m in ("formatting", "range_formatting", "on_type_formatting"):
        opts = {"tabSize": 4, "insertSpaces": True}
        if m == "formatting":
            res, us, _ = srv.request("textDocument/formatting", {"textDocument": td, "options": opts}, timeout=timeout)
        elif m == "range_formatting":
            r = {"start": {"line": cur["line"], "character": 0}, "end": {"line": cur["line"] + 1, "character": 0}}
            res, us, _ = srv.request("textDocument/rangeFormatting", {"textDocument": td, "range": r, "options": opts}, timeout=timeout)
        else:
            p_brace = {"line": cur["line"], "character": cur["end_col"]}
            res, us, _ = srv.request(
                "textDocument/onTypeFormatting",
                {"textDocument": td, "position": p_brace, "ch": ";", "options": opts},
                timeout=timeout,
            )
        orig = (repo_dir / rel).read_text(encoding="utf-8")
        formatted = _apply_text_edits(orig, res if isinstance(res, list) else [])
        return case["expected_substr"] in formatted, f"edits={len(res) if isinstance(res, list) else 0}", us

    if m in ("document_diagnostic", "workspace_diagnostic"):
        rpc = "textDocument/diagnostic" if m == "document_diagnostic" else "workspace/diagnostic"
        params = {"textDocument": td} if m == "document_diagnostic" else {"previousResultIds": []}
        res, us, err = srv.request(rpc, params, timeout=timeout)
        ok = err is None and isinstance(res, dict) and isinstance(res.get("items"), list)
        return ok, f"ok={ok}", us

    if m in ("code_lens", "code_lens_resolve"):
        res, us, _ = srv.request("textDocument/codeLens", {"textDocument": td}, timeout=timeout)
        nl = normalize_code_lens(res)
        lines = sorted({ln - 1 for ln in nl["lines"]})
        if m == "code_lens_resolve":
            if not isinstance(res, list) or not res:
                return False, "empty codeLens", us
            r2, us2, err2 = srv.request("codeLens/resolve", res[0], timeout=timeout)
            ok = err2 is None and isinstance(r2, dict) and isinstance((r2.get("command") or {}).get("title"), str)
            return ok, f"resolved={ok}", us + us2
        exp = set(case["expected_lines"])
        return exp.issubset(set(lines)), f"expected_subset={sorted(exp)}, actual={lines}", us

    if m == "moniker":
        res, us, _ = srv.request("textDocument/moniker", {"textDocument": td, "position": pos}, timeout=timeout)
        monikers = normalize_moniker(res)
        ok = any(case["expected_substr"] in ident for _, ident, _ in monikers)
        return ok, f"monikers={monikers}", us

    return False, f"Unhandled method {m}", 0.0


def evaluate_synth_oracle(cmd: List[str], manifest: dict, is_ra: bool = False, timeout: float = 5.0) -> dict:
    """Runs all synthetic ground-truth cases against `cmd` and returns a structured report."""
    repo_dir = Path(manifest["workspace_dir"]).resolve()
    cases = build_oracle_cases(manifest)
    srv = SimpleLspProcess(cmd, str(repo_dir), is_ra=is_ra)
    per_method: Dict[str, dict] = {}
    results = []
    try:
        for rel in manifest["files"]:
            p = (repo_dir / rel).resolve()
            srv.notify("textDocument/didOpen", {
                "textDocument": {
                    "uri": p.as_uri(),
                    "languageId": "rust",
                    "version": 1,
                    "text": p.read_text(encoding="utf-8"),
                }
            })
        for case in cases:
            passed, detail, us = _run_single_case(srv, case, repo_dir, timeout)
            m = per_method.setdefault(case["method"], {"total": 0, "passed": 0, "failed": 0, "us": []})
            m["total"] += 1
            m["passed" if passed else "failed"] += 1
            m["us"].append(round(us, 1))
            results.append({
                "method": case["method"],
                "case_id": case["case_id"],
                "passed": passed,
                "detail": detail,
                "us": round(us, 1),
            })
    finally:
        srv.close()

    summary = {
        m: {
            "total": st["total"],
            "passed": st["passed"],
            "failed": st["failed"],
            "pass_pct": round(100.0 * st["passed"] / max(1, st["total"]), 1),
        }
        for m, st in per_method.items()
    }
    total_cases = len(results)
    total_passed = sum(1 for r in results if r["passed"])
    return {
        "seed": manifest["seed"],
        "scale": manifest["scale"],
        "total_cases": total_cases,
        "total_passed": total_passed,
        "overall_pass_pct": round(100.0 * total_passed / max(1, total_cases), 1),
        "per_method": summary,
        "cases": results,
    }
