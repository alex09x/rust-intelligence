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

/// `masked` with the `impl` and `fn` items it declares (at the start of a statement) blanked out, header to closing
/// brace: their `self`, `return` and `fn` are their own.
pub(crate) fn without_nested_items(masked: &str) -> String {
    let b = masked.as_bytes();
    let mut out = masked.to_string().into_bytes();
    let mut i = 0;
    while let Some(p) = masked[i..].find(['i', 'f']).map(|p| p + i) {
        let kw = ["impl", "fn"].into_iter().find(|k| masked[p..].starts_with(k) && b.get(p + k.len()).is_some_and(|c| !is_ident_byte(*c)));
        let head = masked[..p].trim_end();
        let at_statement = head.is_empty()
            || head.ends_with([';', '{', '}', ']'])
            || ["pub", "const", "unsafe", "async"].iter().any(|k| head.ends_with(k) && !head[..head.len() - k.len()].ends_with(|c: char| c.is_alphanumeric() || c == '_'));
        let starts_word = p == 0 || !is_ident_byte(b[p - 1]);
        match (kw, starts_word && at_statement, masked[p..].find('{').map(|o| p + o)) {
            (Some(_), true, Some(open)) => {
                let end = super::action_parse_items::skip_balanced(b, open, b'{', b'}').unwrap_or(b.len());
                out[p..end].fill(b' ');
                i = end;
            }
            _ => i = p + 1,
        }
    }
    String::from_utf8(out).unwrap_or_default()
}

pub(crate) fn transform_early_returns(body_src: &str, masked_body: &str) -> Option<(String, String)> {
    if masked_body.contains("'inl") {
        return None;
    }
    // The body's own code, without the items it declares (`impl Drop for G { fn drop(..) { .. } }`).
    let own = without_nested_items(masked_body);
    if contains_word(&own, "async") || contains_word(&own, "fn") {
        return None;
    }

    let mb = own.as_bytes();
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

    // A `return` in a closure leaves the closure, not the function: it stays as written.
    let in_closure = closure_returns(mb, &return_indices);
    let return_indices: Vec<usize> = return_indices.into_iter().zip(in_closure).filter(|(_, c)| !c).map(|(i, _)| i).collect();
    if return_indices.is_empty() {
        return Some((body_src.to_string(), masked_body.to_string()));
    }

    // Rewrite the function's own `return`s to `break 'inl`
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

/// For each `return` at `return_indices`, whether it lies in a closure (its braced body, or right after its `|…|`).
fn closure_returns(mb: &[u8], return_indices: &[usize]) -> Vec<bool> {
    let mut out = vec![false; return_indices.len()];
    let mut i = 0usize;
    let mut depth = 0usize;
    let mut in_closure_params = false;
    // Depths of the braced closure bodies open at `i`, innermost last.
    let mut closure_bodies: Vec<usize> = Vec::new();
    let mut after_params = false;
    // Depths at which braceless closure bodies (`|x| expr`) run, until a `,`/`;` there or the group closes.
    let mut braceless: Vec<usize> = Vec::new();
    while i < mb.len() {
        match mb[i] {
            b'(' | b'[' => depth += 1,
            b')' | b']' => {
                depth = depth.saturating_sub(1);
                braceless.retain(|&d| d <= depth);
            }
            b'{' => {
                depth += 1;
                if after_params {
                    closure_bodies.push(depth);
                    after_params = false;
                }
            }
            b'}' => {
                if closure_bodies.last() == Some(&depth) {
                    closure_bodies.pop();
                }
                depth = depth.saturating_sub(1);
                braceless.retain(|&d| d <= depth);
            }
            b',' | b';' => braceless.retain(|&d| d < depth),
            b'|' if in_closure_params => {
                in_closure_params = false;
                after_params = true;
                i += 1;
                continue;
            }
            b'|' => {
                let prev = mb[..i].iter().rposition(|&b| !b.is_ascii_whitespace());
                let next_is_bar = mb.get(i + 1) == Some(&b'|');
                if prev.is_none_or(|p| matches!(mb[p], b'=' | b'(' | b',' | b'[' | b'{' | b';' | b':' | b'>')) || (next_is_bar && prev.is_some_and(|p| mb[p] == b'(')) {
                    if next_is_bar {
                        after_params = true;
                        i += 2;
                        continue;
                    }
                    in_closure_params = true;
                }
            }
            c if !c.is_ascii_whitespace() => {
                if after_params {
                    braceless.push(depth);
                    after_params = false;
                }
                if let Some(k) = return_indices.iter().position(|&r| r == i) {
                    out[k] = !closure_bodies.is_empty() || in_closure_params || !braceless.is_empty();
                }
            }
            _ => {}
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::transform_early_returns;

    #[test]
    fn rewrites_only_the_functions_own_returns() {
        let body = "if a { return 1; } let f = |x| { if x { return 2; } 3 }; let g = |y| return y; f(4)";
        let (out, _) = transform_early_returns(body, body).unwrap();
        assert!(out.contains("break 'inl 1;") && out.contains("return 2;") && out.contains("|y| return y"), "{out}");
        let only_closures = "let f = |x| { return x; }; f(1)";
        assert_eq!(transform_early_returns(only_closures, only_closures).unwrap().0, only_closures);
        let braceless = "let f = |x| if x { return 1 } else { 2 }; f(true)";
        assert_eq!(transform_early_returns(braceless, braceless).unwrap().0, braceless, "a braceless closure's `return`");
    }
}
