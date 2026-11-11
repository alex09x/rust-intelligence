/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! A struct handed to a macro by its bare path (`diagnostics![.., TypedHole<'db>, ..]`) keeps its shape only if the
//! macro cannot build or destructure it: a `macro_rules!` body naming none of its fields cannot, since a literal or
//! pattern of a struct with named fields spells them (`$t { .. }` matches a tuple struct as well).

use super::action_parse::mask_comments_and_strings;
use super::action_parse_items::{contains_word, skip_balanced};
use super::action_struct_conv_outside::enclosing_open;
use ridx_format::{DefKind, WorkspaceIndex};

/// Whether the macro whose call holds byte `b` of `masked` may use the struct by its `fields`: a `macro_rules!` of
/// that name whose body names one of them, or a macro whose body cannot be read (a procedural one).
pub(crate) fn macro_may_use_fields(ws: &WorkspaceIndex, masked: &str, b: usize, fields: &[&str]) -> bool {
    let Some(name) = macro_name_at(masked, b) else { return true };
    macro_rules_bodies(ws, name).is_none_or(|bodies| bodies.iter().any(|body| fields.iter().any(|f| contains_word(body, f))))
}

/// The masked bodies of every `macro_rules! name` the index knows; `None` when there is none or one cannot be read
/// (a procedural macro, a file gone from disk).
pub(crate) fn macro_rules_bodies(ws: &WorkspaceIndex, name: &str) -> Option<Vec<String>> {
    let defs: Vec<_> = ws.exact_symbols_by_name(name).into_iter().filter(|s| s.kind == DefKind::Macro).collect();
    if defs.is_empty() {
        return None;
    }
    defs.iter()
        .map(|d| {
            let path = match d.source_root {
                Some(root) if !std::path::Path::new(d.file).is_absolute() => ridx_format::join_resolving_overlap(root, std::path::Path::new(d.file)),
                _ => ws.resolve_path_for_crate(d.crate_name, d.file),
            };
            macro_rules_body(&mask_comments_and_strings(&std::fs::read_to_string(path).ok()?), name)
        })
        .collect()
}

/// The name of the innermost macro call (`name!(…)`, `name![…]`, `name! {…}`) around byte `b`.
fn macro_name_at(masked: &str, b: usize) -> Option<&str> {
    let mut at = b;
    while let Some(open) = enclosing_open(masked, at) {
        let before = masked[..open].trim_end();
        if let Some(head) = before.strip_suffix('!') {
            let start = head.trim_end_matches(|c: char| c.is_alphanumeric() || c == '_').len();
            return Some(&head[start..]).filter(|n| !n.is_empty());
        }
        at = open;
    }
    None
}

/// The body of `macro_rules! name { … }` in the masked text.
fn macro_rules_body(masked: &str, name: &str) -> Option<String> {
    let b = masked.as_bytes();
    masked.match_indices("macro_rules!").find_map(|(p, _)| {
        let rest = masked[p + 12..].trim_start();
        let after_name = rest.strip_prefix(name).filter(|r| !r.starts_with(|c: char| c.is_alphanumeric() || c == '_'))?;
        let open = masked.len() - after_name.trim_start().len();
        let close_ch = match *b.get(open)? {
            b'(' => b')',
            b'[' => b']',
            b'{' => b'}',
            _ => return None,
        };
        let close = skip_balanced(b, open, b[open], close_ch)?;
        Some(masked[open..close].to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::{macro_name_at, macro_rules_body};

    #[test]
    fn the_macro_around_a_path_and_its_body_are_read() {
        let s = "diagnostics![AnyDiagnostic -> TypedHole, Other];";
        assert_eq!(macro_name_at(s, s.find("TypedHole").unwrap()), Some("diagnostics"));
        let def = "macro_rules! diagnostics { ($($d:ident),*) => { $(impl From<$d> for Any { fn from(d: $d) -> Any { Any::$d(d) } })* } }";
        let body = macro_rules_body(def, "diagnostics").unwrap();
        assert!(body.contains("impl From") && !body.contains("expr:"));
        assert_eq!(macro_rules_body(def, "diag"), None);
    }
}
