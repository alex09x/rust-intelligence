/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Items an outer `#[cfg(…)]` leaves out of the build (`#[cfg(target_os = "netbsd")] mod impl_netbsd { … }`). The
//! index holds nothing for them; rust-analyzer resolves there only what the enclosing active code sees, so the names
//! such items define or import, method calls, fields and uses of local bindings are `unresolvedReference`.

use super::tokens::RawToken;
use super::tokens_gated::{retype_unresolved, starts_use, UnresolvedScope};
use std::collections::HashSet;

fn is_id(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// The item keyword a line starts with, after visibility and qualifiers (`pub(crate) unsafe fn` → `fn`).
fn item_keyword(line: &str) -> Option<&str> {
    let mut words = line.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).filter(|w| !w.is_empty());
    words
        .find(|w| !matches!(*w, "pub" | "crate" | "super" | "in" | "self" | "unsafe" | "async" | "extern" | "default"))
        .filter(|w| matches!(*w, "fn" | "mod" | "struct" | "enum" | "union" | "impl" | "trait" | "type" | "const" | "static" | "use" | "macro_rules"))
}

/// Line ranges `[attribute, item end]` of the items behind an outer `#[cfg(…)]` that `compiled` (the lines the index
/// has tokens on) never touches, each flagged when it is a `use` item. Statements, blocks, fields and arms behind a
/// `#[cfg]` are not items and are left alone, and so are `cfg(test)` items: the index may hold no test build.
pub(super) fn inactive_items(scans: &[&str], compiled: &HashSet<u32>) -> Vec<(usize, usize, bool)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < scans.len() {
        let t = scans[i].trim_start();
        if !t.starts_with("#[cfg(") || t.contains("test") {
            i += 1;
            continue;
        }
        let start = i;
        // The attribute and any attributes after it, up to the item's first line (which may be the attribute's own).
        let mut depth = 0i32;
        let mut j = i;
        let (item, item_text) = loop {
            let mut rest_from = None;
            for (p, c) in scans[j].bytes().enumerate() {
                match c {
                    b'[' => depth += 1,
                    b']' => {
                        depth -= 1;
                        if depth == 0 {
                            rest_from = Some(p + 1);
                        }
                    }
                    _ => {}
                }
            }
            let rest = rest_from.map_or("", |p| scans[j][p..].trim());
            if depth <= 0 && !rest.is_empty() && !rest.starts_with("#[") {
                break (j, rest);
            }
            let next = scans.get(j + 1).map(|s| s.trim_start());
            if depth <= 0 && !next.is_some_and(|n| n.starts_with("#[") || n.is_empty()) {
                break (j + 1, next.unwrap_or(""));
            }
            j += 1;
            if j >= scans.len() {
                return out;
            }
        };
        let Some(keyword) = item_keyword(item_text) else {
            i = item.max(i + 1);
            continue;
        };
        let Some(end) = item_end(scans, item) else {
            i += 1;
            continue;
        };
        if !(item..=end).any(|l| compiled.contains(&(l as u32))) {
            out.push((start, end, keyword == "use"));
        }
        i = end + 1;
    }
    out
}

/// The last line of the item starting at line `from`: its `;` or the brace that closes its body.
fn item_end(scans: &[&str], from: usize) -> Option<usize> {
    let (mut braces, mut parens) = (0i32, 0i32);
    // A field or match arm ends at its comma; an item's generics and where clauses hold commas of their own.
    let words: Vec<&str> = scans.get(from)?.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).collect();
    let item = words.iter().any(|w| matches!(*w, "fn" | "struct" | "enum" | "union" | "impl" | "trait" | "mod" | "type" | "use" | "const" | "static"));
    for (l, s) in scans.iter().enumerate().skip(from) {
        for c in s.bytes() {
            match c {
                b'(' | b'[' => parens += 1,
                b')' | b']' => parens -= 1,
                b'{' => braces += 1,
                b'}' => {
                    braces -= 1;
                    if braces <= 0 {
                        // `<0`: a field or arm, ended by its enclosing block.
                        return Some(l);
                    }
                }
                b',' if !item && braces == 0 && parens == 0 => return Some(l),
                b';' if braces == 0 && parens == 0 => return Some(l),
                _ => {}
            }
        }
    }
    None
}

