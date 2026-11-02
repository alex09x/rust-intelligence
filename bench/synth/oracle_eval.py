# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Executes synthetic ground-truth cases against an LSP server and evaluates exact oracle parity."""

from pathlib import Path
from typing import Dict, List, Set, Tuple
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from lsp_process import SimpleLspProcess  # noqa: E402
from position_ab_content_rules import uri_path  # noqa: E402
from position_ab_parity_norm import normalize_hover  # noqa: E402
from position_ab_parity_norm_ext import (  # noqa: E402
    normalize_code_lens, normalize_document_symbol_ext, normalize_folding_range,
    normalize_inlay_hint, normalize_linked_editing_range, normalize_moniker,
    normalize_selection_range, normalize_semantic_tokens_any,
    normalize_signature_help_ext, normalize_workspace_symbol_ext,
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


def _set_pr(actual: Set, expected: Set) -> Tuple[bool, float, float]:
    if not expected and not actual:
        return True, 1.0, 1.0
    tp = len(actual & expected)
    return (actual == expected), (tp / len(actual) if actual else 0.0), (tp / len(expected) if expected else 1.0)


def _extract_loc_lines(res, repo_dir: Path) -> List[Tuple[str, int]]:
    items = (res if isinstance(res, list) else [res]) if res else []
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
    return sorted({_rng_tuple(_rel_path(it["uri"], repo_dir) if it.get("uri") else default_rel, it.get("range") or {}) for it in res if isinstance(it, dict)})


def _extract_rename_details(res, repo_dir: Path) -> Tuple[Dict[Tuple[str, int, int, int, int], str], List[Tuple[str, str]], Dict[str, list]]:
    if not isinstance(res, dict):
        return {}, [], {}
    edit_map: Dict[Tuple[str, int, int, int, int], str] = {}
    file_renames: List[Tuple[str, str]] = []
    per_file_edits: Dict[str, list] = {}
    for uri, edits in (res.get("changes") or {}).items():
        rel = _rel_path(uri, repo_dir)
        per_file_edits.setdefault(rel, []).extend(edits or [])
        for ed in edits or []:
            edit_map[_rng_tuple(rel, ed.get("range") or {})] = str(ed.get("newText") or "")
    for doc_ed in res.get("documentChanges") or []:
        if not isinstance(doc_ed, dict):
            continue
        if doc_ed.get("kind") == "rename":
            file_renames.append((_rel_path(doc_ed.get("oldUri", ""), repo_dir), _rel_path(doc_ed.get("newUri", ""), repo_dir)))
        elif "textDocument" in doc_ed:
            rel = _rel_path((doc_ed["textDocument"] or {}).get("uri", ""), repo_dir)
            if rel not in per_file_edits:
                per_file_edits[rel] = list(doc_ed.get("edits") or [])
            for ed in doc_ed.get("edits") or []:
                edit_map[_rng_tuple(rel, ed.get("range") or {})] = str(ed.get("newText") or "")
    return edit_map, file_renames, per_file_edits


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

    parsed = {(to_off(ed["range"].get("start") or {}), to_off(ed["range"].get("end") or {})): str(ed.get("newText") or "") for ed in edits if isinstance(ed, dict) and isinstance(ed.get("range"), dict)}
    out = text
    for (start, end), repl in sorted(parsed.items(), key=lambda x: x[0], reverse=True):
        out = out[:start] + repl + out[end:]
    return out


def _verify_rename_compiles(repo_dir: Path, per_file_edits: Dict[str, list], file_renames: List[Tuple[str, str]]) -> Tuple[bool, str]:
    backups: Dict[Path, str] = {}
    moved: List[Tuple[Path, Path]] = []
    try:
        for rel, edits in per_file_edits.items():
            fp = (repo_dir / rel).resolve()
            if fp.is_file():
                orig = fp.read_text(encoding="utf-8")
                backups[fp] = orig
                fp.write_text(_apply_text_edits(orig, edits), encoding="utf-8")
        for old_r, new_r in file_renames:
            old_p, new_p = (repo_dir / old_r).resolve(), (repo_dir / new_r).resolve()
            if old_p.is_file():
                old_p.rename(new_p)
                moved.append((old_p, new_p))
        chk = subprocess.run(["cargo", "check", "--workspace", "--tests", "--quiet"], cwd=str(repo_dir), capture_output=True, text=True, timeout=60)
        return chk.returncode == 0, (chk.stderr.strip().splitlines()[0] if chk.stderr.strip() else "ok")
    finally:
        for old_p, new_p in reversed(moved):
            if new_p.exists():
                new_p.rename(old_p)
        for fp, orig in backups.items():
            fp.write_text(orig, encoding="utf-8")


def _run_single_case(srv: SimpleLspProcess, case: dict, repo_dir: Path, timeout: float) -> Tuple[bool, str, float, float, float]:
    m = case["method"]
    cur = case["cursor"]
    rel = cur["rel_path"]
    uri = (repo_dir / rel).resolve().as_uri()
    pos = {"line": cur["line"], "character": cur["col"]}
    td = {"uri": uri}
    rng_span = {"start": {"line": cur["line"], "character": cur["col"]}, "end": {"line": cur["end_line"], "character": cur["end_col"]}}

    if m in ("definition", "declaration", "type_definition", "implementation"):
        rpc = {"definition": "textDocument/definition", "declaration": "textDocument/declaration", "type_definition": "textDocument/typeDefinition", "implementation": "textDocument/implementation"}[m]
        res, us, _ = srv.request(rpc, {"textDocument": td, "position": pos}, timeout=timeout)
        actual = set(_extract_loc_lines(res, repo_dir))
        expected = {(s[0], s[1]) for s in case["expected_spans"]}
        ok, prec, rec = _set_pr(actual, expected)
        return ok, f"expected={sorted(expected)}, actual={sorted(actual)}", us, prec, rec

    if m == "references":
        res, us, _ = srv.request("textDocument/references", {"textDocument": td, "position": pos, "context": {"includeDeclaration": True}}, timeout=timeout)
        actual = set(_extract_exact_spans(res, repo_dir))
        expected = set(case["expected_spans"])
        ok, prec, rec = _set_pr(actual, expected)
        return ok, f"expected={len(expected)} spans, actual={len(actual)} spans", us, prec, rec

    if m == "document_highlight":
        res, us, _ = srv.request("textDocument/documentHighlight", {"textDocument": td, "position": pos}, timeout=timeout)
        actual = set(_extract_exact_spans(res, repo_dir, default_rel=rel))
        expected = set(case["expected_spans"])
        ok, prec, rec = _set_pr(actual, expected)
        return ok, f"expected={len(expected)}, actual={len(actual)}", us, prec, rec

    if m == "rename":
        res, us, err = srv.request("textDocument/rename", {"textDocument": td, "position": pos, "newName": case["new_name"]}, timeout=timeout)
        if case.get("expect_error"):
            edit_map, _, _ = _extract_rename_details(res, repo_dir)
            refused = (err is not None) or (res is None) or (len(edit_map) == 0)
            return refused, f"refused={refused} (err={err is not None})", us, float(refused), float(refused)
        if err is not None:
            return False, f"unexpected error: {err}", us, 0.0, 0.0
        edit_map, file_renames, per_file_edits = _extract_rename_details(res, repo_dir)
        actual_spans, expected_spans = set(edit_map.keys()), set(case["expected_spans"])
        spans_ok, prec, rec = _set_pr(actual_spans, expected_spans)
        text_ok = (edit_map == case["expected_text_map"]) if "expected_text_map" in case else (set(edit_map.values()) == {case["new_name"]})
        fren_ok = (case["expected_file_rename"] in file_renames) if "expected_file_rename" in case else True
        comp_ok, comp_msg = _verify_rename_compiles(repo_dir, per_file_edits, file_renames) if edit_map else (False, "no edits")
        ok = spans_ok and text_ok and fren_ok and comp_ok
        return ok, f"spans={len(actual_spans)}/{len(expected_spans)}, text_ok={text_ok}, fren_ok={fren_ok}, cargo_check={comp_ok} ({comp_msg})", us, prec, rec

    if m == "prepare_rename":
        res, us, _ = srv.request("textDocument/prepareRename", {"textDocument": td, "position": pos}, timeout=timeout)
        if not isinstance(res, dict):
            return False, f"non-dict response: {res!r}", us, 0.0, 0.0
        actual = {_rng_tuple(rel, res.get("range") or res)}
        ok, prec, rec = _set_pr(actual, set(case["expected_spans"]))
        return ok, f"actual={sorted(actual)}", us, prec, rec

    if m == "linked_editing_range":
        res, us, _ = srv.request("textDocument/linkedEditingRange", {"textDocument": td, "position": pos}, timeout=timeout)
        actual = {(rel, sl - 1, sc, el - 1, ec) for sl, sc, el, ec in normalize_linked_editing_range(res)}
        ok, prec, rec = _set_pr(actual, set(case["expected_spans"]))
        return ok, f"actual={len(actual)}", us, prec, rec

    if m in ("prepare_call_hierarchy", "incoming_calls", "outgoing_calls"):
        items, us1, _ = srv.request("textDocument/prepareCallHierarchy", {"textDocument": td, "position": pos}, timeout=timeout)
        if not isinstance(items, list) or not items:
            return False, "empty prepareCallHierarchy", us1, 0.0, 0.0
        if m == "prepare_call_hierarchy":
            ok = items[0].get("name") == case["expected_name"]
            return ok, f"got={items[0].get('name')!r}", us1, float(ok), float(ok)
        rpc, key = ("callHierarchy/incomingCalls", "from") if m == "incoming_calls" else ("callHierarchy/outgoingCalls", "to")
        res, us2, _ = srv.request(rpc, {"item": items[0]}, timeout=timeout)
        names = {str((it.get(key) or {}).get("name") or "") for it in (res or []) if isinstance(it, dict)}
        ok, prec, rec = _set_pr(names, set(case["expected_names"]))
        return ok, f"expected={case['expected_names']}, actual={sorted(names)}", us1 + us2, prec, rec

    if m in ("prepare_type_hierarchy", "supertypes", "subtypes"):
        items, us1, _ = srv.request("textDocument/prepareTypeHierarchy", {"textDocument": td, "position": pos}, timeout=timeout)
        if not isinstance(items, list) or not items:
            return False, "empty prepareTypeHierarchy", us1, 0.0, 0.0
        if m == "prepare_type_hierarchy":
            ok = items[0].get("name") == case["expected_name"]
            return ok, f"got={items[0].get('name')!r}", us1, float(ok), float(ok)
        rpc = "typeHierarchy/supertypes" if m == "supertypes" else "typeHierarchy/subtypes"
        res, us2, _ = srv.request(rpc, {"item": items[0]}, timeout=timeout)
        names = {str(it.get("name") or "") for it in (res or []) if isinstance(it, dict)}
        ok, prec, rec = _set_pr(names, set(case["expected_names"]))
        return ok, f"expected={case['expected_names']}, actual={sorted(names)}", us1 + us2, prec, rec

    if m == "hover":
        res, us, _ = srv.request("textDocument/hover", {"textDocument": td, "position": pos}, timeout=timeout)
        nh = normalize_hover(res)
        ok = case["expected_substr"] in nh["text"]
        return ok, f"sig={nh['sig_line']!r}", us, float(ok), float(ok)

    if m == "signature_help":
        res, us, _ = srv.request("textDocument/signatureHelp", {"textDocument": td, "position": pos}, timeout=timeout)
        ns = normalize_signature_help_ext(res)
        lbl = ns["signatures"][0] if ns["signatures"] else ""
        ok = bool(ns["signatures"]) and (case["expected_substr"] in lbl) and (ns["active_param"] == case["expected_active_param"])
        return ok, f"label={lbl!r}, active_param={ns['active_param']}", us, float(ok), float(ok)

    if m in ("inlay_hint", "inlay_hint_resolve"):
        res, us, _ = srv.request("textDocument/inlayHint", {"textDocument": td, "range": {"start": {"line": 0, "character": 0}, "end": {"line": 400, "character": 0}}}, timeout=timeout)
        nh = normalize_inlay_hint(res)
        ok = any(case["expected_substr"] in lbl for _, _, lbl, _ in nh["hints"])
        if m == "inlay_hint_resolve" and isinstance(res, list) and res:
            r2, us2, err2 = srv.request("inlayHint/resolve", res[0], timeout=timeout)
            ok = ok and isinstance(r2, dict) and err2 is None
            return ok, f"resolved={ok}", us + us2, float(ok), float(ok)
        return ok, f"hints={len(nh['hints'])}", us, float(ok), float(ok)

    if m == "document_symbol":
        res, us, _ = srv.request("textDocument/documentSymbol", {"textDocument": td}, timeout=timeout)
        actual = {n for n, k, _ in normalize_document_symbol_ext(res)["all"] if k not in (8, 13, 22) and not n.startswith("impl") and not n.startswith("apply_op_")}
        expected = set(case["expected_names"])
        ok, prec, rec = _set_pr(actual, expected)
        return rec == 1.0 and prec >= 0.9, f"prec={prec:.2f}, rec={rec:.2f}", us, prec, rec

    if m in ("workspace_symbol", "workspace_symbol_resolve"):
        res, us, _ = srv.request("workspace/symbol", {"query": case["query"]}, timeout=timeout)
        nw = normalize_workspace_symbol_ext(res, str(repo_dir))
        ok = case["expected_name"] in set(nw["names"])
        if m == "workspace_symbol_resolve" and isinstance(res, list) and res:
            r2, us2, err2 = srv.request("workspaceSymbol/resolve", res[0], timeout=timeout)
            ok = ok and isinstance(r2, dict) and err2 is None
            return ok, f"resolved={ok}", us + us2, float(ok), float(ok)
        return ok, f"names={nw['names']}", us, float(ok), float(ok)

    if m == "folding_range":
        res, us, _ = srv.request("textDocument/foldingRange", {"textDocument": td}, timeout=timeout)
        actual = {(s - 1, e - 1) for s, e in normalize_folding_range(res)["spans"]}
        expected = set(case["expected_folds"])
        _, prec, rec = _set_pr(actual, expected)
        return rec == 1.0, f"prec={prec:.2f}, rec={rec:.2f}", us, prec, rec

    if m == "selection_range":
        res, us, _ = srv.request("textDocument/selectionRange", {"textDocument": td, "positions": [pos]}, timeout=timeout)
        chains = normalize_selection_range(res)
        chain = chains[0] if chains else []
        exp = case["expected_inner"]
        ok = len(chain) >= 2 and chain[0] == (exp[1] + 1, exp[2], exp[3] + 1, exp[4])
        return ok, f"chain_head={chain[:2]}", us, float(ok), float(ok)

    if m in ("document_link", "document_link_resolve"):
        res, us, _ = srv.request("textDocument/documentLink", {"textDocument": td}, timeout=timeout)
        urls = {str(it.get("target") or "") for it in (res or []) if isinstance(it, dict)}
        ok = case["expected_url"] in urls
        if m == "document_link_resolve" and isinstance(res, list) and res:
            r2, us2, err2 = srv.request("documentLink/resolve", res[0], timeout=timeout)
            ok = ok and isinstance(r2, dict) and err2 is None
            return ok, f"resolved={ok}", us + us2, float(ok), float(ok)
        return ok, f"urls={len(urls)}", us, float(ok), float(ok)

    if m in ("semantic_tokens_full", "semantic_tokens_delta", "semantic_tokens_range"):
        if m == "semantic_tokens_full":
            res, us, _ = srv.request("textDocument/semanticTokens/full", {"textDocument": td}, timeout=timeout)
        elif m == "semantic_tokens_delta":
            res, us, _ = srv.request("textDocument/semanticTokens/full/delta", {"textDocument": td, "previousResultId": "0"}, timeout=timeout)
        else:
            res, us, _ = srv.request("textDocument/semanticTokens/range", {"textDocument": td, "range": {"start": {"line": 0, "character": 0}, "end": {"line": 250, "character": 0}}}, timeout=timeout)
        nt = normalize_semantic_tokens_any(res)
        coords = {(ln - 1, col, length) for ln, col, length in nt["spans"]}
        ok = case["expected_token"] in coords
        return ok, f"token_count={len(nt['tokens'])}", us, float(ok), float(ok)

    if m in ("completion", "completion_resolve"):
        res, us, _ = srv.request("textDocument/completion", {"textDocument": td, "position": pos}, timeout=timeout)
        items = res.get("items", []) if isinstance(res, dict) else (res if isinstance(res, list) else [])
        ordered = [str(it.get("label") or "").split("(")[0].strip() for it in items if isinstance(it, dict)]
        labels = set(ordered)
        if m == "completion_resolve":
            ok = (case["expected_name"] in labels) and bool(items)
            if ok:
                r2, us2, err2 = srv.request("completionItem/resolve", items[0], timeout=timeout)
                ok = isinstance(r2, dict) and err2 is None
                return ok, f"resolved={ok}", us + us2, float(ok), float(ok)
            return False, "empty completion", us, 0.0, 0.0
        exp_mem = set(case.get("expected_members") or [case["expected_name"]])
        forb_mem = set(case.get("forbidden_members") or [])
        rec = len(labels & exp_mem) / max(1, len(exp_mem))
        leaked = labels & forb_mem
        prec = 1.0 if not leaked else max(0.0, 1.0 - len(leaked) / max(1, len(forb_mem)))
        ok = (rec == 1.0) and (len(leaked) == 0)
        return ok, f"rec={rec:.2f}, leaked={sorted(leaked)}, total={len(items)}", us, prec, rec

    if m in ("code_action", "code_action_resolve"):
        res, us, _ = srv.request("textDocument/codeAction", {"textDocument": td, "range": rng_span, "context": {"diagnostics": []}}, timeout=timeout)
        ok = isinstance(res, list) and len(res) > 0
        if m == "code_action_resolve" and ok:
            r2, us2, err2 = srv.request("codeAction/resolve", res[0], timeout=timeout)
            ok = isinstance(r2, dict) and err2 is None
            return ok, f"resolved={ok}", us + us2, float(ok), float(ok)
        return ok, f"actions={len(res) if isinstance(res, list) else 0}", us, float(ok), float(ok)

    if m in ("formatting", "range_formatting", "on_type_formatting"):
        opts = {"tabSize": 4, "insertSpaces": True}
        if m == "formatting":
            res, us, _ = srv.request("textDocument/formatting", {"textDocument": td, "options": opts}, timeout=timeout)
        elif m == "range_formatting":
            r = {"start": {"line": cur["line"], "character": 0}, "end": {"line": cur["line"] + 1, "character": 0}}
            res, us, _ = srv.request("textDocument/rangeFormatting", {"textDocument": td, "range": r, "options": opts}, timeout=timeout)
        else:
            p_brace = {"line": cur["line"], "character": cur["end_col"]}
            res, us, _ = srv.request("textDocument/onTypeFormatting", {"textDocument": td, "position": p_brace, "ch": ";", "options": opts}, timeout=timeout)
        orig = (repo_dir / rel).read_text(encoding="utf-8")
        ok = case["expected_substr"] in _apply_text_edits(orig, res if isinstance(res, list) else [])
        return ok, f"ok={ok}", us, float(ok), float(ok)

    if m in ("document_diagnostic", "workspace_diagnostic"):
        rpc = "textDocument/diagnostic" if m == "document_diagnostic" else "workspace/diagnostic"
        params = {"textDocument": td} if m == "document_diagnostic" else {"previousResultIds": []}
        res, us, err = srv.request(rpc, params, timeout=timeout)
        ok = err is None and isinstance(res, dict) and isinstance(res.get("items"), list)
        return ok, f"ok={ok}", us, float(ok), float(ok)

    if m in ("code_lens", "code_lens_resolve"):
        res, us, _ = srv.request("textDocument/codeLens", {"textDocument": td}, timeout=timeout)
        lines = {ln - 1 for ln in normalize_code_lens(res)["lines"]}
        if m == "code_lens_resolve":
            if not isinstance(res, list) or not res:
                return False, "empty codeLens", us, 0.0, 0.0
            r2, us2, err2 = srv.request("codeLens/resolve", res[0], timeout=timeout)
            ok = err2 is None and isinstance(r2, dict) and isinstance((r2.get("command") or {}).get("title"), str)
            return ok, f"resolved={ok}", us + us2, float(ok), float(ok)
        exp = set(case["expected_lines"])
        ok = exp.issubset(lines)
        return ok, f"expected={sorted(exp)}, actual={sorted(lines)}", us, float(ok), float(ok)

    if m == "moniker":
        res, us, _ = srv.request("textDocument/moniker", {"textDocument": td, "position": pos}, timeout=timeout)
        monikers = normalize_moniker(res)
        ok = any(case["expected_substr"] in ident for _, ident, _ in monikers)
        return ok, f"monikers={len(monikers)}", us, float(ok), float(ok)

    return False, f"Unhandled method {m}", 0.0, 0.0, 0.0


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
            srv.notify("textDocument/didOpen", {"textDocument": {"uri": p.as_uri(), "languageId": "rust", "version": 1, "text": p.read_text(encoding="utf-8")}})
        for case in cases:
            passed, detail, us, prec, rec = _run_single_case(srv, case, repo_dir, timeout)
            m = per_method.setdefault(case["method"], {"total": 0, "passed": 0, "failed": 0, "prec": [], "rec": []})
            m["total"] += 1
            m["passed" if passed else "failed"] += 1
            m["prec"].append(prec)
            m["rec"].append(rec)
            results.append({"method": case["method"], "case_id": case["case_id"], "passed": passed, "detail": detail, "us": round(us, 1), "precision": round(prec, 4), "recall": round(rec, 4)})
    finally:
        srv.close()

    summary = {
        m: {"total": st["total"], "passed": st["passed"], "failed": st["failed"], "pass_pct": round(100.0 * st["passed"] / max(1, st["total"]), 1), "precision_pct": round(100.0 * sum(st["prec"]) / max(1, len(st["prec"])), 1), "recall_pct": round(100.0 * sum(st["rec"]) / max(1, len(st["rec"])), 1)}
        for m, st in per_method.items()
    }
    total_cases, total_passed = len(results), sum(1 for r in results if r["passed"])
    return {"seed": manifest["seed"], "scale": manifest["scale"], "topology": manifest.get("topology", {}), "total_cases": total_cases, "total_passed": total_passed, "overall_pass_pct": round(100.0 * total_passed / max(1, total_cases), 1), "per_method": summary, "cases": results}
