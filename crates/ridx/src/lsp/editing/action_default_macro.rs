/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! An impl body may define members through macros (`forward_to_deserialize_any! { bool i8 … }`). A member the
//! macros may define is left out: a name a `macro_rules!` expansion defines is spelled in the call or in the bodies of
//! the macros it reaches, unless it is pasted together from pieces.

use super::action_inline_callers_macros::STD_MACROS;
use super::action_inline_callers_qualify::tokens;
use super::action_parse_items::{contains_word, skip_balanced};
use super::action_struct_conv_macro::macro_rules_bodies;
use ridx_format::WorkspaceIndex;
use std::collections::HashSet;

/// How many macros one impl's calls may reach before the answer is given up.
const MAX_MACROS: usize = 32;

/// Every word the top-level macro calls of `impl_body_masked` and the `macro_rules!` bodies they reach spell: the
/// members they may define are among them. `None` when a macro's body cannot be read (a procedural macro) or builds
/// names out of pieces (`paste!`'s `[<…>]`, `concat_idents!`).
pub(crate) fn macro_defined_words(ws: &WorkspaceIndex, impl_body_masked: &str) -> Option<HashSet<String>> {
    let mut words = HashSet::new();
    let mut queue = Vec::new();
    for (name, args) in top_level_macro_calls(impl_body_masked)? {
        words.extend(words_of(args));
        queue.push(name.to_string());
    }
    let mut seen = HashSet::new();
    while let Some(name) = queue.pop() {
        if !seen.insert(name.clone()) {
            continue;
        }
        if seen.len() > MAX_MACROS {
            return None;
        }
        for body in macro_rules_bodies(ws, &name)? {
            if body.contains("[<") || contains_word(&body, "concat_idents") || contains_word(&body, "paste") {
                return None;
            }
            words.extend(words_of(&body));
            queue.extend(macro_calls_in(&body).into_iter().filter(|n| !STD_MACROS.contains(&n.as_str())));
        }
    }
    Some(words)
}

/// The macro calls at the top level of an item body, as `(name, arguments)`; `None` for a call whose name or
/// arguments cannot be read.
fn top_level_macro_calls(masked: &str) -> Option<Vec<(&str, &str)>> {
    let b = masked.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < b.len() {
        match b[i] {
            b'#' => {
                let k = masked[i + 1..].trim_start_matches('!').trim_start();
                let at = masked.len() - k.len();
                i = if k.starts_with('[') { skip_balanced(b, at, b'[', b']')? } else { i + 1 };
            }
            b'{' => i = skip_balanced(b, i, b'{', b'}')?,
            b'(' => i = skip_balanced(b, i, b'(', b')')?,
            b'[' => i = skip_balanced(b, i, b'[', b']')?,
            b'!' if b.get(i + 1) != Some(&b'=') => {
                let head = masked[..i].trim_end();
                let name = &head[head.trim_end_matches(|c: char| c.is_alphanumeric() || c == '_').len()..];
                let rest = masked[i + 1..].trim_start();
                let open = masked.len() - rest.len();
                let close = match *b.get(open)? {
                    b'(' => b')',
                    b'[' => b']',
                    b'{' => b'}',
                    _ => return None,
                };
                let end = skip_balanced(b, open, b[open], close)?;
                if name.is_empty() {
                    return None;
                }
                out.push((name, &masked[open..end]));
                i = end;
            }
            _ => i += 1,
        }
    }
    Some(out)
}

fn words_of(masked: &str) -> impl Iterator<Item = String> + '_ {
    tokens(masked).into_iter().map(move |(s, e)| masked[s..e].to_string())
}

/// The names of the macros a `macro_rules!` body calls (`helper! { … }`), leaving out metavariables (`$m!`).
fn macro_calls_in(masked: &str) -> Vec<String> {
    tokens(masked)
        .into_iter()
        .filter(|&(s, e)| masked[e..].starts_with('!') && !masked[e..].starts_with("!=") && !masked[..s].ends_with('$'))
        .map(|(s, e)| masked[s..e].to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{macro_calls_in, top_level_macro_calls};

    #[test]
    fn top_level_macro_calls_and_their_nested_macros_are_read() {
        let body = "type Error = E; fn any(self) { visit!(x) } #[inline] forward! { bool i128 } m![a];";
        let calls = top_level_macro_calls(body).unwrap();
        assert_eq!(calls, vec![("forward", "{ bool i128 }"), ("m", "[a]")]);
        assert_eq!(macro_calls_in("{ $(helper!{$f<'de>})* $m!{} if a != b {} }"), vec!["helper".to_string()]);
    }
}
