/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Bindings the overlay parser takes from patterns: closure parameters, `for`, `if let` / `while let`, match arms and
//! destructuring `let`s. Each but the last gets a pattern scope, so what it binds is seen in the closure, loop body,
//! branch or arm only: a name it shadows (`let v = …; |v| v + 1`) no longer resolves to the outer binding, and linked
//! editing of one no longer rewrites the other.

use super::{is_rust_ident_continue, is_rust_ident_start, skip_string_or_char_literal, OverlayParser, OverlaySymbol, Scope};
use ridx_format::DefKind;
use std::cmp::Reverse;

/// Words of a pattern that bind nothing.
const NOT_BINDINGS: &[&str] = &["mut", "ref", "box", "_", "true", "false", "in", "if", "let", "self", "Self", "crate", "super"];

/// Skips whitespace and comments from `pos`.
pub(super) fn skip_trivia(text: &str, pos: &mut usize) {
    let b = text.as_bytes();
    loop {
        while *pos < b.len() && b[*pos].is_ascii_whitespace() {
            *pos += 1;
        }
        if text[*pos..].starts_with("//") {
            *pos = text[*pos..].find('\n').map_or(b.len(), |i| *pos + i);
        } else if text[*pos..].starts_with("/*") {
            *pos = text[*pos + 2..].find("*/").map_or(b.len(), |i| *pos + 4 + i);
        } else {
            return;
        }
    }
}

/// Walks the code of `text` from `from` (comments and literals skipped) and returns the first position `at(pos, depth)`
/// accepts, `depth` being the bracket depth there; `None` once a closing bracket leaves depth 0 or the text ends.
fn find_code(text: &str, from: usize, mut at: impl FnMut(usize, usize) -> bool) -> Option<usize> {
    let b = text.as_bytes();
    let (mut pos, mut depth) = (from, 0usize);
    while pos < b.len() {
        let before = pos;
        skip_trivia(text, &mut pos);
        if pos != before {
            continue;
        }
        if skip_string_or_char_literal(text, &mut pos) {
            continue;
        }
        if at(pos, depth) {
            return Some(pos);
        }
        match b[pos] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' if depth == 0 => return None,
            b')' | b']' | b'}' => depth -= 1,
            _ => {}
        }
        pos += text[pos..].chars().next().map_or(1, char::len_utf8);
    }
    None
}

/// The byte after the `}` closing the block opened at `open`.
fn block_end(text: &str, open: usize) -> usize {
    let b = text.as_bytes();
    find_code(text, open + 1, |p, d| d == 0 && b[p] == b'}').map_or(text.len(), |p| p + 1)
}

/// Where the expression starting at `from` ends: its `,` or `;`, or the bracket closing around it.
fn expr_end(text: &str, from: usize) -> usize {
    let b = text.as_bytes();
    find_code(text, from, |p, d| d == 0 && matches!(b[p], b',' | b';' | b')' | b']' | b'}')).unwrap_or(text.len())
}

/// The first `{` at depth 0 from `from`, unless a `;` comes first.
fn body_brace(text: &str, from: usize) -> Option<usize> {
    let b = text.as_bytes();
    find_code(text, from, |p, d| d == 0 && matches!(b[p], b'{' | b';')).filter(|&p| b[p] == b'{')
}

/// The `{` opening the body after a header expression from `from` (`for x in EXPR {`, `if let P = EXPR {`, `match
/// EXPR {`): a block that is part of the expression (`unsafe { xs }`, `match y { … }`, `{ compute() }`) is followed by
/// the body's `{`, or, when `suffixed`, by a `.method()` or `?` of the expression too.
fn header_body_brace(text: &str, from: usize, suffixed: bool) -> Option<usize> {
    let mut at = from;
    for _ in 0..64 {
        let brace = body_brace(text, at)?;
        let end = block_end(text, brace);
        let mut next = end;
        skip_trivia(text, &mut next);
        let continues = text[next..].starts_with('{') || suffixed && text[next..].starts_with(['.', '?']);
        if !continues {
            return Some(brace);
        }
        at = end;
    }
    None
}

