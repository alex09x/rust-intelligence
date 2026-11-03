/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Module-scope identifier and macro-invocation scanners for `Implement default members`.

pub use super::action_default_safety::is_default_fn_compile_safe;
use super::action_generics_sub::split_top_level_commas;
use super::action_parse_decl::next_ident_token;
use super::action_parse_items::skip_balanced;
use std::collections::HashSet;

pub fn has_top_level_macro_call(masked_slice: &str) -> bool {
    let b = masked_slice.as_bytes();
    let (mut brace, mut paren, mut bracket) = (0i32, 0i32, 0i32);
    let mut i = 0usize;
    while i < b.len() {
        if brace == 0 && paren == 0 && bracket == 0 && b[i] == b'#' {
            let mut k = i + 1;
            if b.get(k).copied() == Some(b'!') {
                k += 1;
            }
            while k < b.len() && mb_ws(b.get(k).copied()) {
                k += 1;
            }
            if b.get(k).copied() == Some(b'[') {
                if let Some(after_attr) = skip_balanced(b, k, b'[', b']') {
                    i = after_attr;
                    continue;
                }
            }
        }
        match b[i] {
            b'{' => brace += 1,
            b'}' => brace = (brace - 1).max(0),
            b'(' => paren += 1,
            b')' => paren = (paren - 1).max(0),
            b'[' => bracket += 1,
            b']' => bracket = (bracket - 1).max(0),
            b'!' if brace == 0 && paren == 0 && bracket == 0 && b.get(i + 1).copied() != Some(b'=') => {
                return true;
            }
            _ => {}
        }
        i += 1;
    }
    false
}

#[inline]
fn mb_ws(b: Option<u8>) -> bool {
    b.is_some_and(|c| c.is_ascii_whitespace())
}

pub fn enclosing_module_span(masked_doc: &str, target_byte: usize) -> (usize, usize) {
    let b = masked_doc.as_bytes();
    let (mut lo, mut hi) = (0usize, b.len());
    'outer: loop {
        let (mut brace, mut paren, mut bracket) = (0i32, 0i32, 0i32);
        let mut i = lo;
        while i < hi {
            match b[i] {
                b'{' => {
                    brace += 1;
                    i += 1;
                }
                b'}' => {
                    brace = (brace - 1).max(0);
                    i += 1;
                }
                b'(' => {
                    paren += 1;
                    i += 1;
                }
                b')' => {
                    paren = (paren - 1).max(0);
                    i += 1;
                }
                b'[' => {
                    bracket += 1;
                    i += 1;
                }
                b']' => {
                    bracket = (bracket - 1).max(0);
                    i += 1;
                }
                _ if brace == 0 && paren == 0 && bracket == 0 && (b[i].is_ascii_alphabetic() || b[i] == b'_') => {
                    let Some((tok, end_tok)) = next_ident_token(masked_doc, i) else {
                        i += 1;
                        continue;
                    };
                    if tok == "mod" {
                        if let Some((_, after_name)) = next_ident_token(masked_doc, end_tok) {
                            let mut k = after_name;
                            while k < hi && mb_ws(b.get(k).copied()) {
                                k += 1;
                            }
                            if k < hi && b[k] == b'{' {
                                if let Some(after_mod) = skip_balanced(b, k, b'{', b'}') {
                                    let close_brace = after_mod - 1;
                                    if k < target_byte && target_byte < close_brace {
                                        lo = k + 1;
                                        hi = close_brace;
                                        continue 'outer;
                                    }
                                    i = after_mod;
                                    continue;
                                }
                            }
                        }
                    }
                    i = end_tok;
                }
                _ => i += 1,
            }
        }
        return (lo, hi);
    }
}

pub fn collect_module_scope_idents(masked_doc: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    let b = masked_doc.as_bytes();
    let (mut brace, mut paren, mut bracket) = (0i32, 0i32, 0i32);
    let mut i = 0usize;
    while i < b.len() {
        match b[i] {
            b'{' => {
                brace += 1;
                i += 1;
            }
            b'}' => {
                brace = (brace - 1).max(0);
                i += 1;
            }
            b'(' => {
                paren += 1;
                i += 1;
            }
            b')' => {
                paren = (paren - 1).max(0);
                i += 1;
            }
            b'[' => {
                bracket += 1;
                i += 1;
            }
            b']' => {
                bracket = (bracket - 1).max(0);
                i += 1;
            }
            _ if brace == 0 && paren == 0 && bracket == 0 && (b[i].is_ascii_alphabetic() || b[i] == b'_') => {
                let Some((tok, end_tok)) = next_ident_token(masked_doc, i) else {
                    i += 1;
                    continue;
                };
                if tok == "use" {
                    let mut semi = end_tok;
                    let mut ub = 0i32;
                    while semi < b.len() {
                        match b[semi] {
                            b'{' => ub += 1,
                            b'}' => ub = (ub - 1).max(0),
                            b';' if ub == 0 => break,
                            _ => {}
                        }
                        semi += 1;
                    }
                    collect_use_tree_leaf_idents(&masked_doc[end_tok..semi.min(b.len())], None, &mut out);
                    i = (semi + 1).min(b.len());
                    continue;
                }
                if matches!(tok, "fn" | "struct" | "enum" | "union" | "type" | "const" | "static" | "trait" | "mod") {
                    if let Some((name, after_name)) = next_ident_token(masked_doc, end_tok) {
                        out.insert(name.strip_prefix("r#").unwrap_or(name).to_string());
                        i = after_name;
                        continue;
                    }
                }
                i = end_tok;
            }
            _ => i += 1,
        }
    }
    out
}

fn collect_use_tree_leaf_idents(tree: &str, parent_seg: Option<&str>, out: &mut HashSet<String>) {
    for part in split_top_level_commas(tree) {
        let t = part.trim();
        if t.is_empty() || t.ends_with('*') {
            continue;
        }
        if let Some(open_b) = t.find('{') {
            if let Some(close_b) = t.rfind('}') {
                let prefix = t[..open_b].trim().trim_end_matches("::");
                let last_seg = prefix.rsplit("::").next().map(|s| s.trim().strip_prefix("r#").unwrap_or(s.trim()));
                collect_use_tree_leaf_idents(&t[open_b + 1..close_b], last_seg, out);
                continue;
            }
        }
        if let Some((_, alias)) = t.rsplit_once(" as ") {
            let a_raw = alias.trim();
            let a = a_raw.strip_prefix("r#").unwrap_or(a_raw);
            if a != "_" && !a.is_empty() {
                out.insert(a.to_string());
            }
            continue;
        }
        let raw_leaf = t.rsplit("::").next().unwrap_or(t).trim();
        let leaf = raw_leaf.strip_prefix("r#").unwrap_or(raw_leaf);
        if leaf == "self" {
            if let Some(p) = parent_seg {
                if !p.is_empty() && !matches!(p, "crate" | "super" | "self") {
                    out.insert(p.to_string());
                }
            }
        } else if !leaf.is_empty() && !matches!(leaf, "crate" | "super") {
            out.insert(leaf.to_string());
        }
    }
}
