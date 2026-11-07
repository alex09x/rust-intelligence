/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Lazy `macro_rules!` scope tracking and zero-alloc `mod`-keyword pre-filtering for [`Builder`].

use std::borrow::Cow;
use std::cell::OnceCell;

use super::attr::{eval_outer_attrs, EffectiveAttrs};
use super::db::{FileId, ModuleId};
use super::mbe_parse::MacroRulesDef;
use super::mod_builder::Builder;
use crate::parser::parse;
use crate::sema_fn::norm_ident;
use crate::syntax_kind::SyntaxKind as K;
use crate::tree::{NodeId, SyntaxTree};

pub(super) type MacroEnv = Vec<u32>;

pub(super) struct LazyMacro<'a> {
    pub(super) name: Cow<'a, str>,
    body: Cow<'a, str>,
    has_mod_or_macro: bool,
    has_bang: bool,
    parsed: OnceCell<Option<MacroRulesDef>>,
}

impl<'a, 't> Builder<'a, 't> {
    pub(super) fn record_macro_rules(&mut self, file: FileId, st: &SyntaxTree<'_>, item: NodeId, macro_env: &mut MacroEnv, mbe_depth: u32) {
        let attrs = eval_outer_attrs(st, item, self.db, self.krate);
        if !attrs.enabled {
            return;
        }
        let (Some(name_node), Some(tt_node)) = (st.child(item, K::Name), st.child(item, K::TokenTree)) else {
            return;
        };
        let (name_cow, body_cow) = if mbe_depth == 0 {
            let file_src = self.sources.file_text(file);
            let nr = st.range(name_node);
            let tr = st.range(tt_node);
            (Cow::Borrowed(norm_ident(&file_src[nr])), Cow::Borrowed(&file_src[tr]))
        } else {
            (Cow::Owned(norm_ident(st.text(name_node)).to_string()), Cow::Owned(st.text(tt_node).to_string()))
        };
        let has_mod_or_macro = has_mod_or_macro_kw(&body_cow);
        let has_bang = body_cow.as_bytes().contains(&b'!');
        if has_mod_or_macro {
            self.any_def_has_mod = true;
        }
        let idx = self.macro_defs.len() as u32;
        self.macro_defs.push(LazyMacro { name: name_cow, body: body_cow, has_mod_or_macro, has_bang, parsed: OnceCell::new() });
        if attrs.macro_export {
            self.exported_macros.push(idx);
        }
        macro_env.push(idx);
    }

    pub(super) fn try_expand_macro_call(&mut self, mod_id: ModuleId, file: FileId, st: &SyntaxTree<'_>, call_node: NodeId, macro_env: &mut MacroEnv, mbe_depth: u32) {
        let (Some(path_node), Some(tt_node)) = (st.child(call_node, K::Path), st.child(call_node, K::TokenTree)) else {
            return;
        };
        let inner_call = strip_tt_delims(st.text(tt_node));
        let call_has_mod = has_mod_or_macro_kw(inner_call);
        let raw_call = st.text(path_node).trim();

        if !raw_call.contains("::") {
            let short_name = norm_ident(raw_call);
            let defs = &self.macro_defs;
            let local_idx = macro_env
                .iter()
                .rev()
                .copied()
                .find(|&i| defs[i as usize].name == short_name)
                .or_else(|| self.exported_macros.iter().rev().copied().find(|&i| defs[i as usize].name == short_name));
            if let Some(idx) = local_idx {
                let entry = &self.macro_defs[idx as usize];
                let can_emit = call_has_mod || entry.has_mod_or_macro || (entry.has_bang && self.any_def_has_mod);
                if !can_emit {
                    return;
                }
                let attrs = eval_outer_attrs(st, call_node, self.db, self.krate);
                if !attrs.enabled {
                    return;
                }
                let entry = &self.macro_defs[idx as usize];
                let expanded = {
                    let parsed_opt = entry.parsed.get_or_init(|| MacroRulesDef::parse_body(&entry.body));
                    let Some(parsed) = parsed_opt.as_ref() else {
                        return;
                    };
                    let Some(exp) = parsed.expand(inner_call, "crate") else {
                        return;
                    };
                    exp
                };
                if has_mod_or_macro_kw(&expanded) {
                    let exp_tree = parse(&expanded);
                    self.scan_container(mod_id, file, &exp_tree, exp_tree.root(), macro_env, mbe_depth + 1);
                }
                return;
            }
        }

        let mod_path = self.tree.crate_path(mod_id);
        let Some(ext_body) = self.db.macro_rules_body(self.krate, mod_path, raw_call) else {
            return;
        };
        if !call_has_mod && !has_mod_or_macro_kw(ext_body) {
            return;
        }
        let attrs = eval_outer_attrs(st, call_node, self.db, self.krate);
        if !attrs.enabled {
            return;
        }
        let Some(parsed) = MacroRulesDef::parse_body(ext_body) else {
            return;
        };
        let Some(expanded) = parsed.expand(inner_call, "crate") else {
            return;
        };
        if has_mod_or_macro_kw(&expanded) {
            let exp_tree = parse(&expanded);
            self.scan_container(mod_id, file, &exp_tree, exp_tree.root(), macro_env, mbe_depth + 1);
        }
    }
}

