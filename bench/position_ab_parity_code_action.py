# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Normalization and parity rules for `textDocument/codeAction` and `codeAction/resolve`."""

import hashlib
import re
from pathlib import Path

_ITEM_DECL_RE = re.compile(
    r"(?:^|\n)\s*(?:pub(?:\s*\([^)]*\))?\s+)?"
    r"(?:default\s+)?(?:const\s+)?(?:async\s+)?(?:unsafe\s+)?(?:extern(?:\s+\"[^\"]*\")?\s+)?"
    r"(fn|type|struct|enum|trait|const|static|let)\s+(?:mut\s+)?(r#[A-Za-z_]\w*|[A-Za-z_]\w*)"
)
_IMPL_FOR_RE = re.compile(
    r"(?:^|\n)\s*(?:unsafe\s+)?impl(?:\s*<[^>]*>)?\s+([^\n{]+?)\s+for\s+([^\n{]+?)\s*(?:where\b|\{)"
)
_IMPL_INH_RE = re.compile(
    r"(?:^|\n)\s*(?:unsafe\s+)?impl(?:\s*<[^>]*>)?\s+([^\n{]+?)\s*(?:where\b|\{)"
)
_DOC_SEC_RE = re.compile(r"^\s*///\s*(#\s+[A-Za-z][A-Za-z0-9 _-]*)\s*$", re.MULTILINE)


def _is_ra_placeholder_stub(title: str) -> bool:
    return title.startswith("Generate trait impl for ") or title == "Generate blanket trait implementation"


def _leaf_ident(ty_src: str) -> str:
    base = ty_src.split("<", 1)[0].strip()
    leaf = base.rsplit("::", 1)[-1].strip()
    return leaf[2:] if leaf.startswith("r#") else leaf


def _collapse_ws(text: str) -> str:
    return re.sub(r"\s+", " ", text.strip())


def _extract_edits_by_uri(edit, clean_uri_fn, repo):
    if not isinstance(edit, dict):
        return {}, []
    by_uri = {}
    ops = []
    for dc in edit.get("documentChanges") or []:
        if not isinstance(dc, dict):
            continue
        if "textDocument" in dc and dc.get("edits"):
            raw_u = (dc.get("textDocument") or {}).get("uri") or ""
            if raw_u:
                cu = clean_uri_fn(raw_u, repo)
                by_uri.setdefault(cu, []).extend(dc.get("edits") or [])
        elif dc.get("kind") in ("create", "rename", "delete") and dc.get("uri"):
            cu = clean_uri_fn(dc["uri"], repo)
            ops.append(f"{dc['kind']}:{cu}")
            by_uri.setdefault(cu, [])
    for raw_u, es in (edit.get("changes") or {}).items():
        if not es:
            continue
        cu = clean_uri_fn(raw_u, repo)
        if cu not in by_uri:
            by_uri[cu] = list(es)
    norm_by_uri = {}
    for cu, es in sorted(by_uri.items()):
        seen = set()
        dedup = []
        for e in es:
            if not isinstance(e, dict):
                continue
            rng = e.get("range") or {}
            st = rng.get("start") or {}
            en = rng.get("end") or {}
            key = (
                int(st.get("line", 0)),
                int(st.get("character", 0)),
                int(en.get("line", 0)),
                int(en.get("character", 0)),
                str(e.get("newText") or ""),
            )
            if key not in seen:
                seen.add(key)
                dedup.append(key)
        dedup.sort(key=lambda x: x[:4])
        norm_by_uri[cu] = dedup
    return norm_by_uri, sorted(set(ops))


def _applied_text(repo, cu, dedup):
    """The workspace file `cu` (`ws:` + repo-relative path) after the edits `dedup` (`(sl, sc, el, ec, text)`, UTF-16 columns), or None
    when the file cannot be read or an edit falls outside it. Two answers that leave the same text are the same
    change, however their edits are cut (one replaced statement or an inserted line)."""
    if not str(cu).startswith("ws:"):
        return None
    try:
        src = (Path(repo) / str(cu)[3:]).read_text(encoding="utf-8")
    except (OSError, ValueError):
        return None
    lines = src.split("\n")
    starts = [0]
    for ln in lines[:-1]:
        starts.append(starts[-1] + len(ln) + 1)

    def off(line, ch):
        if line >= len(lines):
            return None
        units = lines[line].encode("utf-16-le")[: 2 * ch]
        return starts[line] + len(units.decode("utf-16-le", errors="ignore"))

    spans = []
    for sl, sc, el, ec, text in dedup:
        a, b = off(sl, sc), off(el, ec)
        if a is None or b is None or b < a:
            return None
        spans.append((a, b, text))
    spans.sort(key=lambda s: (s[0], s[1]))
    if any(spans[i + 1][0] < spans[i][1] for i in range(len(spans) - 1)):
        return None
    for a, b, text in reversed(spans):
        src = src[:a] + text + src[b:]
    return src


