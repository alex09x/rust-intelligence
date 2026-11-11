/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Tokens for a target root its own `#![cfg(…)]` switches off. rust-analyzer still resolves what the crate root
//! reaches (the preludes, macros, other crates' paths, the `use` paths themselves) but none of the file's own items or
//! imports, so their uses, method calls and uses of local bindings are `unresolvedReference`; definition sites and
//! bindings are typed by their syntax.

use super::tokens::{RawToken, TOKEN_TYPE_GENERIC, TOKEN_TYPE_UNRESOLVED, TOKEN_TYPE_VARIABLE};
use std::collections::HashSet;

fn is_id(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// Whether masked line `scan` starts a `use` item.
fn starts_use(scan: &str) -> bool {
    let t = scan.trim_start();
    let t = t.strip_prefix("pub").map_or(t, |r| if r.starts_with('(') { r.split_once(')').map_or(r, |(_, a)| a) } else { r }).trim_start();
    t.starts_with("use ")
}

/// Names the file defines at item level or brings in with `use` (the leaf of each path, or its alias).
fn own_names(text: &str, masked: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    for s in crate::lsp::symbols::document_symbol_tree::document_symbols(text).as_array().into_iter().flatten() {
        if let Some(n) = s["name"].as_str() {
            out.insert(n.to_string());
        }
    }
    let mut in_use = false;
    for scan in masked.lines() {
        if !in_use && !starts_use(scan) {
            continue;
        }
        let body = if in_use { scan } else { scan.trim_start().split_once("use ").map_or(scan, |(_, b)| b) };
        in_use = !scan.contains(';');
        let b = body.as_bytes();
        let mut i = 0;
        while i < b.len() {
            if is_id(b[i]) && (i == 0 || !is_id(b[i - 1])) {
                let s = i;
                while i < b.len() && is_id(b[i]) {
                    i += 1;
                }
                let next = body[i..].trim_start();
                if !next.starts_with("::") && !next.starts_with("as ") && !matches!(&body[s..i], "self" | "super" | "crate" | "as") {
                    out.insert(body[s..i].to_string());
                }
                continue;
            }
            i += 1;
        }
    }
    out
}

/// The first segment of the path that ends right before byte `at` of `scan` (`a::b::` → `a`), if `at` follows `::`.
fn path_root(scan: &str, at: usize) -> Option<&str> {
    let mut end = at;
    let mut root = None;
    while scan[..end].ends_with("::") {
        let seg_end = end - 2;
        let seg_start = scan[..seg_end].bytes().rposition(|c| !is_id(c)).map_or(0, |p| p + 1);
        if seg_start == seg_end {
            break;
        }
        root = Some(&scan[seg_start..seg_end]);
        end = seg_start;
    }
    root
}

/// Rewrites the scanner's `tokens` for a switched-off file: definition sites and bindings come from `defs`, and the
/// uses the crate root cannot resolve become `unresolvedReference`.
pub(super) fn retype_gated_tokens(text: &str, defs: &[RawToken], tokens: &mut Vec<RawToken>) {
    let masked = crate::lsp::editing::action_parse::mask_comments_and_strings(text);
    let names = own_names(text, &masked);
    let def_at: HashSet<(u32, u32)> = defs.iter().map(|d| (d.line, d.col)).collect();
    let lines: Vec<&str> = text.lines().collect();
    let scans: Vec<&str> = masked.lines().collect();
    let mut use_lines = vec![false; scans.len()];
    let mut in_use = false;
    for (i, s) in scans.iter().enumerate() {
        if in_use || starts_use(s) {
            use_lines[i] = true;
            in_use = !s.contains(';');
        }
    }
    tokens.retain(|t| !defs.iter().any(|d| d.line == t.line && t.col < d.col + d.length && d.col < t.col + t.length));
    for t in tokens.iter_mut() {
        let (Some(line), Some(scan)) = (lines.get(t.line as usize), scans.get(t.line as usize)) else { continue };
        if !line.is_ascii() || line.len() != scan.len() || use_lines[t.line as usize] || def_at.contains(&(t.line, t.col)) {
            continue;
        }
        let (s, e) = (t.col as usize, (t.col + t.length) as usize);
        if e > scan.len() || !scan[s..e].bytes().all(is_id) {
            continue;
        }
        let name = &scan[s..e];
        // The derives of a switched-off item expand to nothing: their names are left as plain attribute tokens.
        if scan[..s].rfind("#[derive(").is_some_and(|d| !scan[d..s].contains(")]")) {
            t.token_type = TOKEN_TYPE_GENERIC;
            t.token_modifiers = 0;
            continue;
        }
        let method_or_field = scan[..s].trim_end().ends_with('.') && !scan[..s].trim_end().ends_with("..");
        let own = names.contains(name) && !scan[..s].ends_with("::");
        let under_own = path_root(scan, s).is_some_and(|r| names.contains(r));
        if method_or_field || own || under_own || t.token_type == TOKEN_TYPE_VARIABLE {
            t.token_type = TOKEN_TYPE_UNRESOLVED;
            t.token_modifiers = 0;
        }
    }
    tokens.extend_from_slice(defs);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_of_own_names_methods_and_locals_are_unresolved() {
        let text = "#![cfg(feature = \"off\")]\nuse tokio::runtime::Builder;\nfn rt() -> Builder {\n    let b = Builder::new();\n    b.build();\n    tokio::spawn(1)\n}\n";
        let tok = |line, col, length, token_type| RawToken { line, col, length, token_type, token_modifiers: 0 };
        let defs = vec![tok(2, 3, 2, 11), tok(3, 8, 1, TOKEN_TYPE_VARIABLE)];
        let mut tokens = vec![tok(1, 20, 7, 5), tok(2, 11, 7, 5), tok(3, 12, 7, 5), tok(3, 21, 3, 11), tok(4, 4, 1, TOKEN_TYPE_VARIABLE), tok(4, 6, 5, 12), tok(5, 11, 5, 11)];
        retype_gated_tokens(text, &defs, &mut tokens);
        let ty = |line, col| tokens.iter().find(|t| t.line == line && t.col == col).map(|t| t.token_type);
        // The `use` path keeps its type; `Builder` and its `new`, the method `build` and the local `b` do not resolve;
        // `tokio::spawn` does.
        assert_eq!(ty(1, 20), Some(5));
        assert_eq!((ty(2, 11), ty(3, 12), ty(3, 21), ty(4, 4), ty(4, 6)), (Some(18), Some(18), Some(18), Some(18), Some(18)));
        assert_eq!(ty(5, 11), Some(11));
        assert_eq!((ty(2, 3), ty(3, 8)), (Some(11), Some(TOKEN_TYPE_VARIABLE)));
    }
}