pub(super) fn finish_macro_scope(outer: &EffectiveAttrs<'_>, env_mark: usize, macro_env: &mut MacroEnv, defs: &[LazyMacro<'_>]) {
    if !outer.macro_use {
        macro_env.truncate(env_mark);
        return;
    }
    if let Some(filter) = &outer.macro_use_filter {
        let mut write = env_mark;
        for read in env_mark..macro_env.len() {
            let name = defs[macro_env[read] as usize].name.as_ref();
            if filter.contains(&name) {
                if write != read {
                    macro_env.swap(write, read);
                }
                write += 1;
            }
        }
        macro_env.truncate(write);
    }
}

#[inline]
fn is_ascii_ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Zero-alloc word scan returning `true` iff `s` contains the ASCII identifier `mod` or `macro_rules`.
#[inline]
pub(super) fn has_mod_or_macro_kw(s: &str) -> bool {
    let b = s.as_bytes();
    let len = b.len();
    let mut i = 0;
    while i + 3 <= len {
        if b[i] == b'm' && (i == 0 || !is_ascii_ident(b[i - 1])) {
            if b[i + 1] == b'o' && b[i + 2] == b'd' && (i + 3 == len || !is_ascii_ident(b[i + 3])) && !(i >= 2 && b[i - 2] == b'r' && b[i - 1] == b'#') {
                return true;
            }
            if i + 11 <= len && &b[i + 1..i + 11] == b"acro_rules" && (i + 11 == len || !is_ascii_ident(b[i + 11])) {
                return true;
            }
        }
        i += 1;
    }
    false
}

fn strip_tt_delims(s: &str) -> &str {
    let b = s.as_bytes();
    if b.len() >= 2 && matches!((b[0], b[b.len() - 1]), (b'{', b'}') | (b'(', b')') | (b'[', b']')) {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

/// Zero-alloc word scan returning `true` iff `s` contains any top-level item keyword.
pub(super) fn has_scope_item_kw(s: &str) -> bool {
    let b = s.as_bytes();
    for kw in [&b"use"[..], b"mod", b"struct", b"enum", b"trait", b"type", b"fn", b"const", b"static", b"extern", b"macro_rules"] {
        if contains_word(b, kw) {
            return true;
        }
    }
    false
}

fn contains_word(hay: &[u8], kw: &[u8]) -> bool {
    if hay.len() < kw.len() {
        return false;
    }
    let mut i = 0;
    while i + kw.len() <= hay.len() {
        if &hay[i..i + kw.len()] == kw && (i == 0 || !is_ascii_ident(hay[i - 1])) && (i + kw.len() == hay.len() || !is_ascii_ident(hay[i + kw.len()])) {
            return true;
        }
        i += 1;
    }
    false
}
