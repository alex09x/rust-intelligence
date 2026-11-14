/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Semantic tokens inside doc comments, as rust-analyzer emits them: the Rust code of fenced examples, and
//! intra-doc links (`` [`Foo`] ``, `[text][path]`, `[text](path)`) that resolve to an item.

use super::tokens::{defkind_to_token, RawToken, TOKEN_TYPE_MACRO};
use super::tokens_doc_macro::MacroArgs;
use super::tokens_scan::{byte_to_utf16_col, scan_code_line};
use ridx_format::{DefKind, WorkspaceIndex};

/// Whether a fence's info string marks Rust code, as rustdoc reads it.
fn is_rust_fence(info: &str) -> bool {
    let (mut rust, mut other) = (false, false);
    for tok in info.split(|c: char| c == ',' || c.is_whitespace()).map(str::trim) {
        match tok {
            "" => {}
            "rust" => rust = true,
            "should_panic" | "ignore" | "no_run" | "compile_fail" | "allow_fail" | "test_harness" | "standalone_crate" => rust = !other,
            t if t.starts_with("edition") => rust = !other,
            _ => other = true,
        }
    }
    !other || rust
}

/// Where the block comment open at `depth` closes on `line`, scanning from `from`: block comments nest, so a
/// `/*` inside (`src/**/foo.rs`) opens a level its `*/` closes. Updates `depth`; the byte offset of the final `*/`.
fn block_close(line: &str, from: usize, depth: &mut u32) -> Option<usize> {
    let b = line.as_bytes();
    let mut i = from;
    while i + 1 < b.len() {
        match (b[i], b[i + 1]) {
            (b'/', b'*') => {
                *depth += 1;
                i += 2;
            }
            (b'*', b'/') => {
                *depth -= 1;
                if *depth == 0 {
                    return Some(i);
                }
                i += 2;
            }
            _ => i += 1,
        }
    }
    None
}

/// Byte range of a doc line's text: a `///` / `//!` line, or a line of a `/** … */` / `/*! … */` block
/// (`depth` carries the block's nesting across lines, 0 outside). `None` for any other line.
fn doc_content(line: &str, depth: &mut u32) -> Option<(usize, usize)> {
    let lead = line.len() - line.trim_start().len();
    let t = &line[lead..];
    if *depth > 0 {
        let end = block_close(line, 0, depth);
        let start = if t.starts_with('*') && !t.starts_with("*/") { lead + 1 } else { lead };
        return Some((start, end.unwrap_or(line.len()).max(start)));
    }
    if (t.starts_with("///") && !t.starts_with("////")) || t.starts_with("//!") {
        return Some((lead + 3, line.len()));
    }
    if (t.starts_with("/**") && !t.starts_with("/***") && !t.starts_with("/**/")) || t.starts_with("/*!") {
        *depth = 1;
        let end = block_close(line, lead + 3, depth);
        return Some((lead + 3, end.unwrap_or(line.len())));
    }
    None
}

/// Tokens for every doc comment of `text`. `ws` resolves intra-doc links; without it links stay plain.
pub(super) fn scan_doc_comments(text: &str, ws: Option<&WorkspaceIndex>, tokens: &mut Vec<RawToken>) {
    let mut fence: Option<(&str, bool)> = None;
    let (mut in_block_comment, mut in_doc_block) = (false, 0u32);
    let mut macro_args = MacroArgs::default();
    for (line_idx, full_line) in text.lines().enumerate() {
        let Some((start, end)) = doc_content(full_line, &mut in_doc_block) else {
            fence = None;
            continue;
        };
        let line = &full_line[..end];
        let content = &line[start..];
        let trimmed = content.trim_start();
        let marker = if trimmed.starts_with("```") {
            Some("```")
        } else if trimmed.starts_with("~~~") {
            Some("~~~")
        } else {
            None
        };
        match (fence, marker) {
            (None, Some(m)) => {
                fence = Some((m, is_rust_fence(trimmed.trim_start_matches(m.as_bytes()[0] as char))));
                in_block_comment = false;
                macro_args = MacroArgs::default();
            }
            (Some((open, _)), Some(m)) if m == open => fence = None,
            (Some((_, true)), _) => {
                // A hidden doctest line (`# use x;`) is code too.
                let lead = content.len() - trimmed.len();
                let skip = if trimmed == "#" || trimmed.starts_with("# ") { 1 } else { 0 };
                let first = tokens.len();
                scan_code_line(line, line, start + lead + skip, line_idx as u32, &mut in_block_comment, true, tokens);
                macro_args.retype_line(line, start + lead + skip, &mut tokens[first..]);
            }
            (Some(_), _) => {}
            (None, None) => {
                if let Some(ws) = ws {
                    scan_links(line, start, line_idx as u32, ws, tokens);
                }
            }
        }
    }
}