def _extract_item_headers(raw_texts):
    headers = set()
    joined = "\n".join(raw_texts)
    if "#[allow(" in joined or "#![allow(" in joined:
        headers.add("attr:allow")
    doc_lines = [ln.strip() for ln in joined.splitlines() if ln.lstrip().startswith("///")]
    if doc_lines:
        if any(ln.startswith("/// Documentation for ") for ln in doc_lines):
            headers.add("doc:stub")
        else:
            headers.add("doc:template")
        for m in _DOC_SEC_RE.finditer(joined):
            headers.add(f"doc_sec:{_collapse_ws(m.group(1))}")
    code_lines = [
        ln for ln in joined.splitlines() if not ln.lstrip().startswith("///") and not ln.lstrip().startswith("//")
    ]
    code = "\n".join(code_lines)
    for m in _IMPL_FOR_RE.finditer(code):
        headers.add(f"impl_for:{_leaf_ident(m.group(1))}:{_leaf_ident(m.group(2))}")
    for m in _IMPL_INH_RE.finditer(code):
        span_txt = m.group(0)
        if " for " not in span_txt:
            headers.add(f"impl:{_leaf_ident(m.group(1))}")
    for m in _ITEM_DECL_RE.finditer(code):
        kw, raw_name = m.group(1), m.group(2)
        name = raw_name[2:] if raw_name.startswith("r#") else raw_name
        headers.add(f"{kw}:{name}")
    return sorted(headers)


def normalize_code_action(res):
    if not isinstance(res, list):
        return {"kinds": [], "titles": []}
    kinds = sorted({str(it.get("kind") or "") for it in res if isinstance(it, dict) and it.get("kind")})
    titles = sorted({str(it.get("title") or "") for it in res if isinstance(it, dict) and it.get("title")})
    return {"kinds": kinds, "titles": titles}


def normalize_code_action_resolve(res, clean_uri_fn, repo):
    if not isinstance(res, list):
        return {"titles": [], "by_title": {}, "norm_text_by_title": {}, "headers_by_title": {}, "kinds": []}
    by_title = {}
    norm_text_by_title = {}
    headers_by_title = {}
    kinds = set()
    for it in res:
        if not isinstance(it, dict):
            continue
        title = str(it.get("title") or "")
        if not title:
            continue
        edits_by_uri, ops = _extract_edits_by_uri(it.get("edit"), clean_uri_fn, repo)
        cmd = (it.get("command") or {}).get("command") if isinstance(it.get("command"), dict) else None
        if not edits_by_uri and not ops and not cmd:
            continue
        files = sorted(edits_by_uri.keys())
        prev_files = set(by_title.get(title, []))
        by_title[title] = sorted(prev_files | set(files))
        per_uri_norm = dict(norm_text_by_title.get(title, {}))
        raw_chunks = []
        for cu, dedup in edits_by_uri.items():
            raws = [t[4] for t in dedup]
            raw_chunks.extend(raws)
            applied = _applied_text(repo, cu, dedup) if repo else None
            if applied is not None:
                # Whitespace is collapsed as in the edit-text comparison it replaces.
                per_uri_norm[cu] = "<text:" + hashlib.sha1(_collapse_ws(applied).encode("utf-8")).hexdigest() + ">"
                continue
            collapsed = [_collapse_ws(x) for x in raws if _collapse_ws(x)]
            per_uri_norm[cu] = " ".join(collapsed) if collapsed else f"<delete:{len(dedup)}>"
        for op in ops:
            per_uri_norm[f"__op__:{op}"] = op
        if cmd and not per_uri_norm:
            per_uri_norm["__cmd__"] = f"<cmd:{cmd}>"
        norm_text_by_title[title] = dict(sorted(per_uri_norm.items()))
        prev_hdrs = set(headers_by_title.get(title, []))
        headers_by_title[title] = sorted(prev_hdrs | set(_extract_item_headers(raw_chunks)))
        if it.get("kind"):
            kinds.add(str(it["kind"]))
    titles = sorted(by_title.keys())
    return {
        "titles": titles,
        "by_title": {t: by_title[t] for t in titles},
        "norm_text_by_title": {t: norm_text_by_title.get(t, {}) for t in titles},
        "headers_by_title": {t: headers_by_title.get(t, []) for t in titles},
        "kinds": sorted(kinds),
    }


