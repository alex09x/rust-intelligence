# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Method-specific normalizers for `position_ab_parity.py`: reduces raw LSP responses from RIDX and
rust-analyzer into comparable canonical structures across 19 request methods."""

import re
from position_ab_content_rules import decode_tokens, scrub_path, token_data, uri_path

CODE_BLOCK_RE = re.compile(r"```(?:rust)?\n(.*?)```", re.DOTALL)
DECL_HEAD_RE = re.compile(
    r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:unsafe\s+|async\s+|const\s+|auto\s+)*"
    r"(fn|struct|enum|trait|type|mod|const|static|union|macro_rules!)\s+([A-Za-z_][A-Za-z0-9_]*)"
)
PAREN_SUFFIX_RE = re.compile(r"\s*\(.*\)$")
TOKEN_TYPES = (
    "comment", "keyword", "string", "number", "regexp", "operator", "namespace",
    "type", "struct", "class", "interface", "enum", "typeParameter", "function",
    "method", "decorator", "macro", "variable", "parameter", "property", "label",
)


def clean_path(uri, repo):
    """Scrubs a URI or file path to `ws:`, `registry:`, `git:`, `sysroot:`, or `external:`."""
    p = str(uri_path(uri) or "").replace("\\", "/")
    if p.startswith("/library/"):
        return "sysroot:" + p[1:]
    return scrub_path(p, repo)


def normalize_locations(res, repo):
    """For definition, declaration, type_definition, implementation: returns a dict with
    `targets` (sorted `[(path, line_1based), ...]`) and `full_ranges` (`[(path, start_line, end_line), ...]`)."""
    if not res:
        return {"targets": [], "full_ranges": []}
    items = res if isinstance(res, list) else [res]
    targets, full_ranges = set(), set()
    for it in items:
        if not isinstance(it, dict):
            continue
        uri = it.get("uri") or it.get("targetUri")
        if not uri:
            continue
        path = clean_path(uri, repo)
        sel = it.get("targetSelectionRange") or it.get("range") or it.get("targetRange") or {}
        full = it.get("targetRange") or it.get("range") or sel
        s_line = int((sel.get("start") or {}).get("line", 0)) + 1
        f_start = int((full.get("start") or {}).get("line", 0)) + 1
        f_end = int((full.get("end") or {}).get("line", s_line - 1)) + 1
        targets.add((path, s_line))
        full_ranges.add((path, f_start, f_end))
    return {"targets": sorted(targets), "full_ranges": sorted(full_ranges)}


def normalize_references(res, repo):
    """For references and document_highlight (where `uri` may be omitted): returns `spans`
    `[(path, start_line, start_col, end_line, end_col), ...]` and `ws_lines` `[(path, start_line), ...]`."""
    if not isinstance(res, list):
        return {"spans": [], "ws_lines": []}
    spans, ws_lines = set(), set()
    for it in res:
        if not isinstance(it, dict):
            continue
        uri = it.get("uri")
        path = clean_path(uri, repo) if uri else "doc:"
        rng = it.get("range") or {}
        s, e = rng.get("start") or {}, rng.get("end") or {}
        sl, sc = int(s.get("line", 0)) + 1, int(s.get("character", 0))
        el, ec = int(e.get("line", 0)) + 1, int(e.get("character", 0))
        spans.add((path, sl, sc, el, ec))
        if path == "doc:" or path.startswith("ws:"):
            ws_lines.add((path, sl))
    return {"spans": sorted(spans), "ws_lines": sorted(ws_lines)}


def normalize_hover(res):
    """Extracts fenced code blocks, declaration head `(kind, name)`, first signature line, and
    inferred `Type:` line from a Hover response."""
    if not isinstance(res, dict):
        return {"empty": True, "decl": None, "sig_line": "", "type_line": "", "text": ""}
    contents = res.get("contents")
    if isinstance(contents, dict):
        val = str(contents.get("value") or "")
    elif isinstance(contents, str):
        val = contents
    elif isinstance(contents, list):
        val = "\n".join(c.get("value", "") if isinstance(c, dict) else str(c) for c in contents)
    else:
        val = ""
    if not val.strip():
        return {"empty": True, "decl": None, "sig_line": "", "type_line": "", "text": ""}
    blocks = [b.strip() for b in CODE_BLOCK_RE.findall(val) if b.strip()]
    code_lines = [ln.strip() for b in blocks for ln in b.splitlines() if ln.strip()]
    decl, sig_line, type_line = None, "", ""
    for ln in code_lines:
        if ln.startswith("Type:"):
            type_line = ln[len("Type:"):].strip()
            continue
        m = DECL_HEAD_RE.match(ln)
        if m and decl is None:
            decl = (m.group(1), m.group(2))
            sig_line = re.sub(r"^pub(?:\([^)]*\))?\s+", "", ln)
    if not sig_line and code_lines:
        sig_line = code_lines[0]
    return {"empty": False, "decl": decl, "sig_line": sig_line, "type_line": type_line, "text": val}


def normalize_completion(res, top_n=20):
    """Normalizes a CompletionList or CompletionItem[] into stripped identifier labels and top-N."""
    items = res.get("items", []) if isinstance(res, dict) else (res if isinstance(res, list) else [])
    labels = []
    for it in items:
        if not isinstance(it, dict):
            continue
        raw = str(it.get("label") or "").strip()
        clean = PAREN_SUFFIX_RE.sub("", raw).rstrip("!")
        if clean:
            labels.append(clean)
    return {"labels": sorted(set(labels)), "top_n": labels[:top_n], "count": len(items)}


def normalize_signature_help(res):
    if not isinstance(res, dict) or not res.get("signatures"):
        return {"signatures": [], "active_sig": 0, "active_param": None}
    sigs = [str(s.get("label") or "").strip() for s in res.get("signatures", []) if isinstance(s, dict)]
    return {"signatures": sigs, "active_sig": int(res.get("activeSignature") or 0), "active_param": res.get("activeParameter")}


def normalize_prepare_rename(res):
    if not isinstance(res, dict):
        return None
    rng = res.get("range") or res
    s, e = rng.get("start") or {}, rng.get("end") or {}
    return {"range": (int(s.get("line", 0)) + 1, int(s.get("character", 0)), int(e.get("line", 0)) + 1, int(e.get("character", 0))),
            "placeholder": res.get("placeholder")}


def normalize_rename(res, repo):
    if not isinstance(res, dict):
        return []
    edits = set()
    for uri, ch_list in (res.get("changes") or {}).items():
        path = clean_path(uri, repo)
        for ed in ch_list or []:
            rng = ed.get("range") or {}
            s, e = rng.get("start") or {}, rng.get("end") or {}
            edits.add((path, int(s.get("line", 0)) + 1, int(s.get("character", 0)),
                       int(e.get("line", 0)) + 1, int(e.get("character", 0)), str(ed.get("newText") or "")))
    for doc_ed in res.get("documentChanges") or []:
        if not isinstance(doc_ed, dict) or "edits" not in doc_ed:
            continue
        uri = (doc_ed.get("textDocument") or {}).get("uri", "")
        path = clean_path(uri, repo)
        for ed in doc_ed.get("edits") or []:
            rng = ed.get("range") or {}
            s, e = rng.get("start") or {}, rng.get("end") or {}
            edits.add((path, int(s.get("line", 0)) + 1, int(s.get("character", 0)),
                       int(e.get("line", 0)) + 1, int(e.get("character", 0)), str(ed.get("newText") or "")))
    return sorted(edits)


def normalize_symbols(res):
    if not isinstance(res, list):
        return []
    out = set()

    def walk(items):
        for it in items:
            if not isinstance(it, dict):
                continue
            rng = it.get("selectionRange") or it.get("range") or (it.get("location") or {}).get("range") or {}
            line = int((rng.get("start") or {}).get("line", 0)) + 1
            out.add((str(it.get("name") or ""), int(it.get("kind") or 0), line))
            if isinstance(it.get("children"), list):
                walk(it["children"])

    walk(res)
    return sorted(out)


def normalize_answer(method, res, repo):
    """Dispatches `method` to its canonical normalizer."""
    if method in ("definition", "declaration", "type_definition", "implementation"):
        return normalize_locations(res, repo)
    if method in ("references", "document_highlight"):
        return normalize_references(res, repo)
    if method == "hover":
        return normalize_hover(res)
    if method == "completion":
        return normalize_completion(res)
    if method == "signature_help":
        return normalize_signature_help(res)
    if method == "prepare_rename":
        return normalize_prepare_rename(res)
    if method == "rename":
        return normalize_rename(res, repo)
    if method in ("document_symbol", "workspace_symbol"):
        return normalize_symbols(res)
    if method == "semantic_tokens_full":
        data = token_data(res)
        return [(t[0] + 1, t[1], t[2], TOKEN_TYPES[t[3]] if 0 <= t[3] < len(TOKEN_TYPES) else str(t[3]))
                for t in (decode_tokens(data) if data else [])]
    return res
