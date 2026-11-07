/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Arena data types and `extern crate` lowerer for [`UseArena`] (nested `use` lowering lives in
//! [`super::use_lower`]).

use super::attr::{eval_outer_attrs, extract_visibility};
use super::db::{CrateId, ItemDb, ModuleId, VisKind};
use super::mod_tree::ModuleTree;
use super::per_ns::compute_vis_mod;
use super::use_lower::body_tokens_after_headers;
use crate::sema_fn::norm_ident;
use crate::token::{Token, TokenKind as T};
use crate::tree::{NodeId, SyntaxTree};

/// Leading root anchor of a `use` or multi-segment item path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathAnchor {
    /// Bare `foo::bar` (uniform path in 2018+, crate-root relative in `use` in 2015).
    Bare,
    /// Leading `crate::...` (or `use crate as alias;` when segment list is empty).
    CrateRoot,
    /// Leading `self::...` (or `use self as alias;`).
    SelfMod,
    /// Leading `super::` chain of length `count >= 1`.
    Super(u16),
    /// Leading `::foo::bar` (extern crate prelude in 2018+, crate root in 2015).
    ExternRoot,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlatUseKind {
    Named = 0,
    Underscore = 1,
    Glob = 2,
}

/// One flattened leaf of a `use` tree stored inside [`UseArena`].
#[derive(Clone, Copy, Debug)]
pub struct FlatUse {
    pub module: ModuleId,
    pub anchor: PathAnchor,
    pub seg_start: u32,
    pub seg_len: u16,
    pub bind_start: u32,
    pub bind_end: u32,
    pub vis: VisKind,
    pub vis_mod: ModuleId,
    pub kind: FlatUseKind,
    pub span_start: u32,
    pub span_end: u32,
}

/// One flattened `extern crate` item.
#[derive(Clone, Copy, Debug)]
pub struct FlatExternCrate {
    pub module: ModuleId,
    pub is_self: bool,
    pub is_underscore: bool,
    pub macro_use: bool,
    pub orig_start: u32,
    pub orig_end: u32,
    pub bind_start: u32,
    pub bind_end: u32,
    pub vis: VisKind,
    pub vis_mod: ModuleId,
}

/// Contiguous arena storing all flattened `use` leaves and `extern crate` declarations of a crate.
#[derive(Clone, Debug, Default)]
pub struct UseArena {
    pub(super) strings: String,
    pub(super) segs: Vec<(u32, u32)>,
    pub uses: Vec<FlatUse>,
    pub extern_crates: Vec<FlatExternCrate>,
}

impl UseArena {
    pub fn with_capacity(est_uses: usize) -> Self {
        Self { strings: String::with_capacity(est_uses * 16), segs: Vec::with_capacity(est_uses * 3), uses: Vec::with_capacity(est_uses), extern_crates: Vec::new() }
    }

    #[inline]
    pub(super) fn push_str(&mut self, s: &str) -> (u32, u32) {
        let start = self.strings.len() as u32;
        self.strings.push_str(s);
        (start, self.strings.len() as u32)
    }

    #[inline]
    pub fn str_span(&self, range: (u32, u32)) -> &str {
        &self.strings[range.0 as usize..range.1 as usize]
    }

    #[inline]
    pub fn bind_name(&self, u: &FlatUse) -> &str {
        &self.strings[u.bind_start as usize..u.bind_end as usize]
    }

    #[inline]
    pub fn seg_spans(&self, u: &FlatUse) -> &[(u32, u32)] {
        let s = u.seg_start as usize;
        &self.segs[s..s + u.seg_len as usize]
    }

    #[inline]
    pub fn extern_orig(&self, ec: &FlatExternCrate) -> &str {
        &self.strings[ec.orig_start as usize..ec.orig_end as usize]
    }

    #[inline]
    pub fn extern_bind(&self, ec: &FlatExternCrate) -> &str {
        &self.strings[ec.bind_start as usize..ec.bind_end as usize]
    }

    /// Evaluates attributes + visibility and flattens a `K::ExternCrate` node.
    pub fn lower_extern_crate(&mut self, st: &SyntaxTree<'_>, node: NodeId, mod_tree: &ModuleTree, module: ModuleId, db: &dyn ItemDb, krate: CrateId) {
        let attrs = eval_outer_attrs(st, node, db, krate);
        if !attrs.enabled {
            return;
        }
        let (raw_vis, in_path) = extract_visibility(st, node);
        let (vis, vis_mod) = compute_vis_mod(mod_tree, module, raw_vis, in_path);
        let sig: Vec<Token> = body_tokens_after_headers(st, node).iter().copied().filter(|t| !t.is_trivia() && t.kind != T::Semi).collect();
        let Some(crate_pos) = sig.iter().position(|t| t.kind == T::Crate) else { return };
        let Some(&target_tok) = sig.get(crate_pos + 1) else { return };
        let (is_self, orig_str) = match target_tok.kind {
            T::SelfKw => (true, "self"),
            T::Ident | T::RawIdent => (false, norm_ident(target_tok.text(st.src))),
            _ => return,
        };
        let mut is_underscore = false;
        let bind_str = if sig.get(crate_pos + 2).map(|t| t.kind) == Some(T::As) {
            match sig.get(crate_pos + 3) {
                Some(t) if t.kind == T::Underscore => {
                    is_underscore = true;
                    "_"
                }
                Some(t) if matches!(t.kind, T::Ident | T::RawIdent) => norm_ident(t.text(st.src)),
                _ => return,
            }
        } else if is_self {
            return;
        } else {
            orig_str
        };
        let (orig_start, orig_end) = self.push_str(orig_str);
        let (bind_start, bind_end) = self.push_str(bind_str);
        self.extern_crates.push(FlatExternCrate { module, is_self, is_underscore, macro_use: attrs.macro_use, orig_start, orig_end, bind_start, bind_end, vis, vis_mod });
    }
}
