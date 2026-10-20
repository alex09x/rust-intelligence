# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Classes for the positions where RIDX answers empty and rust-analyzer answers, by method, each
decided from evidence that the sidecar records (no row has to be re-asked to check a class).
Rules are tried in the order of `RULE_ORDER`; a row gets the first class whose rule holds, else
`unclassified`. A row whose rust-analyzer re-ask did not answer is `x_ra_reask_failed`.

implementation (RIDX: definition at the cursor, then references of kind Impl in workspace shards)
  i2_cursor_unresolved            RIDX `definition` at the cursor is empty.
  i7_definition_of_another_item   RIDX `definition` at the cursor resolves, but to a line that does
                                  not name the symbol (an enclosing function, a `use` line).
  i3_impls_in_unstored_files      every rust-analyzer location is a workspace file no shard stores.
  i1_impls_outside_workspace      no location is in a stored workspace file.
  i5_generated_impls              every workspace location is a `#[...]` attribute line (or a line
                                  inside a multi-line attribute), the declaration line of the symbol
                                  itself, or the declaration of another item carrying a derive: impls
                                  from derive and attribute macros, which rust-analyzer maps to the
                                  attribute or the item.
  i6_macro_invocation_impls       the remaining workspace locations are macro invocation lines or
                                  lines inside a multi-line macro invocation.
  i4a_unsafe_impl_lines           the remaining workspace locations are `unsafe impl` lines.
  i4d_multiline_impl_header       a workspace location inside an `impl<...>` header that spans lines:
                                  the handler's line test sees no `impl` on the reference's line.
  i4b_impl_line_without_reference an `impl` line in a stored workspace file where RIDX holds no
                                  reference of the symbol.
  i4c_impl_line_with_reference    an `impl` line where RIDX holds a reference of the symbol and
                                  still did not return it.

incomingCalls (RIDX: a Fn starting at the item line, its references that look like calls, the
enclosing function of each)
  c3_item_not_resolved            RIDX `prepareCallHierarchy` at the cursor is empty.
  then by the callers rust-analyzer returned, the row taking the first present category in
  the order of CALLER_PRIORITY:
  c4_workspace_callers_missed     missed: no RIDX reference at the call lines of a workspace
                                  caller; enclosing_unknown: RIDX `definition` at the caller's name
                                  does not resolve to it; body_end_short: the handler's brace scan
                                  from the caller's line ends before the call line;
                                  enclosing_lookup_failed: reference, call site and enclosing
                                  function all known, still no caller returned.
  c5_call_site_not_recognized     RIDX has the reference, but the text after it is not `(`.
  c7_item_without_references      RIDX holds no reference to the item anywhere (the call reaches it
                                  through trait dispatch, which the index records on the trait).
  c2_callers_in_macro_expansions  RIDX's references at the call lines are all macro expansions, or it
                                  has none there and the call lines are inside macro invocations.
  c6_caller_not_a_function        rust-analyzer's caller is not a function or method item.
  c1_callers_outside_workspace    the caller's file is not a stored workspace file.

documentHighlight (RIDX: references at the cursor, in the same file)
  h2a_exit_or_loop_keyword        cursor on fn / return / ? / await / loop / while / for / break /
                                  continue: rust-analyzer marks exit or loop points.
  h2b_keyword_literal_or_lifetime cursor on another keyword, a literal, a lifetime or punctuation.
  h3_macro_site                   cursor inside a macro invocation or a `macro_rules!` body and
                                  RIDX `definition` at the cursor is empty.
  h1_local_binding                RIDX `definition` is empty and rust-analyzer's definition is a
                                  binding site in the same file.
  h4a_resolved_no_references      RIDX `definition` resolves, RIDX `references` is empty.
  h4b_external_item_references_elsewhere  RIDX resolves to a dependency or sysroot item; its
                                  `references` has entries, none in this file.
  h4d_workspace_item_references_elsewhere  the same for a workspace item.
  h4c_resolved_references_in_file RIDX `references` has entries in this file, still no highlight.
  h5a_associated_type_path        `::`-qualified cursor; rust-analyzer's definition is a `type` item.
  h5b_module_path_segment         rust-analyzer's definition is a file start or a `mod` item.
  h5c_item_outside_workspace      rust-analyzer's definition is in a dependency or sysroot file.
  h5d_enum_variant                rust-analyzer's definition is an enum variant line.
  h5g_definition_in_macro_input   rust-analyzer's definition line is inside a macro invocation.
  h5e_workspace_item_unresolved   rust-analyzer's definition is another workspace item.
  h5f_no_definition_either        neither server resolves the cursor.
