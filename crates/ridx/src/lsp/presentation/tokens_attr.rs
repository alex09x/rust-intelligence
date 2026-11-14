/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Tokens inside attributes as rust-analyzer types them: the paths a `#[derive(…)]` lists are derive macros,
//! everything else inside a built-in attribute (`cfg(not(…))`, lint names, `derive` itself) carries only lexical
//! highlighting, and the arguments of a macro's attribute (a derive helper `#[arg(value_parser = f(x))]`, an
//! attribute macro's) are code the macro expands, highlighted as such.

use super::tokens::{RawToken, TOKEN_TYPE_MACRO};
use super::tokens_scan::byte_to_utf16_col;

/// A `(line, UTF-16 column)` range, end exclusive.
pub(super) type Span = ((u32, u32), (u32, u32));

pub(super) struct AttrSpans {
    attrs: Vec<Span>,
    derive_args: Vec<Span>,
    /// Attributes no built-in names: derive helpers and attribute macros, whose arguments a macro expands.
    pub macro_attrs: Vec<Span>,
}

/// Attributes the compiler or a tool defines: their arguments are no code a macro expands.
const BUILTIN_ATTRS: &[&str] = &[
    "cfg",
    "cfg_attr",
    "derive",
    "allow",
    "warn",
    "deny",
    "forbid",
    "expect",
    "doc",
    "inline",
    "cold",
    "must_use",
    "repr",
    "test",
    "bench",
    "ignore",
    "should_panic",
    "path",
    "macro_use",
    "macro_export",
    "no_mangle",
    "export_name",
    "link_section",
    "used",
    "track_caller",
    "non_exhaustive",
    "deprecated",
    "automatically_derived",
    "rustfmt",
    "clippy",
    "diagnostic",
    "feature",
    "recursion_limit",
    "no_std",
    "no_implicit_prelude",
    "crate_type",
    "crate_name",
    "global_allocator",
    "panic_handler",
    "target_feature",
    "unsafe",
    "link",
    "link_name",
    "proc_macro",
    "proc_macro_derive",
    "proc_macro_attribute",
    "rustc_builtin_macro",
    "allow_internal_unstable",
    "stable",
    "unstable",
    "rustc_const_stable",
];

pub(super) fn attr_spans(text: &str) -> AttrSpans {
    let m = crate::lsp::editing::action_parse::mask_comments_and_strings(text);
    let b = m.as_bytes();
    let mut line_starts = vec![0usize];
    line_starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
    let pos = |off: usize| {
        let l = line_starts.partition_point(|&s| s <= off) - 1;
        let line_end = line_starts.get(l + 1).map_or(text.len(), |&s| s - 1);
        (l as u32, byte_to_utf16_col(&text[line_starts[l]..line_end], off - line_starts[l]))
    };
    let close_of = |open: usize| {
        let mut depth = 0i32;
        for (j, &c) in b.iter().enumerate().skip(open) {
            match c {
                b'[' | b'(' | b'{' => depth += 1,
                b']' | b')' | b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(j);
                    }
                }
                _ => {}
            }
        }
        None
    };
    let mut out = AttrSpans { attrs: Vec::new(), derive_args: Vec::new(), macro_attrs: Vec::new() };
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'#' {
            i += 1;
            continue;
        }
        let mut open = i + 1;
        if b.get(open) == Some(&b'!') {
            open += 1;
        }
        if b.get(open) != Some(&b'[') {
            i += 1;
            continue;
        }
        let Some(close) = close_of(open) else { break };
        out.attrs.push((pos(i), pos(close + 1)));
        let body = m[open + 1..close].trim_start();
        let first = body.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).next().unwrap_or("");
        if !first.is_empty() && !BUILTIN_ATTRS.contains(&first) {
            out.macro_attrs.push((pos(i), pos(close + 1)));
        }
        // `derive(…)` at the top of the attribute or nested (`cfg_attr(feature = "x", derive(…))`).
        for (at, _) in m[open + 1..close].match_indices("derive") {
            let kw = open + 1 + at;
            let mut paren = kw + "derive".len();
            if kw > 0 && (b[kw - 1].is_ascii_alphanumeric() || b[kw - 1] == b'_') {
                continue;
            }
            while paren < close && b[paren].is_ascii_whitespace() {
                paren += 1;
            }
            if b[paren] == b'(' {
                if let Some(end) = close_of(paren) {
                    out.derive_args.push((pos(paren + 1), pos(end)));
                }
            }
        }
        i = close + 1;
    }
    out
}

/// Whether `line` spells, from 1-based char column `col`, the last segment of `target_path` (a reference the source
/// writes, not one a macro put on another token).
pub(super) fn spells_target(line: Option<&str>, col: u32, target_path: &str) -> bool {
    let name = target_path.rsplit("::").next().unwrap_or(target_path);
    let name = name.strip_prefix("r#").unwrap_or(name);
    let Some(line) = line else { return false };
    let at: String = line.chars().skip(col.saturating_sub(1) as usize).take(name.chars().count() + 2).collect();
    let at = at.strip_prefix("r#").unwrap_or(&at);
    !name.is_empty() && at.starts_with(name) && !at[name.len()..].starts_with(|c: char| c.is_alphanumeric() || c == '_')
}

/// Whether the token lies in one of `spans` (sorted by start, not overlapping).
pub(super) fn inside(t: &RawToken, spans: &[Span]) -> bool {
    let (s, e) = ((t.line, t.col), (t.line, t.col + t.length));
    let i = spans.partition_point(|&(start, _)| start <= s);
    i > 0 && e <= spans[i - 1].1
}

/// Retypes derive arguments as macros, keeps the index's tokens in a macro attribute's arguments (the first
/// `resolved` tokens are the index's; a textual guess there is no code the compiler resolved), and drops every
/// other token inside an attribute.
pub(super) fn apply_attribute_rules(text: &str, tokens: &mut Vec<RawToken>, resolved: usize) {
    if !text.contains('#') {
        return;
    }
    let spans = attr_spans(text);
    if spans.attrs.is_empty() {
        return;
    }
    let mut idx = 0usize;
    tokens.retain_mut(|t| {
        idx += 1;
        if !inside(t, &spans.attrs) {
            return true;
        }
        if inside(t, &spans.derive_args) {
            t.token_type = TOKEN_TYPE_MACRO;
            t.token_modifiers = 0;
            return true;
        }
        // The macro attribute's own name (`arg` in `#[arg(…)]`) starts at `#[` + 2; its arguments follow.
        let i = spans.macro_attrs.partition_point(|&(start, _)| start <= (t.line, t.col));
        idx <= resolved && i > 0 && inside(t, &spans.macro_attrs) && (t.line, t.col) > (spans.macro_attrs[i - 1].0 .0, spans.macro_attrs[i - 1].0 .1 + 2)
    });
}
