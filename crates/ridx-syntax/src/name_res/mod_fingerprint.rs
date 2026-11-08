/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Zero-false-positive module-structure CST fingerprinting and [`ModuleTree`] structural equality
//! for [`super::crate_snapshot::CrateSnapshot`].

use super::attr::{eval_inner_attrs, eval_outer_attrs, extract_visibility, EffectiveAttrs};
use super::db::{CrateId, ItemDb};
use super::mod_tree::ModuleTree;
use crate::sema_fn::norm_ident;
use crate::syntax_kind::SyntaxKind as K;
use crate::tree::{NodeId, SyntaxTree};

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

#[inline]
fn fnv_byte(h: &mut u64, b: u8) {
    *h = (*h ^ (b as u64)).wrapping_mul(FNV_PRIME);
}

#[inline]
fn fnv_bytes(h: &mut u64, bytes: &[u8]) {
    for &b in bytes {
        fnv_byte(h, b);
    }
    fnv_byte(h, 0xff);
}

fn hash_non_trivia_tokens(st: &SyntaxTree<'_>, node: NodeId, h: &mut u64) {
    for tok in st.tokens_of(node) {
        if !tok.is_trivia() {
            fnv_byte(h, tok.kind as u8);
            fnv_bytes(h, tok.text(st.src).as_bytes());
        }
    }
    fnv_byte(h, 0xfe);
}

/// Computes a 64-bit structural fingerprint of ONLY module-tree-affecting items in `st`:
/// inner `#![...]` attributes, enabled `mod` headers (recursing into inline `mod { ... }`),
/// enabled `macro_rules!`, and enabled item-position macro calls (`cfg_rt! { mod ... }`, etc.).
pub(super) fn file_mod_fingerprint(st: &SyntaxTree<'_>, db: &dyn ItemDb, krate: CrateId) -> u64 {
    let mut h = FNV_OFFSET;
    let mut inner = EffectiveAttrs::default();
    eval_inner_attrs(st, st.root(), db, krate, &mut inner);
    fnv_byte(&mut h, inner.enabled as u8);
    fnv_byte(&mut h, inner.no_std as u8);
    fnv_byte(&mut h, inner.no_core as u8);
    fnv_byte(&mut h, inner.no_implicit_prelude as u8);
    if inner.enabled {
        hash_container_mod_items(st, st.root(), db, krate, &mut h);
    }
    h
}

fn hash_container_mod_items(st: &SyntaxTree<'_>, container: NodeId, db: &dyn ItemDb, krate: CrateId, h: &mut u64) {
    for ch in st.children(container) {
        match st.kind(ch) {
            K::Module => {
                let outer = eval_outer_attrs(st, ch, db, krate);
                if !outer.enabled {
                    continue;
                }
                fnv_byte(h, 1);
                match outer.explicit_path.as_deref() {
                    Some(p) => {
                        fnv_byte(h, 1);
                        fnv_bytes(h, p.as_bytes());
                    }
                    None => fnv_byte(h, 0),
                }
                fnv_byte(h, outer.macro_use as u8);
                if let Some(flt) = &outer.macro_use_filter {
                    fnv_byte(h, 1);
                    for s in flt {
                        fnv_bytes(h, s.as_bytes());
                    }
                } else {
                    fnv_byte(h, 0);
                }
                fnv_byte(h, outer.no_implicit_prelude as u8);
                let (vis, _) = extract_visibility(st, ch);
                fnv_byte(h, vis as u8);
                if let Some(nn) = st.child(ch, K::Name) {
                    fnv_bytes(h, norm_ident(st.text(nn)).as_bytes());
                }
                if let Some(il) = st.child(ch, K::ItemList) {
                    fnv_byte(h, 1);
                    let mut il_in = EffectiveAttrs::default();
                    eval_inner_attrs(st, il, db, krate, &mut il_in);
                    fnv_byte(h, il_in.enabled as u8);
                    fnv_byte(h, il_in.no_implicit_prelude as u8);
                    if il_in.enabled {
                        hash_container_mod_items(st, il, db, krate, h);
                    }
                } else {
                    fnv_byte(h, 0);
                }
            }
            K::MacroRules => {
                let outer = eval_outer_attrs(st, ch, db, krate);
                if !outer.enabled {
                    continue;
                }
                fnv_byte(h, 2);
                fnv_byte(h, outer.macro_export as u8);
                if let Some(nn) = st.child(ch, K::Name) {
                    fnv_bytes(h, norm_ident(st.text(nn)).as_bytes());
                }
                if let Some(tt) = st.child(ch, K::TokenTree) {
                    hash_non_trivia_tokens(st, tt, h);
                }
            }
            K::MacroCall => {
                let outer = eval_outer_attrs(st, ch, db, krate);
                if !outer.enabled {
                    continue;
                }
                fnv_byte(h, 3);
                if let Some(p) = st.child(ch, K::Path) {
                    hash_non_trivia_tokens(st, p, h);
                }
                if let Some(tt) = st.child(ch, K::TokenTree) {
                    hash_non_trivia_tokens(st, tt, h);
                }
            }
            _ => {}
        }
    }
}

pub(super) fn mod_trees_structurally_equal(a: &ModuleTree, b: &ModuleTree) -> bool {
    if a.krate != b.krate || a.edition != b.edition || a.no_std != b.no_std || a.no_core != b.no_core || a.modules.len() != b.modules.len() {
        return false;
    }
    for (ma, mb) in a.modules.iter().zip(b.modules.iter()) {
        if ma.parent != mb.parent
            || ma.file != mb.file
            || ma.is_inline() != mb.is_inline()
            || ma.vis != mb.vis
            || ma.flags != mb.flags
            || a.name(ma.id) != b.name(mb.id)
            || a.crate_path(ma.id) != b.crate_path(mb.id)
        {
            return false;
        }
    }
    true
}
