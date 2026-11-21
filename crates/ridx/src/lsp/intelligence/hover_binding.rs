/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Hover on a binding where the index has its type but no definition record, the binding itself: a `let`, a
//! parameter, a closure's parameter, a name in a pattern. As rust-analyzer shows it: `[let] [mut] name: Type`, the
//! layout under it (`size = 1, align = 0x1, no Drop`), and the name's range (prod-code, m=20120: a parameter showed
//! `Type: bool`, with no layout and no range). What is a binding is the syntax tree's to say, an identifier pattern,
//! never the characters around the name (review of 2e1fcfb1: `f(42)` read as a binding `42`).

use ridx_syntax::{SyntaxKind as K, TokenKind as T};
use serde_json::{json, Value};

/// A binding found at a position: its name (a raw identifier with its `r#`), the byte range of the name and the byte
/// offset where its pattern starts (at its `ref` or `mut`, as the compiler's span of it and the layout note recorded there).
struct Binding<'s> {
    name: &'s str,
    name_range: std::ops::Range<usize>,
    pattern_start: usize,
    is_let: bool,
    is_mut: bool,
}

/// The binding whose name covers byte `offset` of `src`.
fn binding_at(src: &str, offset: usize) -> Option<Binding<'_>> {
    let tree = ridx_syntax::parse(src);
    let token = tree.tokens[ridx_syntax::token_at(&tree.tokens, offset as u32)?];
    if !matches!(token.kind, T::Ident | T::RawIdent) {
        return None;
    }
    let name = tree.covering(offset);
    let pat = tree.parent(name).filter(|&p| tree.kind(name) == K::Name && tree.kind(p) == K::IdentPat)?;
    let is_mut = tree.own_tokens(pat).any(|(_, t)| t.kind == T::Mut);
    let is_let = tree.parent(pat).is_some_and(|p| tree.kind(p) == K::LetStmt);
    let name_range = tree.range(name);
    Some(Binding { name: &src[name_range.clone()], name_range, pattern_start: tree.range(pat).start, is_let, is_mut })
}

/// Byte offset of 1-based char column `col` on 0-based line `line` of `src`.
fn offset_of(src: &str, line: u32, col: u32) -> Option<usize> {
    let start = if line == 0 { 0 } else { src.match_indices('\n').nth(line as usize - 1)?.0 + 1 };
    let text = &src[start..src[start..].find('\n').map_or(src.len(), |e| start + e)];
    Some(start + text.char_indices().nth(col.saturating_sub(1) as usize).map_or(text.len(), |(i, _)| i))
}

/// 0-based line, and the column of byte `offset` in `unit` (chars or UTF-16 code units) on it.
fn position(src: &str, offset: usize, utf16: bool) -> (u32, usize) {
    let line_start = src[..offset].rfind('\n').map_or(0, |i| i + 1);
    let line = src[..line_start].matches('\n').count() as u32;
    let before = &src[line_start..offset];
    (line, if utf16 { before.encode_utf16().count() } else { before.chars().count() })
}

/// The hover of the binding named at 1-based char column `col` of 0-based line `line_0` of `src`, when the index has
/// its type `ty` and no definition there; `None` unless the syntax tree has an identifier pattern there. `layout_at`
/// gives the layout note recorded at a 0-based line and 1-based char column of `src`: where the pattern starts, on
/// its own line (review of 68375e6e: a `mut` on the line above lost the layout).
pub(crate) fn binding_hover(src: &str, line_0: u32, col: u32, ty: &str, layout_at: &dyn Fn(u32, u32) -> Option<String>) -> Option<Value> {
    let b = binding_at(src, offset_of(src, line_0, col)?)?;
    let text = format!("{}{}{}: {}", if b.is_let { "let " } else { "" }, if b.is_mut { "mut " } else { "" }, b.name, super::hover_decl::short_type(ty));
    let block = format!("```rust\n{text}\n```");
    let (pat_line, pat_col) = position(src, b.pattern_start, false);
    let layout = layout_at(pat_line, pat_col as u32 + 1);
    let value = match layout {
        Some(layout) => format!("{block}\n\n---\n\n{layout}"),
        None => block,
    };
    let ((sl, sc), (el, ec)) = (position(src, b.name_range.start, true), position(src, b.name_range.end, true));
    Some(json!({
        "contents": { "kind": "markdown", "value": value },
        "range": { "start": { "line": sl, "character": sc }, "end": { "line": el, "character": ec } }
    }))
}

#[cfg(test)]
#[path = "hover_binding_tests.rs"]
mod tests;
