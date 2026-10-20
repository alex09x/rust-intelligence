# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Pure rules of `position_ab_content.py`: token decoding, where a token lands in the live file,
the content classes (semantic tokens, call hierarchy, root lib.rs, rename) and the scrubbing of
private path prefixes. No server and no filesystem beyond the live file's lines."""

import os
from collections import Counter

from matrix_categories import LSP_SPECS
from position_ab_classify import target_kind_of

RPC = {key: rpc for _, key, rpc, _, _ in LSP_SPECS}
TOKEN_KEYS = frozenset({"semantic_tokens_full", "semantic_tokens_range", "semantic_tokens_delta"})
CALL_KEYS = frozenset({"prepare_call_hierarchy", "incoming_calls", "outgoing_calls"})
WORD = frozenset("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_:")
KEYWORDS = frozenset(
    "as async await break const continue crate dyn else enum extern false fn for if impl in let loop "
    "match mod move mut pub ref return self Self static struct super trait true type unsafe use where while".split()
)
CLASS_TOKENS, CLASS_CALLS, CLASS_NONE = "i_semantic_tokens", "ii_call_hierarchy", "iv_no_indexed_target"
CLASS_ROOT, CLASS_RENAME = "v_root_package_lib", "vi_rename_symbol_from_another_file"
CLASS_NOT_STORED, CLASS_PARENT_DIR = "iv_file_not_stored", "viii_stored_with_parent_dir"
RENAME_KEYS = frozenset({"prepare_rename", "rename"})


def decode_tokens(data):
    """(line, start, length, type) of every token of an LSP semanticTokens data array."""
    out, line, start = [], 0, 0
    for i in range(0, len(data) - len(data) % 5, 5):
        delta_line, delta_start, length, kind = data[i], data[i + 1], data[i + 2], data[i + 3]
        if delta_line:
            line += delta_line
            start = delta_start
        else:
            start += delta_start
        out.append((line, start, length, kind))
    return out


def token_data(res):
    return res.get("data") if isinstance(res, dict) and isinstance(res.get("data"), list) else None


def uri_path(uri):
    return uri[len("file://"):] if isinstance(uri, str) and uri.startswith("file://") else uri


def scrub_path(path, repo):
    """Path without its private prefix: `ws:` relative to the workspace, `registry:` after the
    crate index directory, `git:` after the checkout directory, `sysroot:` after the rust-src root,
    otherwise `external:` with the last two components."""
    p = str(path).replace("\\", "/")
    repo = str(repo).rstrip("/")
    if p == repo or p.startswith(repo + "/"):
        return "ws:" + p[len(repo) + 1:]
    for marker, prefix, skip in (("/registry/src/", "registry:", 1), ("/git/checkouts/", "git:", 2), ("/lib/rustlib/src/rust/", "sysroot:", 0)):
        idx = p.find(marker)
        if idx >= 0:
            rest = p[idx + len(marker):].split("/")
            return prefix + "/".join(rest[skip:])
    return "external:" + "/".join(p.split("/")[-2:])


def uris_of(res):
    out = []

    def walk(x):
        if isinstance(x, dict):
            for k, v in x.items():
                out.append(v) if k == "uri" and isinstance(v, str) else walk(v)
        elif isinstance(x, list):
            for v in x:
                walk(v)

    walk(res)
    return out


def utf16_to_char(line_text, col):
    """Character index of UTF-16 unit `col` in `line_text` (surrogate halves count as one)."""
    units = line_text.encode("utf-16-le")
    return len(units[:2 * col].decode("utf-16-le", errors="ignore"))


def char_span(lines, token):
    """(line, char column, char length, type) of a token given in UTF-16 units, or None past the end."""
    line, col, length = token[0], token[1], token[2]
    if line >= len(lines):
        return None
    text = lines[line]
    start = utf16_to_char(text, col)
    end = utf16_to_char(text, col + length)
    return (line, start, end - start, token[3])


def span_kind(lines, token):
    """Where a token lands in the live file: past_eof, past_eol, comment_or_keyword (a `//` line
    or a Rust keyword, never a name the index could emit), not_identifier (the span is not a whole
    identifier or path) or identifier. Columns are UTF-16 units, as in the protocol."""
    line, col, length = token[0], token[1], token[2]
    if line >= len(lines):
        return "past_eof"
    units = lines[line].encode("utf-16-le")
    if 2 * (col + length) > len(units):
        return "past_eol"

    def unit_text(a, b):
        return units[2 * a:2 * b].decode("utf-16-le", errors="replace")

    segment = unit_text(col, col + length)
    if lines[line].lstrip().startswith("//") or segment.rstrip(":") in KEYWORDS:
        return "comment_or_keyword"
    whole = segment != "" and all(ch in WORD for ch in segment)
    left = col == 0 or unit_text(col - 1, col) not in WORD
    right = 2 * (col + length) == len(units) or unit_text(col + length, col + length + 1) not in WORD
    return "identifier" if whole and left and right else "not_identifier"


