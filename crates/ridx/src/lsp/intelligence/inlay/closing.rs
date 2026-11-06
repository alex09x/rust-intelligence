/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Closing-brace hints: after the `}` ending a block of at least 25 lines, what the block belongs to (`fn name`,
//! `impl Trait for Type`, `match expr`, `name!`, ...), as rust-analyzer labels it.

use super::{Hint, SourceMap};

/// rust-analyzer's default `inlayHints.closingBraceHints.minLines`.
const MIN_LINES: u32 = 25;

pub fn closing_brace_hints(src: &SourceMap<'_>, lines: (u32, u32)) -> Vec<Hint> {
    let b = src.masked.as_bytes();
    let mut stack: Vec<usize> = Vec::new();
    let mut out = Vec::new();
    for (i, &c) in b.iter().enumerate() {
        match c {
            b'{' => stack.push(i),
            b'}' => {
                let Some(open) = stack.pop() else { continue };
                let (close_line, _) = src.line_col(i);
                if close_line < lines.0 || close_line > lines.1 {
                    continue;
                }
                // Only a `}` ending its line, possibly followed by `;` or `,`.
                let rest = &src.masked[i + 1..];
                let after = rest.split('\n').next().unwrap_or("");
                let trimmed = after.trim_end();
                let hint_at = match trimmed {
                    "" => i + 1,
                    ";" | "," => i + 1 + after.find([';', ',']).unwrap_or(0) + 1,
                    _ => continue,
                };
                let (open_line, _) = src.line_col(open);
                if close_line - open_line + 1 < MIN_LINES {
                    continue;
                }
                let Some(label) = block_label(src, open) else { continue };
                let mut h = Hint::new(src, hint_at, label, None);
                h.padding_left = true;
                out.push(h);
            }
            _ => {}
        }
    }
    out
}

/// The label for the block opening at `open`: from the header text before it back to the previous statement.
fn block_label(src: &SourceMap<'_>, open: usize) -> Option<String> {
    let b = src.masked.as_bytes();
    let mut depth = 0i32;
    let mut i = open;
    while i > 0 {
        i -= 1;
        match b[i] {
            b')' | b']' => depth += 1,
            b'(' | b'[' => depth -= 1,
            b'{' | b'}' | b';' if depth == 0 => {
                i += 1;
                break;
            }
            _ => {}
        }
    }
    // From the masked text: a comment before the header must not join it.
    let header: String = src.masked[i..open].split_whitespace().collect::<Vec<_>>().join(" ");
    let header = strip_attributes(&header);
    label_for_header(&header)
}

/// `header` without leading `#[...]` attributes.
fn strip_attributes(header: &str) -> String {
    let mut h = header.trim();
    while let Some(rest) = h.strip_prefix("#[").or_else(|| h.strip_prefix("#![")) {
        let mut depth = 1;
        let mut end = rest.len();
        for (j, c) in rest.char_indices() {
            match c {
                '[' => depth += 1,
                ']' => {
                    depth -= 1;
                    if depth == 0 {
                        end = j + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        h = rest[end..].trim_start();
    }
    h.to_string()
}

fn label_for_header(header: &str) -> Option<String> {
    let words: Vec<&str> = header.split(' ').collect();
    let skip_vis = |w: &[&str]| -> usize {
        match w.first() {
            Some(&"pub") => 1,
            Some(v) if v.starts_with("pub(") => 1,
            _ => 0,
        }
    };
    let rest = &words[skip_vis(&words)..];
    let joined = rest.join(" ");
    if let Some(at) = rest.iter().position(|w| *w == "fn") {
        let name = rest.get(at + 1)?.split(['<', '(']).next()?;
        let quals: Vec<&str> = rest[..at].iter().copied().filter(|q| matches!(*q, "unsafe" | "async" | "const")).collect();
        let q = if quals.is_empty() { String::new() } else { format!("{} ", quals.join(" ")) };
        return Some(format!("{q}fn {name}"));
    }
    let is_impl = |w: Option<&&str>| w.is_some_and(|w| *w == "impl" || w.starts_with("impl<"));
    if is_impl(rest.first()) || (rest.first() == Some(&"unsafe") && is_impl(rest.get(1))) {
        return Some(impl_label(&joined));
    }
    if let Some(at) = rest.iter().position(|w| *w == "trait") {
        let name = rest.get(at + 1)?.split(['<', ':']).next()?;
        return Some(format!("trait {name}"));
    }
    if rest.first() == Some(&"mod") {
        return Some(format!("mod {}", rest.get(1)?));
    }
    if let Some(name) = joined.strip_suffix('!').or_else(|| joined.split_once("! ").map(|(n, _)| n)) {
        if !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == ':') {
            return Some(format!("{name}!"));
        }
    }
    let last_kw = |kw: &str| -> Option<usize> { rest.iter().rposition(|w| *w == kw) };
    if let Some(label) = rest.first().filter(|w| w.starts_with('\'') && w.ends_with(':')) {
        return Some(label.trim_end_matches(':').to_string());
    }
    if rest.last() == Some(&"else") {
        return Some("else".to_string());
    }
    for kw in ["match", "for", "while", "if"] {
        if let Some(at) = last_kw(kw) {
            return Some(truncate_words(kw, &rest[at + 1..]));
        }
    }
    match rest.last() {
        Some(&"loop") => Some("loop".to_string()),
        Some(&"unsafe") => Some("unsafe".to_string()),
        Some(&"async") | Some(&"move") if rest.contains(&"async") => Some("async".to_string()),
        _ => None,
    }
}

/// `impl Trait for Type` (the trait by its name alone) or `impl Type`, generics of the impl dropped.
fn impl_label(header: &str) -> String {
    let h = header.trim_start_matches("unsafe ").trim_start_matches("impl");
    let h = if h.starts_with('<') { skip_generics(h) } else { h }.trim();
    let h = h.split(" where ").next().unwrap_or(h).trim();
    let short = |s: &str| super::type_label::truncate(&super::type_label::short_paths(s.trim()), super::type_label::MAX_LENGTH);
    match h.split_once(" for ") {
        Some((tr, ty)) => {
            let tr = tr.trim().trim_start_matches('!');
            let name = tr.split('<').next().unwrap_or(tr).rsplit("::").next().unwrap_or(tr);
            format!("impl {name} for {}", short(ty))
        }
        None => format!("impl {}", short(h)),
    }
}

/// `text` past the generic list it starts with.
fn skip_generics(text: &str) -> &str {
    let mut depth = 0i32;
    for (i, c) in text.char_indices() {
        match c {
            '<' => depth += 1,
            '>' if i > 0 && text.as_bytes()[i - 1] == b'-' => {}
            '>' => {
                depth -= 1;
                if depth == 0 {
                    return &text[i + 1..];
                }
            }
            _ => {}
        }
    }
    text
}

/// `kw` and the words after it while they fit in 25 characters, then `…` when any were left out.
fn truncate_words(kw: &str, words: &[&str]) -> String {
    let mut text = String::new();
    let mut cut = false;
    for w in words {
        let next = if text.is_empty() { w.to_string() } else { format!("{text} {w}") };
        if next.chars().count() > super::type_label::MAX_LENGTH {
            cut = true;
            break;
        }
        text = next;
    }
    match (text.is_empty(), cut) {
        (true, true) => format!("{kw} …"),
        (_, true) => format!("{kw} {text}…"),
        (true, false) => kw.to_string(),
        (false, false) => format!("{kw} {text}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_header_labels() {
        assert_eq!(label_for_header("pub fn visit_type<'a>(&mut self, ty: &Type)").as_deref(), Some("fn visit_type"));
        assert_eq!(label_for_header("pub(crate) unsafe fn collect_ty_garbage()").as_deref(), Some("unsafe fn collect_ty_garbage"));
        assert_eq!(label_for_header("impl<'a> ser::Serializer for &mut Formatter<'a>").as_deref(), Some("impl Serializer for &mut Formatter<'a>"));
        assert_eq!(label_for_header("impl<'ast> FindTyParams<'ast>").as_deref(), Some("impl FindTyParams<'ast>"));
        assert_eq!(label_for_header("pub trait Serialize").as_deref(), Some("trait Serialize"));
        assert_eq!(label_for_header("mod range").as_deref(), Some("mod range"));
        assert_eq!(label_for_header("quote_block!").as_deref(), Some("quote_block!"));
        assert_eq!(label_for_header("match effective_style(variant)").as_deref(), Some("match effective_style(variant)"));
        assert_eq!(label_for_header("for attr in &item.attrs").as_deref(), Some("for attr in &item.attrs"));
        assert_eq!(label_for_header("if module_crate_root == SomeVeryLongNameHere").as_deref(), Some("if module_crate_root ==…"));
        assert_eq!(label_for_header("let x = loop").as_deref(), Some("loop"));
        assert_eq!(label_for_header("} else").as_deref(), Some("else"));
    }
}