def compare_code_action(r_norm, a_norm):
    rk, ak = set(r_norm["kinds"]), set(a_norm["kinds"])
    rt, at = set(r_norm["titles"]), set(a_norm["titles"])
    if rt == at:
        return "exact_match", None, {"titles": sorted(rt)}
    a_valid = {t for t in at if not _is_ra_placeholder_stub(t)}
    if rt and rt == a_valid and at - a_valid:
        return "partial", "p_code_action_ra_has_placeholder_stub", {
            "titles": sorted(rt),
            "ra_stubs": sorted(at - a_valid),
        }
    if rt & at:
        return "partial", "p_code_action_shared_titles_diff_set", {
            "common": sorted(rt & at)[:5],
            "ridx_only": sorted(rt - at)[:5],
            "ra_only": sorted(at - rt)[:5],
        }
    if rk & ak:
        return "partial", "p_code_action_shared_kind_diff_titles", {
            "ridx_kinds": sorted(rk),
            "ra_kinds": sorted(ak),
            "ridx_titles": sorted(rt)[:5],
            "ra_titles": sorted(at)[:5],
        }
    return "mismatch", "p_code_action_disjoint_kinds", {
        "ridx_kinds": sorted(rk),
        "ra_kinds": sorted(ak),
        "ridx_titles": sorted(rt)[:5],
        "ra_titles": sorted(at)[:5],
    }


def _title_edits_core_compatible(t, r_norm, a_norm) -> bool:
    if r_norm["by_title"].get(t) != a_norm["by_title"].get(t):
        return False
    r_txt = r_norm.get("norm_text_by_title", {}).get(t)
    a_txt = a_norm.get("norm_text_by_title", {}).get(t)
    if r_txt == a_txt and r_txt is not None:
        return True
    r_hdr = r_norm.get("headers_by_title", {}).get(t) or []
    a_hdr = a_norm.get("headers_by_title", {}).get(t) or []
    return bool(r_hdr) and r_hdr == a_hdr


def compare_code_action_resolve(r_norm, a_norm):
    rt, at = set(r_norm["titles"]), set(a_norm["titles"])
    if not rt and not at:
        return "both_empty", None, {}
    if rt == at and rt:
        r_txt = r_norm.get("norm_text_by_title", {})
        a_txt = a_norm.get("norm_text_by_title", {})
        if r_norm["by_title"] == a_norm["by_title"] and r_txt == a_txt:
            return "exact_match", None, {"resolved": len(rt)}
        diff_titles = sorted(t for t in rt if r_txt.get(t) != a_txt.get(t))
        if r_norm["by_title"] == a_norm["by_title"] and all(
            _title_edits_core_compatible(t, r_norm, a_norm) for t in rt
        ):
            return "core_match", "c_code_action_resolve_same_headers_diff_body", {
                "resolved": len(rt),
                "diff_titles": diff_titles[:5],
            }
        return "partial", "p_code_action_resolve_diff_edits", {
            "resolved": len(rt),
            "diff_titles": diff_titles[:5],
        }
    a_valid = {t for t in at if not _is_ra_placeholder_stub(t)}
    if rt and rt == a_valid and at - a_valid:
        return "partial", "p_code_action_resolve_ra_has_placeholder_stub", {
            "resolved": sorted(rt),
            "ra_stubs": sorted(at - a_valid),
        }
    if rt & at:
        return "partial", "p_code_action_resolve_partial_overlap", {
            "common": sorted(rt & at)[:5],
            "ridx_only": sorted(rt - at)[:5],
            "ra_only": sorted(at - rt)[:5],
        }
    if rt and not at:
        return "ridx_only", "c_code_action_resolve_ridx_only", {"ridx_titles": sorted(rt)[:5]}
    return "mismatch", "p_code_action_resolve_disjoint", {
        "ridx_titles": sorted(rt)[:5],
        "ra_titles": sorted(at)[:5],
    }
