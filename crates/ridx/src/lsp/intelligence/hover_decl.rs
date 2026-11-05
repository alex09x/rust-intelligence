/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Declarations as rust-analyzer's hover prints them, cut from the item's source: the header without its body, a
//! `where` clause one bound per line, struct fields and enum variants listed (first five), a field or a variant on
//! its own.

use super::hover_generics::with_where_layout;

/// How many fields or variants a struct or enum hover lists before `/* … */`, as rust-analyzer's default.
const SHOWN_MEMBERS: usize = 5;

/// Byte offset where 1-based `line` starts in `src`.
pub(crate) fn line_start(src: &str, line: u32) -> Option<usize> {
    if line == 0 {
        return None;
    }
    if line == 1 {
        return Some(0);
    }
    src.match_indices('\n').nth(line as usize - 2).map(|(i, _)| i + 1)
}

/// Skips whitespace, comments and outer attributes from `at`; returns where the item's own tokens start.
pub(crate) fn skip_trivia(src: &str, mut at: usize) -> usize {
    let b = src.as_bytes();
    loop {
        while at < b.len() && b[at].is_ascii_whitespace() {
            at += 1;
        }
        if src[at..].starts_with("//") {
            at = src[at..].find('\n').map_or(src.len(), |e| at + e + 1);
        } else if src[at..].starts_with("/*") {
            at = src[at + 2..].find("*/").map_or(src.len(), |e| at + 2 + e + 2);
        } else if src[at..].starts_with("#[") || src[at..].starts_with("#![") {
            at = matching_close(src, at + src[at..].find('[').unwrap_or(0)).map_or(src.len(), |e| e + 1);
        } else {
            return at;
        }
    }
}

/// Index of the bracket closing the one at `open`, skipping strings, chars and nested brackets.
fn matching_close(src: &str, open: usize) -> Option<usize> {
    let b = src.as_bytes();
    let mut depth = 0i32;
    let mut i = open;
    while i < b.len() {
        if let Some(end) = comment_end(b, i) {
            i = end + 1;
            continue;
        }
        match b[i] {
            b'"' => i = skip_string(b, i),
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// When a comment starts at `i`, the index of its last byte.
fn comment_end(b: &[u8], i: usize) -> Option<usize> {
    match (b[i], b.get(i + 1)) {
        (b'/', Some(b'/')) => Some(b[i..].iter().position(|&c| c == b'\n').map_or(b.len() - 1, |e| i + e)),
        (b'/', Some(b'*')) => Some(b[i + 2..].windows(2).position(|w| w == b"*/").map_or(b.len() - 1, |e| i + 2 + e + 1)),
        _ => None,
    }
}

fn skip_string(b: &[u8], open: usize) -> usize {
    let mut i = open + 1;
    while i < b.len() && b[i] != b'"' {
        i += if b[i] == b'\\' { 2 } else { 1 };
    }
    i
}

/// The item's header from `start` up to its body: the first `{` or `;` outside brackets and generics. Returns the
/// header and the offset of that delimiter.
pub(crate) fn header_end(src: &str, start: usize) -> (usize, Option<u8>) {
    let b = src.as_bytes();
    let (mut paren, mut angle) = (0i32, 0i32);
    let mut i = start;
    while i < b.len() {
        match b[i] {
            b'"' => i = skip_string(b, i),
            b'/' if b.get(i + 1) == Some(&b'/') => i = src[i..].find('\n').map_or(b.len(), |e| i + e),
            b'-' if b.get(i + 1) == Some(&b'>') => i += 1,
            b'=' if b.get(i + 1) == Some(&b'>') => i += 1,
            b'(' | b'[' => paren += 1,
            b')' | b']' => paren -= 1,
            b'<' => angle += 1,
            b'>' if angle > 0 => angle -= 1,
            b'{' | b';' if paren == 0 && angle == 0 => return (i, Some(b[i])),
            b'{' => paren += 1,
            b'}' => paren -= 1,
            _ => {}
        }
        i += 1;
    }
    (b.len(), None)
}

/// Whitespace collapsed to single spaces, without the spaces just inside brackets or before commas, and without a
/// trailing comma before a closing bracket: `(\n    a: T,\n)` becomes `(a: T)`.
pub(crate) fn collapse(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        let code = strip_comment(line).trim();
        if code.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(code);
    }
    let mut s = without_attributes(&out).split_whitespace().collect::<Vec<_>>().join(" ");
    for (from, to) in [("[const] ", ""), ("~const ", "")] {
        s = s.replace(from, to);
    }
    for (from, to) in [("( ", "("), (" )", ")"), ("[ ", "["), (" ]", "]"), (" ,", ","), (",)", ")"), (",]", "]"), (", )", ")"), ("< ", "<"), (",>", ">"), (", >", ">")] {
        while s.contains(from) {
            s = s.replace(from, to);
        }
    }
    s
}

/// `text` without attributes (`#[unstable(…)]` on a generic parameter, `#[cfg(…)]` on an argument).
fn without_attributes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("#[") {
        out.push_str(&rest[..at]);
        match matching_close(rest, at + 1) {
            Some(close) => rest = &rest[close + 1..],
            None => return out + &rest[at..],
        }
    }
    out.push_str(rest);
    out
}

pub(crate) fn strip_comment(line: &str) -> &str {
    let b = line.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'"' => i = skip_string(b, i),
            b'/' if b.get(i + 1) == Some(&b'/') => return &line[..i],
            _ => {}
        }
        i += 1;
    }
    line
}

