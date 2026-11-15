/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Type hints after bindings, where rust-analyzer shows them: `let` (statements without a type, `if let`,
//! `while let`), `for` patterns, `match` arms and untyped closure parameters.

use super::{Hint, SourceMap, TypeSpan};

/// Type hints for the bindings in `masked` (comments and strings blanked) on lines `lines` (0-based, inclusive);
/// `live_type` names the type of a binding (its byte range) the index has none for.
pub fn binding_hints(src: &SourceMap<'_>, types: &[TypeSpan<'_>], lines: (u32, u32), live_type: &dyn Fn(usize, usize) -> Option<String>) -> Vec<Hint> {
    let mut regions: Vec<(usize, usize)> = Vec::new();
    let m = src.masked.as_bytes();
    for at in keyword_positions(src.masked, "let") {
        if let Some(end) = scan_to(m, at + 3, b"=", true) {
            regions.push((at + 3, end));
        }
    }
    for at in keyword_positions(src.masked, "for") {
        let rest = src.masked[at + 3..].trim_start();
        if rest.starts_with('<') {
            continue;
        }
        if let Some(in_at) = find_in_keyword(src.masked, at + 3) {
            regions.push((at + 3, in_at));
        }
    }
    regions.extend(match_arm_patterns(src.masked));
    regions.extend(closure_params(src.masked));

    let mut out = Vec::new();
    for (s, e) in regions {
        let (line0, _) = src.line_col(s);
        if line0 > lines.1 || src.line_col(e).0 < lines.0 {
            continue;
        }
        for (b_start, b_end) in binding_idents(src.masked, s, e) {
            let (l0, c0) = src.line_col(b_start);
            if l0 < lines.0 || l0 > lines.1 {
                continue;
            }
            let (_, e0) = src.line_col(b_end);
            let live;
            let ty = match binding_type(types, l0 + 1, c0 + 1, e0 + 1) {
                Some(ty) => ty,
                None => match live_type(b_start, b_end) {
                    Some(t) => {
                        live = t;
                        live.as_str()
                    }
                    None => continue,
                },
            };
            let Some(label) = super::type_label::hint_type(ty) else { continue };
            out.push(Hint::new(src, b_end, format!(": {label}"), Some(1)));
        }
    }
    out
}

/// The type recorded for the binding spanning 1-based chars `start..end` on `line` (or a `mut`/`ref` binding
/// whose span starts earlier on that line and ends with the name).
fn binding_type<'a>(types: &[TypeSpan<'a>], line: u32, start: u32, end: u32) -> Option<&'a str> {
    types
        .iter()
        .filter(|t| t.start.0 == line && t.end.0 == line && (t.end.1 == end || t.end.1 + 1 == end) && t.start.1 <= start && start - t.start.1 <= 8)
        .max_by_key(|t| t.start.1)
        .map(|t| t.ty)
}

/// Byte offsets of `kw` as a whole word in `text`.
pub(super) fn keyword_positions<'a>(text: &'a str, kw: &'a str) -> impl Iterator<Item = usize> + 'a {
    let b = text.as_bytes();
    text.match_indices(kw).map(|(i, _)| i).filter(move |&i| {
        let before_ok = i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_' || b[i - 1] == b'\'');
        let after = i + kw.len();
        let after_ok = after >= b.len() || !(b[after].is_ascii_alphanumeric() || b[after] == b'_');
        before_ok && after_ok
    })
}