/// Intra-doc links on one doc line, each as one token over the whole link, typed by the item it resolves to.
fn scan_links(line: &str, from: usize, line_idx: u32, ws: &WorkspaceIndex, tokens: &mut Vec<RawToken>) {
    let b = line.as_bytes();
    let mut i = from;
    let mut in_code = false;
    while i < b.len() {
        match b[i] {
            b'`' => in_code = !in_code,
            b'[' if !in_code => {
                if let Some((end, target)) = parse_link(line, i) {
                    if let Some((tt, tm)) = resolve_link(ws, target) {
                        let col = byte_to_utf16_col(line, i);
                        tokens.push(RawToken { line: line_idx, col, length: byte_to_utf16_col(line, end) - col, token_type: tt, token_modifiers: tm });
                    }
                    i = end;
                    continue;
                }
            }
            _ => {}
        }
        i += 1;
    }
}

/// `[text]`, `[text][dest]` or `[text](dest)` at `open`: the link's end and the path it names.
fn parse_link(line: &str, open: usize) -> Option<(usize, &str)> {
    let close = open + 1 + line[open + 1..].find(']')?;
    let text = &line[open + 1..close];
    let rest = &line[close + 1..];
    if let Some(r) = rest.strip_prefix('[') {
        let e = r.find(']')?;
        return Some((close + 2 + e + 1, &r[..e]));
    }
    if let Some(r) = rest.strip_prefix('(') {
        let e = r.find(')')?;
        return Some((close + 2 + e + 1, &r[..e]));
    }
    if rest.starts_with(':') {
        return None; // a link definition `[name]: dest`
    }
    Some((close + 1, text))
}

/// Token for a link destination that names an item (`Foo`, `crate::ser`, `Vec::push`, `name!`, `f()`,
/// `struct@Foo`), when the index knows it.
fn resolve_link(ws: &WorkspaceIndex, target: &str) -> Option<(u32, u32)> {
    let t = target.trim().trim_matches('`');
    let t = t.split_once('@').map_or(t, |(_, p)| p);
    if t.is_empty() || t.contains("://") || t.starts_with('#') || t.ends_with(".html") || t.ends_with(".md") {
        return None;
    }
    let is_macro = t.ends_with('!');
    let path = t.trim_end_matches("()").trim_end_matches('!');
    let path = path.split('<').next().unwrap_or(path);
    if !path.split("::").all(|s| !s.is_empty() && s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')) {
        return None;
    }
    let name = path.rsplit("::").next()?;
    let suffix: Vec<&str> = path.split("::").filter(|s| !matches!(*s, "crate" | "self" | "super" | "Self")).collect();
    let candidates = ws.exact_symbols_by_name(name);
    let fits = |p: &str| {
        let segs: Vec<&str> = p.split("::").collect();
        segs.len() >= suffix.len() && segs[segs.len() - suffix.len()..] == suffix[..]
    };
    let kind = candidates
        .iter()
        .filter(|s| !matches!(s.kind, DefKind::Use | DefKind::Local | DefKind::Impl | DefKind::TyParam | DefKind::Other))
        .filter(|s| !is_macro || s.kind == DefKind::Macro)
        .find(|s| fits(s.path))
        .map(|s| s.kind)?;
    let (tt, tm) = defkind_to_token(kind);
    Some((if is_macro { TOKEN_TYPE_MACRO } else { tt }, tm))
}