/// Splits `text` at commas outside brackets and generics.
pub(crate) fn split_top(text: &str) -> Vec<&str> {
    let b = text.as_bytes();
    let (mut depth, mut from) = (0i32, 0);
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if let Some(end) = comment_end(b, i) {
            i = end + 1;
            continue;
        }
        match b[i] {
            b'"' => i = skip_string(b, i),
            b'-' | b'=' if b.get(i + 1) == Some(&b'>') => i += 1,
            b'(' | b'[' | b'{' | b'<' => depth += 1,
            b')' | b']' | b'}' | b'>' => depth -= 1,
            b',' if depth == 0 => {
                out.push(&text[from..i]);
                from = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    out.push(&text[from..]);
    out.into_iter().filter(|p| !p.trim().is_empty()).collect()
}

/// The members of a `{ … }` body (fields or variants), each without attributes and comments.
fn members(body: &str) -> Vec<String> {
    split_top(body)
        .into_iter()
        .map(|m| {
            let at = skip_trivia(m, 0);
            collapse(&m[at..])
        })
        .filter(|m| !m.is_empty())
        .collect()
}

/// A struct, union or enum with its body as rust-analyzer lists it; `None` when the item cannot be read.
pub(crate) fn render_adt(src: &str, start: usize, is_enum: bool) -> Option<String> {
    let (end, delim) = header_end(src, start);
    let header = with_where_layout(&src[start..end]);
    if delim != Some(b'{') {
        return Some(header);
    }
    let close = matching_close(src, end)?;
    let items = members(&src[end + 1..close]);
    let sep = if header.contains("\nwhere\n") { "\n{" } else { " {" };
    if items.is_empty() {
        return Some(format!("{header}{sep}}}"));
    }
    let mut out = format!("{header}{sep}");
    for item in items.iter().take(SHOWN_MEMBERS) {
        let shown = if is_enum { variant_summary(item) } else { item.clone() };
        out.push_str(&format!("\n    {shown},"));
    }
    if items.len() > SHOWN_MEMBERS {
        out.push_str("\n    /* … */");
    }
    out.push_str("\n}");
    Some(out)
}

/// An enum variant inside its enum's hover: fields elided.
fn variant_summary(variant: &str) -> String {
    let name_end = variant.find(|c: char| !(c.is_alphanumeric() || c == '_')).unwrap_or(variant.len());
    let rest = variant[name_end..].trim_start();
    let name = &variant[..name_end];
    if rest.starts_with('{') {
        format!("{name} {{ /* … */ }}")
    } else if rest.starts_with('(') {
        format!("{name}(/* … */)")
    } else {
        variant.to_string()
    }
}

/// An enum variant on its own: `Name`, `Name(T)`, or `Name { pub f: T, }` as rust-analyzer prints it.
pub(crate) fn render_variant(src: &str, start: usize) -> Option<String> {
    let text = &src[start..];
    let name_end = text.find(|c: char| !(c.is_alphanumeric() || c == '_'))?;
    let name = &text[..name_end];
    let rest = text[name_end..].trim_start();
    let open = start + (text.len() - rest.len());
    if rest.starts_with('{') {
        let close = matching_close(src, open)?;
        let fields: Vec<String> = members(&src[open + 1..close]).into_iter().map(|f| if f.starts_with("pub") { f } else { format!("pub {f}") }).collect();
        return Some(format!("{name} {{ {}, }}", fields.join(", ")));
    }
    if rest.starts_with('(') {
        let close = matching_close(src, open)?;
        return Some(format!("{name}({})", members(&src[open + 1..close]).join(", ")));
    }
    let until = rest.find([',', '}', '\n']).unwrap_or(rest.len());
    let tail = rest[..until].trim();
    Some(if tail.starts_with('=') { format!("{name} {tail}") } else { name.to_string() })
}

/// A named field: `pub radius: f64`.
pub(crate) fn render_field(src: &str, start: usize) -> Option<String> {
    let text = &src[start..];
    let b = text.as_bytes();
    let mut depth = 0i32;
    for (i, &c) in b.iter().enumerate() {
        match c {
            b'(' | b'[' | b'<' | b'{' => depth += 1,
            b')' | b']' | b'>' | b'}' if depth > 0 => depth -= 1,
            b',' | b'}' if depth == 0 => {
                let field = collapse(&text[..i]);
                return field.contains(':').then_some(field);
            }
            _ => {}
        }
    }
    None
}

/// Type paths shortened to their last segment, as rust-analyzer prints types: `std::vec::Vec<shapes::Circle>`
/// becomes `Vec<Circle>`.
pub(crate) fn short_type(ty: &str) -> String {
    let mut out = String::with_capacity(ty.len());
    let mut word_start = 0;
    let chars: Vec<char> = ty.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == ':' && chars.get(i + 1) == Some(&':') && out.len() > word_start && chars.get(i + 2).is_some_and(|n| n.is_alphabetic() || *n == '_') {
            out.truncate(word_start);
            i += 2;
            continue;
        }
        if !(c.is_alphanumeric() || c == '_') {
            out.push(c);
            word_start = out.len();
        } else {
            out.push(c);
        }
        i += 1;
    }
    out
}

#[cfg(test)]
#[path = "tests/hover_decl_tests.rs"]
mod tests;
