# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Extended method-specific normalizers for `position_ab_parity.py`: covers hierarchies,
presentation, formatting, diagnostics, code lens, code actions, inlay hints, and resolve methods."""

import re
from position_ab_content_rules import decode_tokens, scrub_path, token_data, uri_path

RIDX_DEFAULT_LEGEND = ("namespace", "type", "class", "enum", "interface", "struct", "typeParameter", "parameter", "variable", "property", "enumMember", "function", "method", "macro", "keyword", "comment", "string", "number")
MOD_PREFIX_RE = re.compile(r"\b(?:[a-z_][a-z0-9_]*::)+")
SEMANTIC_CANONICAL_TYPE = {
    "namespace": "namespace", "type": "type", "typeAlias": "type", "selfTypeKeyword": "type", "struct": "struct", "class": "struct", "union": "struct", "enum": "enum",
    "interface": "interface", "typeParameter": "typeParameter", "parameter": "variable", "variable": "variable", "const": "variable", "static": "variable",
    "constParameter": "variable", "selfKeyword": "variable", "property": "property", "enumMember": "enumMember", "function": "callable", "method": "callable",
    "macro": "macro", "procMacro": "macro", "derive": "macro", "deriveHelper": "macro",
}
RA_LEXICAL_ONLY_TYPES = frozenset({
    "comment", "keyword", "string", "number", "operator", "punctuation", "angle", "arithmetic", "attributeBracket", "attribute", "bitwise", "boolean", "brace",
    "bracket", "builtinAttribute", "builtinType", "character", "colon", "comma", "comparison", "decorator", "derive", "deriveHelper", "dot", "escapeSequence",
    "formatSpecifier", "generic", "invalidEscapeSequence", "label", "lifetime", "logical", "macroBang", "negation", "parenthesis", "semicolon", "toolModule",
    "unresolvedReference", "selfKeyword", "selfTypeKeyword",
})


def clean_uri(uri, repo):
    p = str(uri_path(uri) or "").replace("\\", "/")
    return ("sysroot:" + p[1:]) if p.startswith("/library/") else (scrub_path(p, repo) if p else "")


def _range_tuple(rng):
    if not isinstance(rng, dict):
        return (1, 0, 1, 0)
    s, e = rng.get("start") or {}, rng.get("end") or {}
    return (int(s.get("line", 0)) + 1, int(s.get("character", 0)), int(e.get("line", 0)) + 1, int(e.get("character", 0)))


def normalize_signature_help_ext(res):
    if not isinstance(res, dict) or not res.get("signatures"):
        return {"signatures": [], "arity": 0, "active_sig": 0, "active_param": None}
    sigs, arities = [], []
    for s in res.get("signatures", []):
        if not isinstance(s, dict):
            continue
        lbl = str(s.get("label") or "").strip()
        sigs.append(lbl)
        params = s.get("parameters")
        if isinstance(params, list) and params:
            arities.append(len(params))
        elif "(" in lbl and ")" in lbl:
            inner = lbl[lbl.find("(") + 1:lbl.rfind(")")].strip()
            arities.append(0 if not inner else len([p for p in inner.split(",") if p.strip()]))
        else:
            arities.append(0)
    active_sig = int(res.get("activeSignature") or 0)
    arity = arities[active_sig] if 0 <= active_sig < len(arities) else (arities[0] if arities else 0)
    return {"signatures": sigs, "arity": arity, "active_sig": active_sig, "active_param": int(res.get("activeParameter") or 0)}


def normalize_inlay_hint(res):
    if not isinstance(res, list):
        return {"hints": [], "line_labels": [], "type_line_labels": []}
    hints, line_labels, type_line_labels = set(), set(), set()
    for it in res:
        if not isinstance(it, dict):
            continue
        pos = it.get("position") or {}
        ln = int(pos.get("line", 0)) + 1
        col = int(pos.get("character", 0))
        raw_lbl = it.get("label")
        if isinstance(raw_lbl, list):
            lbl = "".join(str(p.get("value") or "") for p in raw_lbl if isinstance(p, dict)).strip()
        else:
            lbl = str(raw_lbl or "").strip()
        kind = int(it.get("kind") or 0)
        short_lbl = MOD_PREFIX_RE.sub("", lbl)
        hints.add((ln, col, lbl, kind))
        line_labels.add((ln, short_lbl))
        if kind == 1 or lbl.startswith(":"):
            type_line_labels.add((ln, short_lbl))
    return {"hints": sorted(hints), "line_labels": sorted(line_labels), "type_line_labels": sorted(type_line_labels)}


def normalize_document_symbol_ext(res):
    if not isinstance(res, list):
        return {"all": [], "top": [], "names": []}
    all_items, top_items, names = set(), set(), set()

    def walk(items, depth=0):
        for it in items:
            if not isinstance(it, dict):
                continue
            rng = it.get("selectionRange") or it.get("range") or (it.get("location") or {}).get("range") or {}
            line = int((rng.get("start") or {}).get("line", 0)) + 1
            name = str(it.get("name") or "")
            kind = int(it.get("kind") or 0)
            all_items.add((name, kind, line))
            if name:
                names.add(name)
            if depth == 0 and kind not in (8, 22):
                top_items.add((name, kind, line))
            if isinstance(it.get("children"), list):
                walk(it["children"], depth + 1)

    walk(res)
    return {"all": sorted(all_items), "top": sorted(top_items), "names": sorted(names)}


def normalize_workspace_symbol_ext(res, repo):
    if not isinstance(res, list):
        return {"items": [], "names": []}
    items, names = set(), set()
    for it in res:
        if not isinstance(it, dict):
            continue
        loc = it.get("location") or {}
        uri = loc.get("uri") or it.get("uri") or ""
        path = clean_uri(uri, repo)
        rng = loc.get("range") or it.get("range") or it.get("selectionRange") or {}
        line = int((rng.get("start") or {}).get("line", 0)) + 1
        name = str(it.get("name") or "")
        kind = int(it.get("kind") or 0)
        items.add((path, name, kind, line))
        if name:
            names.add(name)
    return {"items": sorted(items), "names": sorted(names)}


def normalize_folding_range(res):
    if not isinstance(res, list):
        return {"all": [], "regions": [], "spans": []}
    all_r, regions, spans = set(), set(), set()
    for it in res:
        if not isinstance(it, dict):
            continue
        sl = int(it.get("startLine", 0)) + 1
        el = int(it.get("endLine", 0)) + 1
        kind = str(it.get("kind") or "region")
        all_r.add((sl, el, kind))
        spans.add((sl, el))
        if kind not in ("imports", "comment"):
            regions.add((sl, el))
    return {"all": sorted(all_r), "regions": sorted(regions), "spans": sorted(spans)}


def normalize_selection_range(res):
    if not isinstance(res, list):
        return []
    chains = []
    for it in res:
        chain = []
        cur = it
        seen = 0
        while isinstance(cur, dict) and seen < 128:
            rt = _range_tuple(cur.get("range"))
            if (rt[0] != rt[2] or rt[1] != rt[3]) and (not chain or chain[-1] != rt):
                chain.append(rt)
            cur = cur.get("parent")
            seen += 1
        chains.append(chain)
    return chains


def normalize_document_link(res, repo):
    if not isinstance(res, list):
        return []
    out = set()
    for it in res:
        if not isinstance(it, dict):
            continue
        rt = _range_tuple(it.get("range"))
        tgt = clean_uri(it.get("target") or "", repo)
        out.add((*rt, tgt))
    return sorted(out)


def normalize_semantic_tokens_any(res, legend=None):
    if not isinstance(res, dict):
        return {"tokens": [], "spans": [], "sym_map": {}, "all_map": {}, "lex_count": 0, "edits": [], "is_delta": False}
    leg = tuple(legend) if legend else RIDX_DEFAULT_LEGEND
    data = token_data(res) or []
    is_delta = False
    if not data and isinstance(res.get("edits"), list):
        is_delta = True
        raw_edits = [e for e in res["edits"] if isinstance(e, dict)]
        if len(raw_edits) == 1 and int(raw_edits[0].get("start", 0)) == 0 and int(raw_edits[0].get("deleteCount", 0)) == 0:
            data = raw_edits[0].get("data") or []
        else:
            edits = [(int(e.get("start", 0)), int(e.get("deleteCount", 0)), len(e.get("data") or [])) for e in raw_edits]
            return {"tokens": [], "spans": [], "sym_map": {}, "all_map": {}, "lex_count": 0, "edits": edits, "is_delta": True}
    decoded = decode_tokens(data) if data else []
    tokens, sym_map, all_map, lex_count = [], {}, {}, 0
    for t in decoded:
        sp = (t[0] + 1, t[1], t[2])
        raw_tp = leg[t[3]] if 0 <= t[3] < len(leg) else str(t[3])
        canon_tp = SEMANTIC_CANONICAL_TYPE.get(raw_tp, raw_tp)
        tokens.append((*sp, canon_tp))
        all_map[sp] = canon_tp
        if raw_tp in RA_LEXICAL_ONLY_TYPES:
            lex_count += 1
        else:
            sym_map[sp] = canon_tp
    return {"tokens": tokens, "spans": sorted(all_map), "sym_map": sym_map, "all_map": all_map, "lex_count": lex_count, "edits": [], "is_delta": is_delta}


def normalize_hierarchy_items(res, repo):
    if not isinstance(res, list):
        return {"items": [], "names": [], "full_ranges": []}
    items, names, full_ranges = set(), set(), set()
    for it in res:
        if not isinstance(it, dict):
            continue
        path = clean_uri(it.get("uri") or "", repo)
        name = str(it.get("name") or "")
        kind = int(it.get("kind") or 0)
        sel = _range_tuple(it.get("selectionRange") or it.get("range"))
        full = _range_tuple(it.get("range") or it.get("selectionRange"))
        items.add((path, name, kind, sel[0]))
        names.add((path, name))
        full_ranges.add((path, name, full[0], full[2]))
    return {"items": sorted(items), "names": sorted(names), "full_ranges": sorted(full_ranges)}


def normalize_incoming_calls(res, repo):
    if not isinstance(res, list):
        return {"callers": [], "caller_names": [], "call_sites": []}
    callers, caller_names, call_sites = set(), set(), set()
    for it in res:
        if not isinstance(it, dict):
            continue
        frm = it.get("from") or {}
        path = clean_uri(frm.get("uri") or "", repo)
        name = str(frm.get("name") or "")
        sel = _range_tuple(frm.get("selectionRange") or frm.get("range"))
        callers.add((path, name, sel[0]))
        caller_names.add((path, name))
        for r in it.get("fromRanges") or []:
            rt = _range_tuple(r)
            call_sites.add((path, rt[0]))
    return {"callers": sorted(callers), "caller_names": sorted(caller_names), "call_sites": sorted(call_sites)}


def normalize_outgoing_calls(res, repo):
    if not isinstance(res, list):
        return {"callees": [], "callee_names": [], "call_lines": []}
    callees, callee_names, call_lines = set(), set(), set()
    for it in res:
        if not isinstance(it, dict):
            continue
        to = it.get("to") or {}
        path = clean_uri(to.get("uri") or "", repo)
        name = str(to.get("name") or "")
        sel = _range_tuple(to.get("selectionRange") or to.get("range"))
        callees.add((path, name, sel[0]))
        if name:
            callee_names.add(name)
        for r in it.get("fromRanges") or []:
            rt = _range_tuple(r)
            call_lines.add(rt[0])
    return {"callees": sorted(callees), "callee_names": sorted(callee_names), "call_lines": sorted(call_lines)}


def normalize_code_action(res):
    if not isinstance(res, list):
        return {"kinds": [], "titles": []}
    kinds = sorted({str(it.get("kind") or "") for it in res if isinstance(it, dict) and it.get("kind")})
    titles = sorted({str(it.get("title") or "") for it in res if isinstance(it, dict) and it.get("title")})
    return {"kinds": kinds, "titles": titles}


def normalize_formatting(res):
    if not isinstance(res, list):
        return []
    edits = set()
    for it in res:
        if not isinstance(it, dict):
            continue
        rt = _range_tuple(it.get("range"))
        edits.add((*rt, str(it.get("newText") or "")))
    return sorted(edits)


def normalize_diagnostics(res, repo):
    if not isinstance(res, dict):
        return {"errors_warnings": [], "hints_info": []}
    raw_items = []
    if isinstance(res.get("items"), list):
        for it in res["items"]:
            if isinstance(it, dict) and "items" in it:
                p = clean_uri(it.get("uri") or "", repo)
                for sub in it.get("items") or []:
                    if isinstance(sub, dict):
                        raw_items.append((p, sub))
            elif isinstance(it, dict):
                raw_items.append(("doc:", it))
    ew, hi = set(), set()
    for path, d in raw_items:
        rt = _range_tuple(d.get("range"))
        sev = int(d.get("severity") or 1)
        code = str(d.get("code") or "")
        msg = re.sub(r"\s+", " ", str(d.get("message") or "").strip())[:80]
        entry = (path, rt[0], sev, code, msg)
        if sev in (1, 2) and code != "inactive-code":
            ew.add(entry)
        else:
            hi.add(entry)
    return {"errors_warnings": sorted(ew), "hints_info": sorted(hi)}


def normalize_code_lens(res):
    if not isinstance(res, list):
        return {"lines": [], "spans": []}
    lines, spans = set(), set()
    for it in res:
        if not isinstance(it, dict):
            continue
        rt = _range_tuple(it.get("range"))
        lines.add(rt[0])
        spans.add(rt)
    return {"lines": sorted(lines), "spans": sorted(spans)}


def normalize_linked_editing_range(res):
    if not isinstance(res, dict) or not isinstance(res.get("ranges"), list):
        return []
    return sorted({_range_tuple(r) for r in res["ranges"] if isinstance(r, dict)})


def normalize_moniker(res):
    if not isinstance(res, list):
        return []
    return sorted({
        (str(it.get("scheme") or ""), str(it.get("identifier") or ""), str(it.get("uniqueness") or ""))
        for it in res if isinstance(it, dict)
    })


def normalize_resolve_item(res):
    if not isinstance(res, dict):
        return {"key_id": None, "title": "", "range": None, "raw": res}
    cmd = res.get("command") if isinstance(res.get("command"), dict) else {}
    title = str(cmd.get("title") or res.get("title") or "").strip()
    ident = str(res.get("label") or title or res.get("name") or "")
    kind = int(res.get("kind") or 0)
    rng = _range_tuple(res["range"]) if isinstance(res.get("range"), dict) else None
    return {"key_id": (ident, kind, rng), "title": title, "range": rng, "raw": res}


def normalize_answer_ext(method, res, repo, legend=None):
    if method == "signature_help":
        return normalize_signature_help_ext(res)
    if method == "inlay_hint":
        return normalize_inlay_hint(res)
    if method == "document_symbol":
        return normalize_document_symbol_ext(res)
    if method == "workspace_symbol":
        return normalize_workspace_symbol_ext(res, repo)
    if method == "folding_range":
        return normalize_folding_range(res)
    if method == "selection_range":
        return normalize_selection_range(res)
    if method == "document_link":
        return normalize_document_link(res, repo)
    if method in ("semantic_tokens_full", "semantic_tokens_delta", "semantic_tokens_range"):
        return normalize_semantic_tokens_any(res, legend=legend)
    if method in ("prepare_call_hierarchy", "prepare_type_hierarchy", "supertypes", "subtypes"):
        return normalize_hierarchy_items(res, repo)
    if method == "incoming_calls":
        return normalize_incoming_calls(res, repo)
    if method == "outgoing_calls":
        return normalize_outgoing_calls(res, repo)
    if method == "code_action":
        return normalize_code_action(res)
    if method in ("formatting", "range_formatting", "on_type_formatting"):
        return normalize_formatting(res)
    if method in ("document_diagnostic", "workspace_diagnostic"):
        return normalize_diagnostics(res, repo)
    if method == "code_lens":
        return normalize_code_lens(res)
    if method == "linked_editing_range":
        return normalize_linked_editing_range(res)
    if method == "moniker":
        return normalize_moniker(res)
    if method.endswith("_resolve"):
        return normalize_resolve_item(res)
    return res