/// Names lines in `mask` (or outside it, when `inside` is false) define as items, and names their `use` items import.
fn names(scans: &[&str], mask: &[bool], inside: bool) -> (HashSet<String>, HashSet<String>) {
    let (mut defined, mut imported) = (HashSet::new(), HashSet::new());
    let mut in_use = false;
    for (l, s) in scans.iter().enumerate() {
        if mask[l] != inside {
            in_use = false;
            continue;
        }
        let words: Vec<&str> = s.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '!')).filter(|w| !w.is_empty()).collect();
        for w in words.windows(2) {
            if matches!(w[0], "fn" | "mod" | "struct" | "enum" | "trait" | "type" | "const" | "static" | "union" | "macro_rules!") && w[1].bytes().all(is_id) {
                defined.insert(w[1].to_string());
            }
        }
        if in_use || starts_use(s) {
            in_use = !s.contains(';');
            let body = s.split_once("use ").map_or(*s, |(_, b)| b);
            let b = body.as_bytes();
            let mut i = 0;
            while i < b.len() {
                if is_id(b[i]) && (i == 0 || !is_id(b[i - 1])) {
                    let st = i;
                    while i < b.len() && is_id(b[i]) {
                        i += 1;
                    }
                    let next = body[i..].trim_start();
                    if !next.starts_with("::") && !matches!(&body[st..i], "self" | "super" | "crate" | "as") {
                        imported.insert(body[st..i].to_string());
                    }
                    continue;
                }
                i += 1;
            }
        }
    }
    (defined, imported)
}

/// Retypes `tokens` inside the inactive items of `text`. The file is parsed for its definition sites only when it has
/// such items.
pub(super) fn retype_inactive_items(text: &str, compiled: &HashSet<u32>, tokens: &mut Vec<RawToken>) {
    let masked = crate::lsp::editing::action_parse::mask_comments_and_strings(text);
    let scans: Vec<&str> = masked.lines().collect();
    let regions = inactive_items(&scans, compiled);
    if regions.is_empty() {
        return;
    }
    let (mut mask, mut items) = (vec![false; scans.len()], vec![false; scans.len()]);
    for &(s, e, is_use) in &regions {
        let e = e.min(scans.len() - 1);
        mask[s..=e].iter_mut().for_each(|m| *m = true);
        if !is_use {
            items[s..=e].iter_mut().for_each(|m| *m = true);
        }
    }
    // rust-analyzer still resolves the names a switched-off `use` imports; those an inactive item defines or imports
    // inside itself are gone.
    let (in_def, in_imp) = names(&scans, &items, true);
    let (out_def, out_imp) = names(&scans, &mask, false);
    let active = |n: &String| out_def.contains(n) || out_imp.contains(n);
    let defined: HashSet<String> = in_def.into_iter().filter(|n| !active(n)).collect();
    let all: HashSet<String> = defined.iter().cloned().chain(in_imp.into_iter().filter(|n| !active(n))).collect();
    let mut defs = Vec::new();
    super::tokens_detached::detached_definition_tokens(text, &mut defs);
    let region_defs: Vec<RawToken> = defs.iter().filter(|d| mask.get(d.line as usize).copied().unwrap_or(false)).copied().collect();
    retype_unresolved(text, &masked, &region_defs, tokens, &UnresolvedScope { names: all, use_line_names: Some(defined), lines: Some(&mask) });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_cfg_items_the_index_never_touched() {
        let scans = [
            "use std::io;",
            "#[cfg(target_os = \"linux\")]",
            "mod on {",
            "    fn a() {}",
            "}",
            "#[cfg(target_os = \"netbsd\")]",
            "#[allow(dead_code)]",
            "mod off {",
            "    fn b() {}",
            "}",
            "#[cfg(windows)] use x::y;",
            "#[cfg(test)]",
            "mod tests {}",
        ];
        let compiled: HashSet<u32> = [2, 3].into_iter().collect();
        assert_eq!(inactive_items(&scans, &compiled), vec![(5, 9, false), (10, 10, true)]);
        // A `#[cfg]` block or field is not an item.
        let body = ["fn f() {", "    #[cfg(miri)]", "    {", "        g();", "    }", "}"];
        assert_eq!(inactive_items(&body, &[0].into_iter().collect()), vec![]);
    }
}