"""

import re

from position_ab_gap_cursor import MACRO_CALL_RE, is_binding_site  # noqa: E402

RULE_ORDER = {
    "implementation": ["i2_cursor_unresolved", "i7_definition_of_another_item", "i3_impls_in_unstored_files", "i1_impls_outside_workspace",
                       "i5_generated_impls", "i6_macro_invocation_impls", "i4a_unsafe_impl_lines",
                       "i4d_multiline_impl_header", "i4b_impl_line_without_reference", "i4c_impl_line_with_reference"],
    "incoming_calls": ["c3_item_not_resolved", "c4_workspace_callers_missed", "c5_call_site_not_recognized",
                       "c7_item_without_references", "c2_callers_in_macro_expansions", "c6_caller_not_a_function",
                       "c1_callers_outside_workspace"],
    "document_highlight": ["h2a_exit_or_loop_keyword", "h2b_keyword_literal_or_lifetime", "h3_macro_site",
                           "h1_local_binding", "h4a_resolved_no_references", "h4b_external_item_references_elsewhere",
                           "h4d_workspace_item_references_elsewhere", "h4c_resolved_references_in_file",
                           "h5a_associated_type_path", "h5b_module_path_segment", "h5c_item_outside_workspace",
                           "h5d_enum_variant", "h5g_definition_in_macro_input", "h5e_workspace_item_unresolved",
                           "h5f_no_definition_either"],
}
CALLER_PRIORITY = ["missed", "enclosing_unknown", "body_end_short", "enclosing_lookup_failed", "not_call_site",
                   "no_ridx_references", "macro", "non_fn_caller", "outside"]
CALLER_CLASS = {"missed": "c4_workspace_callers_missed", "enclosing_unknown": "c4_workspace_callers_missed",
                "body_end_short": "c4_workspace_callers_missed", "enclosing_lookup_failed": "c4_workspace_callers_missed",
                "not_call_site": "c5_call_site_not_recognized", "no_ridx_references": "c7_item_without_references",
                "macro": "c2_callers_in_macro_expansions", "non_fn_caller": "c6_caller_not_a_function",
                "outside": "c1_callers_outside_workspace"}
FUNCTION_KINDS = (6, 12)  # LSP SymbolKind Method, Function
OUTSIDE = ("registry", "git", "sysroot", "external")
VARIANT_RE = re.compile(r"^\s*[A-Z][A-Za-z0-9_]*(\s*\(.*\)|\s*\{.*\}|\s*=\s*[^,]+)?\s*,\s*(//.*)?$")
TYPE_ITEM_RE = re.compile(r"^\s*(pub(\([^)]*\))?\s+)?type\s")
MOD_ITEM_RE = re.compile(r"^\s*(pub(\([^)]*\))?\s+)?mod\s")


def store_kind(path, repo, ws_table):
    """Where RIDX could have found the file: `ws` for a workspace file some workspace shard stores
    (plainly or through `..` components), `ws_unstored` for a workspace file no shard stores,
    else the location's realm: `registry`, `git`, `sysroot` or `external`."""
    from position_ab_content_rules import resolve_parent_dirs, scrub_path

    scrubbed = scrub_path(path, repo)
    prefix, rel = scrubbed.split(":", 1)
    if prefix != "ws":
        return prefix
    for _shard, stored in ws_table:
        if stored == rel or stored == str(path) or (".." in stored and resolve_parent_dirs(stored) == rel):
            return "ws"
    return "ws_unstored"


DECL_RE = re.compile(r"^(pub(\([^)]*\))?\s+)?(struct|enum|union|type)\s")


def location_kind(loc):
    """What a rust-analyzer implementation location sits on: `attribute` (an attribute line or a
    line inside a multi-line attribute), `decl` (the symbol's own declaration line), `derived_item`
    (another item's declaration under a derive), `macro` (a macro invocation line, or a line inside
    one), `unsafe_impl`, `impl`, `impl_header` (a later line of an `impl<...>` header) or `other`."""
    text = loc.get("line_text", "").lstrip()
    if text.startswith("#[") or loc.get("in_attribute"):
        return "attribute"
    if loc.get("on_decl_line"):
        return "decl"
    if DECL_RE.match(text) and loc.get("derive_above"):
        return "derived_item"
    if text.startswith("unsafe impl"):
        return "unsafe_impl"
    if text.startswith("impl"):
        return "impl"
    if loc.get("in_impl_header"):
        return "impl_header"
    if MACRO_CALL_RE.match(text) or loc.get("in_macro"):
        return "macro"
    return "other"


