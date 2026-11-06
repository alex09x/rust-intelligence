/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Early return transformation for workspace inline into all callers.
//!
//! Transforms function bodies containing early returns into labeled blocks:
//! `'inl: { ... break 'inl val; ... }` with `return;` -> `break 'inl;`.

use super::action_parse::is_ident_byte;
use super::action_parse_items::contains_word;

pub(crate) fn transform_early_returns(body_src: &str, masked_body: &str) -> Option<(String, String)> {
    if masked_body.contains("'inl") {
        return None;
    }
    if contains_word(masked_body, "async") || contains_word(masked_body, "fn") {
        return None;
    }

    let mb = masked_body.as_bytes();
    let mut return_indices = Vec::new();
    let mut i = 0usize;
    while i < mb.len() {
        if mb[i] == b'r' && i + 6 <= mb.len() && &mb[i..i + 6] == b"return" {
            let prev_ok = i == 0 || (!is_ident_byte(mb[i - 1]) && mb[i - 1] != b'.');
            let next_ok = i + 6 == mb.len() || !is_ident_byte(mb[i + 6]);
            if prev_ok && next_ok {
                return_indices.push(i);
            }
        }
        i += 1;
    }

    if return_indices.is_empty() {
        return None;
    }

    // Safety check: ensure no return is inside a closure.
    if !is_all_returns_at_fn_level(mb, &return_indices) {
        return None;
    }

    // Rewrite all `return` occurrences to `break 'inl`
    let mut new_body = String::from("'inl: {");
    let mut new_masked = String::from("'inl: {");
    let mut last_end = 0usize;

    for &ret_idx in &return_indices {
        new_body.push_str(&body_src[last_end..ret_idx]);
        new_masked.push_str(&masked_body[last_end..ret_idx]);
        new_body.push_str("break 'inl");
        new_masked.push_str("break 'inl");
        last_end = ret_idx + 6;
    }

    new_body.push_str(&body_src[last_end..]);
    new_masked.push_str(&masked_body[last_end..]);
    new_body.push_str(" }");
    new_masked.push_str(" }");

    Some((new_body, new_masked))
}

fn is_all_returns_at_fn_level(mb: &[u8], return_indices: &[usize]) -> bool {
    let mut i = 0usize;
    let mut depth = 0usize;
    let mut in_closure_params = false;
    let mut closure_body_depth: Option<usize> = None;

    while i < mb.len() {
        match mb[i] {
            b'(' | b'[' => depth += 1,
            b')' | b']' => {
                depth = depth.saturating_sub(1);
            }
            b'{' => {
                depth += 1;
            }
            b'}' => {
                if let Some(cd) = closure_body_depth {
                    if depth <= cd {
                        closure_body_depth = None;
                    }
                }
                depth = depth.saturating_sub(1);
            }
            b'|' => {
                if in_closure_params {
                    in_closure_params = false;
                    let mut next = i + 1;
                    while next < mb.len() && mb[next].is_ascii_whitespace() {
                        next += 1;
                    }
                    if next < mb.len() && mb[next] == b'{' {
                        closure_body_depth = Some(depth + 1);
                    }
                } else {
                    let prev_non_ws = mb[..i].iter().rposition(|&b| !b.is_ascii_whitespace());
                    let is_start = match prev_non_ws {
                        None => true,
                        Some(p) => matches!(mb[p], b'=' | b'(' | b',' | b'[' | b'{' | b';' | b':'),
                    };
                    if is_start {
                        in_closure_params = true;
                    }
                }
            }
            b'r' if mb[i..].starts_with(b"return") => {
                if (closure_body_depth.is_some() || in_closure_params) && return_indices.contains(&i) {
                    return false;
                }
            }
            _ => {}
        }
        i += 1;
    }

    true
}