/// Where a match arm whose expression starts at `body` ends: after the `}` closing a block-like expression (`{ … }`,
/// `if … { } else { }`, `match … { }`, loops, `unsafe { }`), which needs no comma; else at its `,` or the match's `}`.
fn arm_end(text: &str, body: usize) -> usize {
    let b = text.as_bytes();
    let rest = &text[body..];
    let block_like = rest.starts_with('{') || ["if", "match", "loop", "for", "while", "unsafe", "async"].iter().any(|w| word_at(text, body, w)) || rest.starts_with('\'');
    if !block_like {
        return expr_end(text, body);
    }
    let mut closed_block = false;
    let hit = find_code(text, body, |p, d| {
        if d == 0 && matches!(b[p], b',' | b'}') {
            return true; // `,` ends the arm; a `}` at depth 0 is the match's own
        }
        if b[p] != b'}' || d != 1 {
            return false;
        }
        let mut next = p + 1;
        skip_trivia(text, &mut next);
        closed_block = !(word_at(text, next, "else") || text[next..].starts_with(['.', '?']));
        closed_block
    });
    match hit {
        Some(p) if closed_block => p + 1,
        Some(p) => p,
        None => text.len(),
    }
}

/// Whether `word` stands as a whole word at `p`.
fn word_at(text: &str, p: usize, word: &str) -> bool {
    text[p..].starts_with(word) && !text[..p].chars().next_back().is_some_and(is_rust_ident_continue) && !text[p + word.len()..].chars().next().is_some_and(is_rust_ident_continue)
}

/// The byte ranges of the names `text[start..end]` binds as a pattern: `x`, `mut x`, `(a, b)`, `Some(v)`, `Point { x, y: py }`.
fn pattern_bindings(text: &str, start: usize, end: usize) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut pos = start;
    while pos < end {
        let before = pos;
        skip_trivia(text, &mut pos);
        if pos != before {
            continue;
        }
        if skip_string_or_char_literal(text, &mut pos) {
            continue;
        }
        let Some(c) = text[pos..].chars().next() else { break };
        if !is_rust_ident_start(c) {
            pos += c.len_utf8();
            continue;
        }
        let s = pos;
        // A raw identifier (`r#type`) is one name, as `parse_ident` reads it.
        if text[pos..].starts_with("r#") && text[pos + 2..].chars().next().is_some_and(is_rust_ident_start) {
            pos += 2;
        }
        while let Some(c) = text[pos..].chars().next().filter(|&c| is_rust_ident_continue(c) && pos < end) {
            pos += c.len_utf8();
        }
        let word = &text[s..pos];
        let mut next = pos;
        skip_trivia(text, &mut next);
        let after = &text[next..end.max(next)];
        let prefix = text[start..s].trim_end();
        // A path, a tuple-struct or struct pattern, a macro, or a field name (`Point { x: px }` binds `px`).
        let not_a_name = after.starts_with(['(', '{', '!', ':']) || prefix.ends_with("::") || prefix.ends_with('.');
        if !not_a_name && !word.starts_with(char::is_uppercase) && !NOT_BINDINGS.contains(&word) {
            out.push((s, pos));
        }
    }
    out
}

/// The byte ranges of the closure parameters between `start` and `end`, split at depth-0 commas (`<…>` counts).
fn closure_params(text: &str, start: usize, end: usize) -> Vec<(usize, usize)> {
    let b = text.as_bytes();
    let (mut out, mut depth, mut from) = (Vec::new(), 0i32, start);
    for p in start..end {
        match b[p] {
            b'(' | b'[' | b'{' | b'<' => depth += 1,
            b'>' if p > 0 && b[p - 1] == b'-' => {}
            b')' | b']' | b'}' | b'>' => depth -= 1,
            b',' if depth == 0 => {
                out.push((from, p));
                from = p + 1;
            }
            _ => {}
        }
    }
    out.push((from, end));
    out
}

/// Where the pattern of a parameter `text[start..end]` ends: its depth-0 `:` before the type.
fn before_type(text: &str, start: usize, end: usize) -> usize {
    let b = text.as_bytes();
    find_code(text, start, |p, d| p >= end || d == 0 && b[p] == b':' && b.get(p + 1) != Some(&b':') && (p == 0 || b[p - 1] != b':')).map_or(end, |p| p.min(end))
}

