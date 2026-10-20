# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Classes for the reverse rows: RIDX answers, rust-analyzer is empty on the first answer and on
the retry. Rules in order; the first that holds wins.

  r1_ra_answers_on_reask          rust-analyzer answers when asked a third time: noise, not a gap.
  r4_cursor_in_attribute_or_string  the cursor is on an attribute line or inside a string literal
                                  (serde attribute paths): rust-analyzer gives no highlights there.
  r3a_cursor_in_cfg_macro_block   rust-analyzer has no definition at the cursor and the line is
                                  inside a `cfg_*! { ... }` block (tokio's feature macros): code
                                  rust-analyzer does not analyze under its configuration.
  r3b_cursor_under_cfg_attribute  no rust-analyzer definition and a `#[cfg(...)]` attribute gates an
                                  enclosing item in this file, or `#![cfg(...)]` the whole file.
  r3c_cursor_in_macro             no rust-analyzer definition and the cursor is inside another macro
                                  invocation or a `macro_rules!` body.
  r3_ra_no_definition_at_cursor   no rust-analyzer definition and none of the above: rust-analyzer
                                  does not resolve the token.
  r6_ridx_impl_keyword_in_signature  implementation: a RIDX location line is not an impl header but
                                  contains `impl ` (an `impl Trait` type in a signature): a false
                                  positive of the handler's line test.
  r5_ridx_impls_for_other_types   implementation: every RIDX location is an `impl ... for X` line
                                  whose implemented type is not the symbol, which only appears in
                                  its generics: a false positive of the line heuristic.
  r2_ra_empty_confirmed           rust-analyzer resolves the cursor, answers empty a third time,
                                  RIDX answers again.
"""

import re

from position_ab_gap_cursor import block_open_at  # noqa: E402

REVERSE_ORDER = ["r1_ra_answers_on_reask", "r4_cursor_in_attribute_or_string", "r3a_cursor_in_cfg_macro_block",
                 "r3b_cursor_under_cfg_attribute", "r3c_cursor_in_macro", "r3_ra_no_definition_at_cursor",
                 "r6_ridx_impl_keyword_in_signature", "r5_ridx_impls_for_other_types", "r2_ra_empty_confirmed"]
CFG_MACRO_RE = re.compile(r"\b(cfg_[A-Za-z0-9_]*)!\s*\{")
CFG_ATTR_RE = re.compile(r"^\s*#!?\[cfg\(")
IMPL_FOR_RE = re.compile(r"^\s*(unsafe\s+)?impl\b(<.*?>)?\s*(?P<trait>.*?)\s+for\s+(?P<target>[A-Za-z_][A-Za-z0-9_:]*)")


def impl_for_target(line_text):
    """The implemented type of an `impl Trait for Type` line, or None for an inherent impl or a
    line that is not an impl header."""
    m = IMPL_FOR_RE.match(line_text)
    return m.group("target").rsplit("::", 1)[-1] if m else None


def impl_keyword_in_signature(line_text):
    """True for a line that the handler takes for an impl because it contains `impl ` although it is
    not an impl header (`fn f() -> impl Iterator<...>`)."""
    stripped = line_text.lstrip()
    return "impl " in stripped and not stripped.startswith(("impl", "unsafe impl"))


def impls_for_other_types(symbol, locations):
    """True when every location is an `impl ... for X` line with X different from `symbol`."""
    targets = [impl_for_target(loc.get("line_text", "")) for loc in locations]
    return bool(targets) and all(t is not None and t != symbol for t in targets)


def _indent(text):
    return len(text) - len(text.lstrip())


def cfg_context(lines, idx, lookback=400):
    """How the line `idx` is gated inside its file: `cfg_macro:<name>` when an enclosing `cfg_*! {`
    block is still open at the line; `cfg_attr:<text>` when a `#[cfg(...)]` attribute stands
    directly above an enclosing item (each less indented header above the line, walking outwards)
    or a crate-level `#![cfg(...)]` stands above; else None."""
    if idx >= len(lines):
        return None
    for j in range(idx - 1, max(-1, idx - lookback), -1):
        m = CFG_MACRO_RE.search(lines[j])
        if m and lines[j].rstrip().endswith("{") and block_open_at(lines, j, idx, "{", "}"):
            return "cfg_macro:" + m.group(1)
    threshold = _indent(lines[idx])
    for j in range(idx - 1, -1, -1):
        text = lines[j]
        stripped = text.strip()
        if stripped.startswith("#!["):
            if CFG_ATTR_RE.match(text):
                return "cfg_attr:" + stripped[:60]
            continue
        if not stripped or stripped.startswith(("#[", "//", "}")) or _indent(text) >= threshold:
            continue
        k = j - 1
        while k >= 0 and (not lines[k].strip() or lines[k].strip().startswith(("#[", "///"))):
            if CFG_ATTR_RE.match(lines[k]):
                return "cfg_attr:" + lines[k].strip()[:60]
            k -= 1
        threshold = _indent(text)
    return None


def classify_reverse(key, evidence):
    """A reverse row from its re-asks and cursor. `evidence`: `ra_reask`, `ridx_reask`, `cursor`
    (`token_class`, `line_text`, `in_macro`, `cfg_context`), `ra_definition` (None or dict), `symbol`,
    `ridx_answer`. Returns (class, note)."""
    if evidence.get("ra_reask") == "a":
        return "r1_ra_answers_on_reask", f"{key}: rust-analyzer answered on the third ask"
    if evidence.get("ra_reask") != "b" or evidence.get("ridx_reask") != "a":
        return "unclassified", f"{key}: re-asks ridx {evidence.get('ridx_reask')}, rust-analyzer {evidence.get('ra_reask')}"
    cursor = evidence.get("cursor") or {}
    if cursor.get("line_text", "").startswith("#") or cursor.get("token_class") == "literal":
        return "r4_cursor_in_attribute_or_string", "cursor line: " + cursor.get("line_text", "")[:60]
    if not evidence.get("ra_definition"):
        cfg = cursor.get("cfg_context") or ""
        if cfg.startswith("cfg_macro:"):
            return "r3a_cursor_in_cfg_macro_block", f"{key}: inside {cfg[len('cfg_macro:'):]}! block, no rust-analyzer definition"
        if cfg.startswith("cfg_attr:"):
            return "r3b_cursor_under_cfg_attribute", f"{key}: under {cfg[len('cfg_attr:'):]}, no rust-analyzer definition"
        if cursor.get("in_macro"):
            return "r3c_cursor_in_macro", f"{key}: cursor inside a macro invocation or macro_rules body, no rust-analyzer definition"
        return "r3_ra_no_definition_at_cursor", f"{key}: rust-analyzer has no definition at the cursor"
    if key == "implementation" and any(impl_keyword_in_signature(loc.get("line_text", "")) for loc in evidence.get("ridx_answer") or []):
        return "r6_ridx_impl_keyword_in_signature", "RIDX location: " + next(loc["line_text"] for loc in evidence["ridx_answer"] if impl_keyword_in_signature(loc.get("line_text", "")))[:60]
    if key == "implementation" and impls_for_other_types(evidence.get("symbol"), evidence.get("ridx_answer") or []):
        return "r5_ridx_impls_for_other_types", f"{len(evidence['ridx_answer'])} RIDX impl lines implement other types"
    return "r2_ra_empty_confirmed", f"{key}: rust-analyzer resolves the cursor, empty three times; RIDX answers again"