/// The offset of the first `stop` byte at bracket depth 0 from `from` (`=` not part of `==`, `=>`, `<=`, ...);
/// `None` when a `;`, `{` or a `:` type ascription comes first (a typed `let` gets no hints).
fn scan_to(b: &[u8], from: usize, stop: &[u8], typed_is_none: bool) -> Option<usize> {
    let mut depth = 0i32;
    let mut i = from;
    while i < b.len() {
        let c = b[i];
        match c {
            b'(' | b'[' | b'{' if c == b'{' && depth == 0 => return None,
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                depth -= 1;
                if depth < 0 {
                    return None;
                }
            }
            b';' if depth == 0 => return None,
            b':' if depth == 0 && typed_is_none && b.get(i + 1) != Some(&b':') && (i == 0 || b[i - 1] != b':') => return None,
            b'=' if depth == 0 && stop.contains(&b'=') => {
                let next = b.get(i + 1).copied();
                let prev = if i > 0 { b[i - 1] } else { 0 };
                if next != Some(b'=') && next != Some(b'>') && !matches!(prev, b'=' | b'!' | b'<' | b'>') {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// The offset of the `in` of a `for` loop starting after `from`, if one comes before its `{`.
fn find_in_keyword(text: &str, from: usize) -> Option<usize> {
    let b = text.as_bytes();
    let mut depth = 0i32;
    let mut i = from;
    while i + 2 < b.len() {
        match b[i] {
            b'(' | b'[' => depth += 1,
            b')' | b']' => depth -= 1,
            b'{' | b';' if depth == 0 => return None,
            b'i' if depth == 0 && b[i + 1] == b'n' && b[i - 1].is_ascii_whitespace() && b[i + 2].is_ascii_whitespace() => return Some(i),
            _ => {}
        }
        i += 1;
    }
    None
}

/// The pattern before each `=>` of a `match`: back to the arm's start, cut at a guard's `if`. Macro arms (with
/// `$`) are skipped.
fn match_arm_patterns(text: &str) -> Vec<(usize, usize)> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    for (arrow, _) in text.match_indices("=>") {
        let mut depth = 0i32;
        let mut i = arrow;
        let mut seen_pattern = false;
        while i > 0 {
            i -= 1;
            let c = b[i];
            if c.is_ascii_whitespace() {
                continue;
            }
            match c {
                // A `}` right before `=>` closes a struct pattern; elsewhere at depth 0 it ends the previous arm.
                b'}' if depth == 0 && seen_pattern => break,
                b')' | b']' | b'}' => depth += 1,
                b'(' | b'[' | b'{' if depth > 0 => depth -= 1,
                b'{' | b',' if depth == 0 => break,
                _ => {}
            }
            seen_pattern = true;
        }
        let start = i + 1;
        let mut end = arrow;
        let pat = &text[start..arrow];
        if pat.contains('$') {
            continue;
        }
        if let Some(g) = keyword_positions(pat, "if").next() {
            end = start + g;
        }
        out.push((start, end));
    }
    out
}

/// The parameter lists of closures (`|a, b|`), without the parameters written with a type.
fn closure_params(text: &str) -> Vec<(usize, usize)> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'|' && b.get(i + 1) != Some(&b'|') && (i == 0 || b[i - 1] != b'|') {
            let prev = text[..i].trim_end();
            let opens = prev.is_empty() || prev.ends_with(['(', ',', '=', '{', ';', '[', ':']) || prev.ends_with("move") || prev.ends_with("return") || prev.ends_with("=>");
            if opens {
                if let Some(close) = text[i + 1..].find('|').map(|c| i + 1 + c) {
                    let mut seg_start = i + 1;
                    let mut depth = 0i32;
                    for j in i + 1..=close {
                        let c = b[j];
                        match c {
                            b'(' | b'[' | b'<' => depth += 1,
                            b')' | b']' | b'>' => depth -= 1,
                            _ => {}
                        }
                        if (c == b',' && depth == 0) || j == close {
                            let seg = &text[seg_start..j];
                            let typed = seg.match_indices(':').any(|(k, _)| seg.as_bytes().get(k + 1) != Some(&b':') && (k == 0 || seg.as_bytes()[k - 1] != b':'));
                            if !typed {
                                out.push((seg_start, j));
                            }
                            seg_start = j + 1;
                        }
                    }
                    i = close + 1;
                    continue;
                }
            }
        }
        i += 1;
    }
    out
}

/// The binding identifiers in the pattern `text[s..e]`: lowercase names that are not keywords, not paths, not
/// constructors or struct field labels.
fn binding_idents(text: &str, s: usize, e: usize) -> Vec<(usize, usize)> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = s;
    while i < e {
        let c = b[i];
        if c.is_ascii_alphabetic() || c == b'_' {
            let start = i;
            while i < e && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            let word = &text[start..i];
            let prev = text[s..start].trim_end();
            let next = text[i..e].trim_start();
            let is_keyword = matches!(word, "mut" | "ref" | "box" | "in" | "if" | "let" | "_" | "true" | "false" | "self" | "Self" | "crate" | "super");
            let is_path = prev.ends_with("::") || prev.ends_with('.') || prev.ends_with('\'') || next.starts_with("::");
            let is_ctor = next.starts_with('(') || next.starts_with('{') || next.starts_with('!');
            let is_label = next.starts_with(':') && !next.starts_with("::");
            let upper = word.as_bytes()[0].is_ascii_uppercase();
            let numeric_suffix = start > 0 && b[start - 1].is_ascii_digit();
            if !(is_keyword || is_path || is_ctor || is_label || upper || numeric_suffix) {
                out.push((start, i));
            }
        } else {
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pattern_regions_and_bindings() {
        let t = "let (a, mut b) = f(); if let Some(x) = o {} for (i, item) in v {} match m { Foo { k, v: w } => 1, Bar(y) if y > 0 => 2, _ => 3 } let z: u8 = 1; c(|p, q: u8| p);";
        let names = |regions: Vec<(usize, usize)>| -> Vec<String> { regions.iter().flat_map(|&(s, e)| binding_idents(t, s, e)).map(|(s, e)| t[s..e].to_string()).collect() };
        let lets: Vec<(usize, usize)> = keyword_positions(t, "let").filter_map(|at| scan_to(t.as_bytes(), at + 3, b"=", true).map(|e| (at + 3, e))).collect();
        assert_eq!(names(lets), ["a", "b", "x"]);
        let fors: Vec<(usize, usize)> = keyword_positions(t, "for").filter_map(|at| find_in_keyword(t, at + 3).map(|e| (at + 3, e))).collect();
        assert_eq!(names(fors), ["i", "item"]);
        assert_eq!(names(match_arm_patterns(t)), ["k", "w", "y"]);
        assert_eq!(names(closure_params(t)), ["p"]);
    }
}