impl<'a> OverlayParser<'a> {
    /// Opens the pattern scopes that start by `pos` and closes the ones that ended, on `stack`.
    pub(super) fn sync_pattern_scopes(&mut self, pos: usize, stack: &mut Vec<usize>) {
        while let Some(&top) = stack.last() {
            if !(self.scopes[top].pattern && self.scopes[top].end_byte <= pos) {
                break;
            }
            stack.pop();
        }
        while let Some(&Reverse((start, id))) = self.pending_patterns.peek() {
            if start > pos {
                break;
            }
            self.pending_patterns.pop();
            self.scopes[id].parent = stack.last().copied();
            stack.push(id);
        }
    }

    /// A pattern scope over `start..end` binding `names`, each seen from `visible_from`.
    fn add_pattern_scope(&mut self, start: usize, end: usize, visible_from: usize, names: Vec<(usize, usize)>) {
        if names.is_empty() || end <= start {
            return;
        }
        let id = self.scopes.len();
        self.scopes.push(Scope { id, parent: None, start_byte: start, end_byte: end, sel_start_byte: start, body_start_byte: None, pattern: true });
        self.pending_patterns.push(Reverse((start, id)));
        self.push_locals(names, id, visible_from);
    }

    /// Local symbols for `names` in scope `scope_id`, seen from `visible_from`.
    pub(super) fn push_locals(&mut self, names: Vec<(usize, usize)>, scope_id: usize, visible_from: usize) {
        for (s, e) in names {
            let (start_line, start_col) = self.byte_to_char_line_col(s);
            let (end_line, end_col) = self.byte_to_char_line_col(e);
            let name = self.text[s..e].to_string();
            self.symbols.push(OverlaySymbol {
                name,
                kind: DefKind::Local,
                signature: None,
                start_line,
                start_col,
                end_line,
                end_col,
                scope_id,
                decl_start_byte: s,
                decl_end_byte: e,
                scope_start_byte: visible_from,
            });
        }
    }

    /// A closure opening with the `|` at `open`: its parameters, seen in its body. Returns the byte after its
    /// parameter list, or `None` when that `|` is an operator.
    pub(super) fn parse_closure(&mut self, open: usize) -> Option<usize> {
        let text = self.text;
        let b = text.as_bytes();
        // The code before the `|`, past whitespace and the comments the parser recorded (`= /* c */ |v|`).
        let mut cut = open;
        loop {
            cut = text[..cut].trim_end().len();
            match self.opaque.iter().rev().find(|&&(s, e)| s < cut && cut <= e && text[s..].starts_with('/')) {
                Some(&(s, _)) => cut = s,
                None => break,
            }
        }
        let prefix = &text[..cut];
        let starts_expr = prefix.is_empty()
            || prefix.ends_with(['(', ',', '{', ';', '[', '=', '&'])
            || prefix.ends_with("=>")
            || prefix.ends_with(':') && !prefix.ends_with("::")
            || ["move", "return", "mut", "async", "static"].iter().any(|w| prefix.ends_with(w) && word_at(text, prefix.len() - w.len(), w));
        let operator = prefix.ends_with("==") || prefix.ends_with("!=") || prefix.ends_with("<=") || prefix.ends_with(">=") || prefix.ends_with("&&") || prefix.ends_with("&=");
        if !starts_expr || operator {
            return None;
        }
        if b.get(open + 1) == Some(&b'|') {
            return Some(open + 2);
        }
        // Parameters end at the next `|`; reaching `=>` first, the `|` led an or-pattern (`| Some(v) => …`).
        let close = find_code(text, open + 1, |p, d| d == 0 && (matches!(b[p], b'|' | b';') || text[p..].starts_with("=>"))).filter(|&p| b[p] == b'|')?;
        let mut body = close + 1;
        skip_trivia(text, &mut body);
        let end = if text[body..].starts_with("->") { body_brace(text, body).map_or(text.len(), |p| block_end(text, p)) } else { expr_end(text, body) };
        let names = closure_params(text, open + 1, close).into_iter().flat_map(|(s, e)| pattern_bindings(text, s, before_type(text, s, e))).collect();
        self.add_pattern_scope(open, end, open, names);
        Some(close + 1)
    }