def classify_tokens(lines, before, after, foreign_of):
    """Evidence for class i from two decoded token lists; `foreign_of(token)` names the file sharing
    the path tail in whose after tokens the identifier token starts (character columns; the old code
    converted a foreign token's columns with the live file's text and clamped the end), or None."""
    cb, ca = Counter(before), Counter(after)
    removed, added = cb - ca, ca - cb
    counts, files, unexplained = Counter(), set(), []
    for tok, n in removed.items():
        kind = span_kind(lines, tok)
        if kind == "identifier":
            found = foreign_of(tok)
            kind = "identifier_starts_same_tail_token" if found else "unexplained"
            if found:
                files.add(found)
        counts[kind] += n
        if kind == "unexplained":
            unexplained.append([tok[0] + 1, tok[1], tok[2], tok[3]])
    displaced = sum(n for t, n in added.items() if any(t[0] == u[0] and t[1] < u[1] + u[2] and t[1] + t[2] > u[1] for u in removed))
    added_unexplained = sum(added.values()) - displaced
    evidence = {
        "compared": "semantic_tokens_full", "before_tokens": len(before), "after_tokens": len(after),
        "common": sum((cb & ca).values()),
        "removed": {k: counts[k] for k in ("past_eof", "past_eol", "not_identifier", "comment_or_keyword", "identifier_starts_same_tail_token", "unexplained")},
        "removed_identifier_files": sorted(files),
        "added": {"displaced_by_removed_token": displaced, "unexplained": added_unexplained},
        "unexplained_removed": unexplained[:10],
    }
    return counts["unexplained"] == 0 and added_unexplained == 0, evidence


def classify_calls(removed_uris, added_uris, repo, entries, root_lib_stored=False):
    """Evidence for class ii: where every removed and added call-hierarchy item lives. With a stored
    root package `src/lib.rs` (`root_lib_stored`), items added in `crates/*/src/lib.rs` files belong
    to class v: the old suffix match sent those call sites to the root package's file."""
    repo = str(repo).rstrip("/")

    def where(uri):
        p = uri_path(uri)
        if not p.startswith(repo + "/"):
            return "outside_workspace"
        return "no_indexed_target" if target_kind_of(os.path.relpath(p, repo), entries) == "none" else "workspace"

    removed = [{"path": scrub_path(uri_path(u), repo), "where": where(u)} for u in sorted(removed_uris)]
    added = [{"path": scrub_path(uri_path(u), repo), "where": where(u)} for u in sorted(added_uris)]
    in_crate_lib = lambda item: is_crate_lib(item["path"][len("ws:"):]) if item["path"].startswith("ws:") else False
    root_case = root_lib_stored and not removed and added and all(in_crate_lib(a) for a in added)
    ok = all(r["where"] != "workspace" for r in removed) and not added
    cls = CLASS_ROOT if root_case else CLASS_CALLS
    return ok or root_case, cls, {"removed_items": removed, "added_items": added, "root_lib_stored": root_lib_stored}


def is_crate_lib(rel):
    parts = rel.split("/")
    return len(parts) == 4 and parts[0] == "crates" and parts[2] == "src" and parts[3] == "lib.rs"


def classify_root_lib(rel, before_uris, after_uris, repo):
    """Evidence for class v: the root package's src/lib.rs among the before locations."""
    repo = str(repo).rstrip("/")
    root = f"file://{repo}/src/lib.rs"
    ok = is_crate_lib(rel) and root in before_uris
    return ok, {
        "before_locations": sorted({scrub_path(uri_path(u), repo) for u in before_uris}),
        "after_locations": sorted({scrub_path(uri_path(u), repo) for u in after_uris}),
        "stored_path_hit_by_suffix_match": "src/lib.rs" if ok else None,
    }


def classify_rename(symbol, err_before, res_after):
    """Evidence for class vi: the before error named another symbol, the after answer names the
    symbol under the cursor (prepareRename placeholder, or a rename edit whose text is the new
    name at the cursor's file)."""
    message = (err_before or {}).get("message") if isinstance(err_before, dict) else None
    prefix = "Cannot rename external symbol '"
    named = message[len(prefix):].rstrip("'") if message and message.startswith(prefix) else None
    placeholder = res_after.get("placeholder") if isinstance(res_after, dict) else None
    edits = sum(len(v) for v in res_after.get("changes", {}).values()) if isinstance(res_after, dict) and "changes" in res_after else None
    ok = named is not None and named != symbol and (placeholder == symbol or (edits or 0) > 0)
    return ok, {"before_error": {"code": (err_before or {}).get("code") if isinstance(err_before, dict) else None, "message": message},
                "before_symbol_named": named, "symbol_under_cursor": symbol, "after_placeholder": placeholder, "after_edits": edits}


def is_empty(res):
    return res in (None, [], {}) or (isinstance(res, dict) and res.get("data") == [])




def stored_forms_of(rel, abs_path, file_table):
    """Shards whose file table names the live file: `plain` holds (shard, stored) with the stored path
    equal to the workspace-relative or absolute path, `parent_dir` those that only equal it once their
    `..` components are resolved, `same_tail` those whose stored path is a component suffix of it."""
    from pathlib import PurePosixPath

    plain, parent_dir, same_tail = [], [], []
    parts = rel.split("/")
    for shard, stored in file_table:
        if stored == rel or stored == abs_path:
            plain.append((shard, stored))
        elif ".." in stored and resolve_parent_dirs(stored) == rel:
            parent_dir.append((shard, stored))
        elif not stored.startswith("/") and parts[-len(stored.split("/")):] == stored.split("/") and len(stored.split("/")) < len(parts):
            same_tail.append((shard, stored))
    return {"plain": plain, "parent_dir": parent_dir, "same_tail": same_tail}


def resolve_parent_dirs(path):
    out = []
    for c in path.split("/"):
        if c == "." or c == "":
            continue
        if c == ".." and out and out[-1] != "..":
            out.pop()
        else:
            out.append(c)
    return "/".join(out)