def names_symbol(line_text, symbol):
    """True when the line carries `symbol` as a whole word."""
    return bool(symbol) and re.search(r"(?<![A-Za-z0-9_])" + re.escape(symbol) + r"(?![A-Za-z0-9_])", line_text or "") is not None


def classify_implementation(ridx_definition_outcome, locations, definition_names_symbol=True):
    """`locations`: one dict per rust-analyzer location with `store`, `line_text`, `on_decl_line`,
    `in_macro` and `ridx_ref_at_line`; `definition_names_symbol`: some RIDX definition target line
    names the cursor symbol. Returns (class, note)."""
    if ridx_definition_outcome != "a":
        return "i2_cursor_unresolved", "RIDX definition at the cursor: " + ridx_definition_outcome
    if not definition_names_symbol:
        return "i7_definition_of_another_item", "no RIDX definition target line names the symbol"
    stores = [loc["store"] for loc in locations]
    if locations and all(s == "ws_unstored" for s in stores):
        return "i3_impls_in_unstored_files", f"{len(locations)} locations in workspace files no shard stores"
    if locations and all(s != "ws" for s in stores):
        return "i1_impls_outside_workspace", "locations: " + ", ".join(sorted(set(stores)))
    ws = [loc for loc in locations if loc["store"] == "ws"]
    kinds = [location_kind(loc) for loc in ws]
    rest = [k for k in kinds if k not in ("attribute", "decl", "derived_item")]
    if ws and not rest:
        return "i5_generated_impls", f"{len(ws)} workspace locations on attribute or declaration lines"
    if rest and all(k == "macro" for k in rest):
        return "i6_macro_invocation_impls", f"{len(rest)} workspace locations on macro invocations"
    impls = [loc for loc, k in zip(ws, kinds) if k in ("impl", "unsafe_impl")]
    if rest and all(k in ("macro", "unsafe_impl") for k in rest):
        return "i4a_unsafe_impl_lines", f"{len(impls)} unsafe impl lines"
    if "impl_header" in rest:
        return "i4d_multiline_impl_header", f"{rest.count('impl_header')} locations inside multi-line impl headers"
    plain = [loc for loc in impls if location_kind(loc) == "impl"]
    if plain and any(loc.get("ridx_ref_at_line") for loc in plain):
        return "i4c_impl_line_with_reference", f"{sum(1 for loc in plain if loc.get('ridx_ref_at_line'))} of {len(plain)} impl lines carry a RIDX reference"
    if plain:
        return "i4b_impl_line_without_reference", f"{len(plain)} impl lines without a RIDX reference"
    return "unclassified", "workspace locations: " + ", ".join(sorted(set(kinds)))


def is_call_site(line_text, end_col_1based):
    """The handler's call-site test: after the reference span (1-based exclusive end column) the
    text, skipping blanks, starts with `(` or with `::<...>(`."""
    rest = line_text[max(0, end_col_1based - 1):].lstrip()
    if rest.startswith("("):
        return True
    if rest.startswith("::<") and "(" in rest:
        return ">" in rest[3:rest.index("(")]
    return False


def body_end_line(lines, start_line_1based):
    """The handler's brace scan: from the start line, the line that closes the first `{` block, or
    the line of a `;` seen before any `{` outside parentheses and brackets; strings, chars and
    comments are skipped. 1-based, or the file length when nothing closes."""
    depth = paren = 0
    seen_open = in_block = False
    for idx in range(max(0, start_line_1based - 1), len(lines)):
        chars = lines[idx]
        in_string = in_char = escape = False
        i = 0
        while i < len(chars):
            ch = chars[i]
            nxt = chars[i + 1] if i + 1 < len(chars) else ""
            i += 1
            if in_block:
                if ch == "*" and nxt == "/":
                    i += 1
                    in_block = False
                continue
            if escape:
                escape = False
                continue
            if ch == "\\" and (in_string or in_char):
                escape = True
                continue
            if ch == '"' and not in_char:
                in_string = not in_string
                continue
            if ch == "'" and not in_string:
                in_char = not in_char
                continue
            if in_string or in_char:
                continue
            if ch == "/" and nxt == "/":
                break
            if ch == "/" and nxt == "*":
                i += 1
                in_block = True
                continue
            if not seen_open:
                if ch in "([":
                    paren += 1
                elif ch in ")]" and paren > 0:
                    paren -= 1
                elif ch == ";" and paren == 0:
                    return idx + 1
            if ch == "{":
                depth += 1
                seen_open = True
            elif ch == "}":
                depth -= 1
                if seen_open and depth <= 0:
                    return idx + 1
    return len(lines)