    /// `for PAT in EXPR { … }` after the `for` at `pos`: what `PAT` binds, seen in the loop body.
    pub(super) fn parse_for(&mut self, pos: usize) {
        let text = self.text;
        let b = text.as_bytes();
        let mut p = pos;
        skip_trivia(text, &mut p);
        if text[p..].starts_with('<') {
            return; // `for<'a>` bounds
        }
        let Some(in_at) = find_code(text, p, |q, d| d == 0 && (matches!(b[q], b'{' | b';') || word_at(text, q, "in"))).filter(|&q| b[q] == b'i') else {
            return; // `impl Trait for Type`
        };
        let Some(brace) = header_body_brace(text, in_at + 2, true) else { return };
        let names = pattern_bindings(text, p, in_at);
        self.add_pattern_scope(in_at, block_end(text, brace), brace, names);
    }

    /// `if let PAT = EXPR { … }` / `while let` after the `let` at `pos`: what `PAT` binds, seen in that block.
    pub(super) fn parse_conditional_let(&mut self, pos: usize) {
        let text = self.text;
        let b = text.as_bytes();
        let is_eq = |q: usize| b[q] == b'=' && !matches!(b.get(q + 1), Some(b'=' | b'>')) && !matches!(b[q - 1], b'=' | b'!' | b'<' | b'>');
        let Some(eq) = find_code(text, pos, |q, d| d == 0 && (matches!(b[q], b'{' | b';') || is_eq(q))).filter(|&q| b[q] == b'=') else { return };
        let Some(brace) = header_body_brace(text, eq + 1, true) else { return };
        let names = pattern_bindings(text, pos, eq);
        // In a let chain (`if let Some(v) = opt && v > 0 {`) the names are seen from the `&&` on.
        let chained = find_code(text, eq + 1, |q, d| q >= brace || d == 0 && text[q..].starts_with("&&")).filter(|&q| q < brace);
        self.add_pattern_scope(eq, block_end(text, brace), chained.unwrap_or(brace), names);
    }

    /// The arms of the `match` at `pos`: what each pattern binds, seen in its guard and its expression.
    pub(super) fn parse_match(&mut self, pos: usize) {
        let text = self.text;
        let b = text.as_bytes();
        let Some(open) = header_body_brace(text, pos, false) else { return };
        let mut cur = open + 1;
        for _ in 0..10_000 {
            skip_trivia(text, &mut cur);
            let Some(arrow) = find_code(text, cur, |q, d| d == 0 && text[q..].starts_with("=>")) else { return };
            let pat_end = find_code(text, cur, |q, d| q >= arrow || d == 0 && word_at(text, q, "if")).map_or(arrow, |q| q.min(arrow));
            let mut body = arrow + 2;
            skip_trivia(text, &mut body);
            let end = arm_end(text, body);
            let names = pattern_bindings(text, cur, pat_end);
            self.add_pattern_scope(cur, end, pat_end, names);
            cur = end.max(arrow + 2);
            skip_trivia(text, &mut cur);
            match b.get(cur) {
                Some(b',') => cur += 1,
                Some(b'}') | None => return,
                _ => {}
            }
        }
    }

    /// The names a destructuring `let` pattern starting at `pos` binds (`let (a, b) = …`, `let Some(x) = … else`), and
    /// where the pattern ends; `None` for a plain `let name`.
    pub(super) fn let_pattern(&self, pos: usize) -> Option<(Vec<(usize, usize)>, usize)> {
        let text = self.text;
        let b = text.as_bytes();
        let first = text[pos..].chars().next()?;
        if !is_rust_ident_start(first) && !matches!(first, '(' | '[' | '&') {
            return None; // not a pattern: no binding at all
        }
        // A lone name (`let v = …`, `let Name: T`) stays a plain binding, whatever its case.
        if is_rust_ident_start(first) {
            let mut q = pos + text[pos..].find(|c: char| !is_rust_ident_continue(c)).unwrap_or(text.len() - pos);
            skip_trivia(text, &mut q);
            if !text[q..].starts_with(['(', '{', ':', '@']) || text[q..].starts_with(':') && !text[q..].starts_with("::") {
                return None;
            }
        }
        let end = find_code(text, pos, |q, d| d == 0 && (matches!(b[q], b'=' | b';') || b[q] == b':' && b.get(q + 1) != Some(&b':') && b[q - 1] != b':'))?;
        Some((pattern_bindings(text, pos, end), end))
    }
}
