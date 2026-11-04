/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! The default `fn` members of a trait body, as `Implement default members` copies them.

use super::action_default_qualify::cfg_attr_only_documents;
use super::action_generics_sub::{parse_generics_list, GenericDecl};
use super::action_parse_decl::{extract_angle_generics, find_header_terminator, next_ident_token};
use super::action_parse_items::{contains_word, skip_balanced};
use std::collections::HashSet;

pub(crate) struct DefaultFnMember {
    pub name: String,
    /// Byte offset of `fn_src` in the trait body text.
    pub start: usize,
    pub cfg_attrs: Vec<String>,
    pub fn_src: String,
    pub fn_masked: String,
    pub params_masked: String,
    pub fn_gens: Vec<GenericDecl>,
    pub fn_gen_names: HashSet<String>,
}

/// Default `fn` members of a trait body. `compiled(name)` says whether the index holds the member (`None`: the index
/// does not know the trait); it decides `cfg`-gated members of a trait from another crate.
pub(crate) fn extract_default_fn_members(body_src: &str, body_masked: &str, same_crate: bool, compiled: &dyn Fn(&str) -> Option<bool>) -> Option<Vec<DefaultFnMember>> {
    let mb = body_masked.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < mb.len() {
        while i < mb.len() && mb[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= mb.len() {
            break;
        }
        let mut cfg_attrs = Vec::new();
        while mb.get(i).copied() == Some(b'#') {
            let attr_start = i;
            let mut k = i + 1;
            if mb.get(k).copied() == Some(b'!') {
                k += 1;
            }
            while k < mb.len() && mb[k].is_ascii_whitespace() {
                k += 1;
            }
            if mb.get(k).copied() == Some(b'[') {
                let after_a = skip_balanced(mb, k, b'[', b']')?;
                let inner_m = &body_masked[k + 1..after_a - 1];
                if contains_word(inner_m, "cfg_attr") {
                    // `cfg_attr(docsrs, doc(cfg(...)))` only documents the member; it is not copied.
                    if !cfg_attr_only_documents(inner_m) {
                        return None;
                    }
                } else if contains_word(inner_m, "cfg") {
                    cfg_attrs.push(body_src[attr_start..after_a].trim().to_string());
                }
                i = after_a;
                while i < mb.len() && mb[i].is_ascii_whitespace() {
                    i += 1;
                }
            } else {
                break;
            }
        }
        let item_start = i;
        let mut pos = i;
        while let Some((tok, next_p)) = next_ident_token(body_masked, pos) {
            if matches!(tok, "async" | "const" | "unsafe" | "extern") {
                pos = next_p;
            } else {
                break;
            }
        }
        let Some((kw, kw_end)) = next_ident_token(body_masked, pos) else {
            i += 1;
            continue;
        };
        if kw == "fn" {
            let (raw_n, name_end) = next_ident_token(body_src, kw_end)?;
            let (gen_s, after_g) = extract_angle_generics(body_src, body_masked, name_end);
            let mut p = after_g;
            while p < mb.len() && mb[p].is_ascii_whitespace() {
                p += 1;
            }
            let paren_end = skip_balanced(mb, p, b'(', b')')?;
            let term = find_header_terminator(mb, paren_end)?;
            if mb[term] == b'{' {
                let after_body = skip_balanced(mb, term, b'{', b'}')?;
                // Another crate's `cfg`s were evaluated when it was compiled: a member the index holds is active
                // there (copied without its `cfg`), one it lacks is not (skipped); unknown refuses.
                if !same_crate && !cfg_attrs.is_empty() {
                    match compiled(raw_n.strip_prefix("r#").unwrap_or(raw_n)) {
                        Some(true) => cfg_attrs.clear(),
                        Some(false) => {
                            i = after_body;
                            continue;
                        }
                        None => return None,
                    }
                }
                let fn_gens = parse_generics_list(&gen_s);
                let fn_gen_names: HashSet<String> = fn_gens.iter().map(|g| g.name.clone()).collect();
                out.push(DefaultFnMember {
                    name: raw_n.strip_prefix("r#").unwrap_or(raw_n).to_string(),
                    cfg_attrs,
                    start: item_start,
                    fn_src: body_src[item_start..after_body].to_string(),
                    fn_masked: body_masked[item_start..after_body].to_string(),
                    params_masked: body_masked[p + 1..paren_end - 1].to_string(),
                    fn_gens,
                    fn_gen_names,
                });
                i = after_body;
                continue;
            }
            i = term + 1;
            continue;
        }
        let term = find_header_terminator(mb, kw_end)?;
        i = if mb[term] == b'{' { skip_balanced(mb, term, b'{', b'}')? } else { term + 1 };
    }
    Some(out)
}
