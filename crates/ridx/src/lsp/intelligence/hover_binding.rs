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
//! `Type: bool`, with no layout and no range).

use serde_json::{json, Value};

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The name at 1-based char column `col` of `line`: its chars (0-based, end exclusive) and its block.
fn binding_at(line: &str, col: u32, ty: &str) -> Option<(usize, usize, String)> {
    let chars: Vec<(usize, char)> = line.char_indices().collect();
    let at = (col.saturating_sub(1) as usize).min(chars.len().checked_sub(1)?);
    if !is_ident(chars[at].1) {
        return None;
    }
    let start = chars[..at].iter().rposition(|&(_, c)| !is_ident(c)).map_or(0, |p| p + 1);
    let end = chars[at..].iter().position(|&(_, c)| !is_ident(c)).map_or(chars.len(), |p| at + p);
    let (b0, b1) = (chars[start].0, chars.get(end).map_or(line.len(), |&(b, _)| b));
    let text = super::hover_render::binding_text(line, start as u32 + 1, &line[b0..b1], ty)?;
    Some((start, end, format!("```rust\n{text}\n```")))
}

/// Whether the name at chars `start..end` of `line` reads as a binding other than a `let`'s: a parameter (`on: bool`),
/// a name in a pattern or among a closure's parameters (`Some(v) =>`, `|a, b|`, `Point { x, y }`, `for x in`).
fn reads_as_binding(line: &[char], start: usize, end: usize) -> bool {
    let after: String = line[end..].iter().collect();
    let after = after.trim_start();
    if after.starts_with(':') && !after.starts_with("::") {
        return true;
    }
    let before: String = line[..start].iter().collect();
    let before = before.trim_end();
    let word_before = |w: &str| before.strip_suffix(w).is_some_and(|rest| !rest.ends_with(is_ident));
    let before = if word_before("mut") { before[..before.len() - 3].trim_end() } else { before };
    let opens = before.ends_with(['(', ',', '|', '{', '&', '[']) || before.is_empty() || {
        let b = before;
        b.strip_suffix("ref").is_some_and(|rest| !rest.ends_with(is_ident))
    };
    let closes = after.is_empty() || after.starts_with([',', ')', '|', '}', ']', '@']) || after.starts_with("=>") || (after.starts_with('=') && !after.starts_with("=="));
    let for_in = before.strip_suffix("for").is_some_and(|rest| !rest.ends_with(is_ident)) && after.strip_prefix("in").is_some_and(|rest| !rest.starts_with(is_ident));
    (opens && closes) || for_in
}

/// The hover of the binding named at 1-based char column `col` of `line` (0-based line `line_0`), when the index has
/// its type `ty` and no definition there: `None` unless the name reads as a binding. `layout_at` gives the layout note
/// recorded at a 1-based column (the binding's pattern starts at its name or at its `mut`).
pub(crate) fn binding_hover(line: &str, line_0: u32, col: u32, ty: &str, layout_at: &dyn Fn(u32) -> Option<String>) -> Option<Value> {
    let (start, end, block) = binding_at(line, col, ty)?;
    let chars: Vec<char> = line.chars().collect();
    if !block.starts_with("```rust\nlet ") && !reads_as_binding(&chars, start, end) {
        return None;
    }
    let at = start as u32 + 1;
    let value = match layout_at(at).or_else(|| (at > 4).then(|| layout_at(at - 4)).flatten()) {
        Some(layout) => format!("{block}\n\n---\n\n{layout}"),
        None => block,
    };
    let utf16 = |c: usize| chars[..c].iter().map(|ch| ch.len_utf16()).sum::<usize>();
    Some(json!({
        "contents": { "kind": "markdown", "value": value },
        "range": { "start": { "line": line_0, "character": utf16(start) }, "end": { "line": line_0, "character": utf16(end) } }
    }))
}

/// The `let [mut] name: Type` block for the `let` binding at `col` of `line`.
#[cfg(test)]
pub(crate) fn let_binding_hover(line: &str, col: u32, ty: &str) -> Option<String> {
    let (_, _, block) = binding_at(line, col, ty)?;
    block.starts_with("```rust\nlet ").then_some(block)
}

/// [`binding_hover`]'s text for a `let` binding.
#[cfg(test)]
pub(crate) fn let_binding_hover_with_layout(line: &str, col: u32, ty: &str, layout_at: &dyn Fn(u32) -> Option<String>) -> Option<String> {
    let_binding_hover(line, col, ty)?;
    binding_hover(line, 0, col, ty, layout_at).and_then(|v| v["contents"]["value"].as_str().map(str::to_string))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(text: &'static str, at: u32) -> impl Fn(u32) -> Option<String> {
        move |c| (c == at).then(|| text.to_string())
    }

    /// prod-code, m=20120: the parameter `on` of `fn gap(a: Guard, on: bool)` showed `Type: bool`.
    #[test]
    fn test_a_parameter_shows_its_name_type_layout_and_range() {
        let line = "fn gap(a: Guard, on: bool) {}";
        let h = binding_hover(line, 3, 18, "bool", &layout("size = 1, align = 0x1, no Drop", 18)).unwrap();
        assert_eq!(h["contents"]["value"], "```rust\non: bool\n```\n\n---\n\nsize = 1, align = 0x1, no Drop");
        assert_eq!(h["range"], json!({ "start": { "line": 3, "character": 17 }, "end": { "line": 3, "character": 19 } }));
        let a = binding_hover(line, 3, 8, "app::Guard", &layout("size = 24 (0x18), align = 0x8, needs Drop", 8)).unwrap();
        assert_eq!(a["contents"]["value"], "```rust\na: Guard\n```\n\n---\n\nsize = 24 (0x18), align = 0x8, needs Drop");
        let m = binding_hover("fn f(mut n: u8) {}", 0, 10, "u8", &layout("size = 1, align = 0x1, no Drop", 6)).unwrap();
        assert_eq!(m["contents"]["value"], "```rust\nmut n: u8\n```\n\n---\n\nsize = 1, align = 0x1, no Drop", "the pattern starts at its `mut`");
    }

    #[test]
    fn test_pattern_and_closure_bindings_read_as_bindings_and_other_names_do_not() {
        let none = |_: u32| None;
        let text = |line: &str, col: u32| binding_hover(line, 0, col, "u32", &none).map(|v| v["contents"]["value"].as_str().unwrap().to_string());
        assert_eq!(text("    Some(v) => v + 1,", 10).as_deref(), Some("```rust\nv: u32\n```"));
        assert_eq!(text("    let f = |a, b| a + b;", 17).as_deref(), Some("```rust\nb: u32\n```"));
        assert_eq!(text("    let Point { x, y } = p;", 20).as_deref(), Some("```rust\ny: u32\n```"));
        assert_eq!(text("    for item in items {}", 10).as_deref(), Some("```rust\nitem: u32\n```"));
        assert_eq!(text("    let total = 0;", 9).as_deref(), Some("```rust\nlet total: u32\n```"));
        assert_eq!(text("    total.count()", 11), None, "a method");
        assert_eq!(text("    a == b", 5), None, "a comparison");
        assert_eq!(text("    std::mem::take(x)", 10), None, "a path");
    }
}
