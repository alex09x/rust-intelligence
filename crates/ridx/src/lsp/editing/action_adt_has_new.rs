/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Whether a struct already has an inherent `new`, which `Generate new` would duplicate (`E0592`).

use super::action_parse::{is_ident_byte, ParsedCursorItem};
use super::action_parse_decl::{extract_angle_generics, next_ident_token, split_top_level_where};
use super::action_parse_items::{contains_word, skip_balanced};
use super::action_struct_conv_macro::macro_rules_bodies;
use ridx_format::{DefKind, WorkspaceIndex};
use std::collections::HashSet;
use std::path::Path;

pub(crate) fn struct_already_has_new(doc_text: &str, masked_doc: &str, file_path: &str, item: &ParsedCursorItem, ws: &WorkspaceIndex) -> bool {
    let struct_name = item.name.as_str();
    // A struct declared in a function body is named only inside that body: an `impl` of a same-named struct in another
    // function is another struct's, and no other file can name it.
    if let Some((lo, hi)) = item.enclosing_block.filter(|_| item.in_fn_body) {
        return doc_defines_inherent_new_or_macro(&doc_text[lo..hi], &masked_doc[lo..hi], struct_name, ws);
    }
    if doc_defines_inherent_new_or_macro(doc_text, masked_doc, struct_name, ws) {
        return true;
    }
    let req_path = Path::new(file_path);
    // This struct's own `new` (its path, with or without the crate's name), not that of a same-named struct elsewhere.
    let mut news = HashSet::new();
    for s in ws.exact_local_symbols_by_name(struct_name) {
        if s.kind == DefKind::Struct && s.name.strip_prefix("r#").unwrap_or(s.name) == struct_name && s.resolved_path() == req_path {
            if ws.find_definition_by_path(&format!("{}::new", s.path)).is_some() {
                return true;
            }
            news.insert((s.crate_name.to_string(), format!("{}::new", s.path)));
            if let Some(rel) = s.path.strip_prefix(&format!("{}::", s.crate_name)) {
                news.insert((s.crate_name.to_string(), format!("{rel}::new")));
            }
        }
    }
    !news.is_empty() && ws.exact_local_symbols_by_name("new").iter().any(|sym| sym.name == "new" && news.contains(&(sym.crate_name.to_string(), sym.path.to_string())))
}

/// `new` as a name of its own, which a macro may define: not `Box::new` nor `x.new()`, which call a constructor.
fn names_new(masked: &str) -> bool {
    let b = masked.as_bytes();
    masked.match_indices("new").any(|(p, _)| {
        let before = masked[..p].trim_end();
        (p == 0 || !is_ident_byte(b[p - 1])) && b.get(p + 3).is_none_or(|c| !is_ident_byte(*c)) && !before.ends_with("::") && !before.ends_with('.')
    })
}

fn doc_defines_inherent_new_or_macro(doc_text: &str, masked_doc: &str, struct_name: &str, ws: &WorkspaceIndex) -> bool {
    let mb = masked_doc.as_bytes();
    let mut brace_depth = 0i32;
    let mut i = 0usize;
    while i < mb.len() {
        if brace_depth == 0 && mb[i] == b'!' && mb.get(i + 1).copied() != Some(b'=') {
            let mut k = i + 1;
            while k < mb.len() && mb[k].is_ascii_whitespace() {
                k += 1;
            }
            let end_m = match mb.get(k).copied() {
                Some(b'(') => skip_balanced(mb, k, b'(', b')'),
                Some(b'[') => skip_balanced(mb, k, b'[', b']'),
                Some(b'{') => skip_balanced(mb, k, b'{', b'}'),
                _ => None,
            };
            if let Some(em) = end_m {
                // A macro may write `impl Name { fn new … }` when it names the struct and an `impl` or a `new` (in its
                // arguments, or its own name, `impl_new!`, unless its `macro_rules!` body names no `new`); one that
                // only mentions the type, or calls `Box::new`, does not.
                let args = &masked_doc[k..em];
                let name_start = masked_doc[..i].trim_end_matches(|c: char| c.is_alphanumeric() || c == '_').len();
                let macro_name = &masked_doc[name_start..i];
                let named_ctor =
                    (macro_name.contains("new") || macro_name.contains("impl")) && macro_rules_bodies(ws, macro_name).is_none_or(|bodies| bodies.iter().any(|b| names_new(b)));
                let writes_ctor = names_new(args) || contains_word(args, "impl") || named_ctor;
                if contains_word(args, struct_name) && writes_ctor {
                    return true;
                }
                i = em;
                continue;
            }
        }
        if i + 4 < mb.len() && &mb[i..i + 4] == b"impl" && (i == 0 || !is_ident_byte(mb[i - 1])) && !is_ident_byte(mb[i + 4]) {
            let (_, after_g) = extract_angle_generics(doc_text, masked_doc, i + 4);
            if let Some(bo) = super::action_parse_decl::find_header_terminator(mb, after_g) {
                if mb[bo] == b'{' {
                    if let Some(bc_after) = skip_balanced(mb, bo, b'{', b'}') {
                        let (head_s, head_m, _) = split_top_level_where(&doc_text[after_g..bo], &masked_doc[after_g..bo]);
                        if super::action_parse_decl::split_impl_trait_for(head_s, head_m).is_none() {
                            let base_ty = head_s.split('<').next().unwrap_or(head_s).trim().rsplit("::").next().unwrap_or("").trim();
                            if base_ty.strip_prefix("r#").unwrap_or(base_ty) == struct_name {
                                let body_m = &masked_doc[bo + 1..bc_after - 1];
                                if super::action_default_scope::has_top_level_macro_call(body_m) || impl_body_has_fn_new(body_m) {
                                    return true;
                                }
                            }
                        }
                        i = bc_after;
                        continue;
                    }
                }
            }
        }
        match mb[i] {
            b'{' => brace_depth += 1,
            b'}' => brace_depth = (brace_depth - 1).max(0),
            _ => {}
        }
        i += 1;
    }
    false
}

fn impl_body_has_fn_new(body_masked: &str) -> bool {
    let b = body_masked.as_bytes();
    let (mut br, mut pa, mut bk) = (0i32, 0i32, 0i32);
    let mut i = 0usize;
    while i < b.len() {
        match b[i] {
            b'{' => br += 1,
            b'}' => br = (br - 1).max(0),
            b'(' => pa += 1,
            b')' => pa = (pa - 1).max(0),
            b'[' => bk += 1,
            b']' => bk = (bk - 1).max(0),
            _ if br == 0
                && pa == 0
                && bk == 0
                && i + 2 <= b.len()
                && &b[i..i + 2] == b"fn"
                && (i == 0 || !is_ident_byte(b[i - 1]))
                && (i + 2 == b.len() || !is_ident_byte(b[i + 2])) =>
            {
                if let Some((n, _)) = next_ident_token(body_masked, i + 2) {
                    if n.strip_prefix("r#").unwrap_or(n) == "new" {
                        return true;
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::names_new;

    #[test]
    fn a_constructor_call_names_no_new() {
        assert!(names_new("fn new() -> Self"));
        assert!(names_new("ctor = new,"));
        assert!(!names_new("Box::into_raw(Box::new(Options::default()))"));
        assert!(!names_new("x.new() + renew"));
    }
}