def caller_category(caller):
    """One rust-analyzer caller against RIDX's evidence. `caller`: `store`, `kind` (LSP SymbolKind
    of the `from` item), `refs` (RIDX references at the call lines, each with `is_macro_expansion`
    and `call_site`), `from_in_macro` (every call line inside a macro invocation), `ridx_def_at_caller`
    (RIDX definition at the caller's name resolves to the caller's line), `body_end`, `call_line`."""
    if caller["store"] != "ws":
        return "outside"
    if caller.get("kind") not in FUNCTION_KINDS:
        return "non_fn_caller"
    refs = caller.get("refs") or []
    if refs and all(r["is_macro_expansion"] for r in refs):
        return "macro"
    if not refs:
        if caller.get("from_in_macro"):
            return "macro"
        return "no_ridx_references" if caller.get("refs_total") == 0 else "missed"
    if not any(r["call_site"] for r in refs if not r["is_macro_expansion"]):
        return "not_call_site"
    if not caller.get("ridx_def_at_caller"):
        return "enclosing_unknown"
    if caller.get("body_end") is not None and caller.get("call_line") is not None and caller["body_end"] < caller["call_line"]:
        return "body_end_short"
    return "enclosing_lookup_failed"


def classify_incoming_calls(prepare_outcome, callers):
    """Returns (class, note, per-caller categories)."""
    if prepare_outcome != "a":
        return "c3_item_not_resolved", "RIDX prepareCallHierarchy at the cursor: " + prepare_outcome, []
    categories = [caller_category(c) for c in callers]
    for cat in CALLER_PRIORITY:
        if cat in categories:
            return CALLER_CLASS[cat], f"{categories.count(cat)} of {len(categories)} callers {cat}", categories
    return "unclassified", "no callers", categories


def classify_document_highlight(cursor, ridx_definition_outcome, ra_definition, ridx_references, ridx_definition_store=None):
    """`cursor`: `token`, `token_class`, `in_macro`, `qualified`. `ra_definition`: None or dict with
    `same_file`, `store`, `line`, `line_text`, `col`, `token`, `in_macro`. `ridx_references`: `total`
    and `same_file` counts. `ridx_definition_store`: store of RIDX's first definition target.
    Returns (class, note)."""
    if cursor["token_class"] == "exit_loop":
        return "h2a_exit_or_loop_keyword", cursor["token"]
    if cursor["token_class"] in ("keyword", "literal", "lifetime", "punct"):
        return "h2b_keyword_literal_or_lifetime", f"{cursor['token_class']} {cursor['token']}"
    resolved = ridx_definition_outcome == "a"
    if cursor.get("in_macro") and not resolved:
        return "h3_macro_site", "cursor inside a macro invocation or macro_rules body"
    if not resolved and ra_definition and ra_definition.get("same_file") \
            and is_binding_site(ra_definition["line_text"], ra_definition["col"], ra_definition["token"]):
        return "h1_local_binding", "bound at: " + ra_definition["line_text"].strip()[:80]
    if resolved:
        total, same = ridx_references.get("total", 0), ridx_references.get("same_file", 0)
        if total == 0:
            return "h4a_resolved_no_references", "RIDX references at the cursor: empty"
        if same == 0 and ridx_definition_store in OUTSIDE:
            return "h4b_external_item_references_elsewhere", f"RIDX definition in {ridx_definition_store}; references: {total}, none in this file"
        if same == 0:
            return "h4d_workspace_item_references_elsewhere", f"RIDX references: {total}, none in this file"
        return "h4c_resolved_references_in_file", f"RIDX references: {total}, {same} in this file"
    if not ra_definition:
        return "h5f_no_definition_either", "no definition from either server"
    text = ra_definition.get("line_text", "")
    if cursor.get("qualified") and TYPE_ITEM_RE.match(text):
        return "h5a_associated_type_path", "rust-analyzer definition: " + text.strip()[:60]
    if ra_definition.get("line") == 1 or MOD_ITEM_RE.match(text):
        return "h5b_module_path_segment", "rust-analyzer definition: file start or mod item"
    if ra_definition.get("store") in OUTSIDE:
        return "h5c_item_outside_workspace", f"rust-analyzer definition in {ra_definition['store']}"
    if VARIANT_RE.match(text):
        return "h5d_enum_variant", "rust-analyzer definition: " + text.strip()[:60]
    if ra_definition.get("in_macro"):
        return "h5g_definition_in_macro_input", "rust-analyzer definition inside a macro invocation: " + text.strip()[:50]
    return "h5e_workspace_item_unresolved", "rust-analyzer definition: " + text.strip()[:60]
