/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Lexical rib stack (`RibStack`) for local variables, generic type/const parameters, `Self` contexts,
//! and inner items/imports inside block expressions (`BlockItemRib`).

use std::collections::HashMap;

use super::db::{GlobalDefId, ItemKind, NsMask};
use super::db_impls::ImplTargetKey;
use super::ident_key::IdentKey;
use super::path_res::{GenericBindId, LocalBindId};
use super::per_ns::{PerNs, ResolvedBinding};
use super::use_tree::UseArena;
use crate::syntax_kind::SyntaxKind as K;
use crate::tree::{NodeId, SyntaxTree};

/// Active `Self` context inside an `impl` block or `trait` definition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SelfCtx {
    pub target: ImplTargetKey,
    pub trait_def: GlobalDefId,
    pub self_has_value_ctor: bool,
}

/// Associated item declared on an `impl` or `trait` inside a block expression.
#[derive(Clone, Copy, Debug)]
pub(super) struct BlockAssocEntry {
    pub target: ImplTargetKey,
    pub trait_def: GlobalDefId,
    pub name_start: u32,
    pub name_end: u32,
    pub def: GlobalDefId,
    pub kind: ItemKind,
}

/// Inner item/import symbol table built lazily only for `BlockExpr`s that declare items.
#[derive(Clone, Debug, Default)]
pub(super) struct BlockItemRib {
    pub entries: HashMap<IdentKey, PerNs>,
    pub unnamed_traits: Vec<ResolvedBinding>,
    pub enum_variants: Vec<(GlobalDefId, IdentKey, ResolvedBinding, NsMask)>,
    pub alias_targets: Vec<(GlobalDefId, ImplTargetKey)>,
    pub assocs: Vec<BlockAssocEntry>,
}

impl BlockItemRib {
    #[inline]
    pub fn clear(&mut self) {
        self.entries.clear();
        self.unnamed_traits.clear();
        self.enum_variants.clear();
        self.alias_targets.clear();
        self.assocs.clear();
    }
}

/// Saved barrier snapshot restored after leaving a nested item inside a body.
#[derive(Clone, Copy)]
pub(super) struct BarrierSave {
    pub local_barrier: usize,
    pub generic_barrier: usize,
    pub self_ctx: Option<SelfCtx>,
}

/// Allocation-minimal lexical rib stack reused across files via [`super::path_res::FileNameRes`].
#[derive(Clone, Debug)]
pub(super) struct RibStack {
    pub locals_active: Vec<(u32, u32, LocalBindId)>,
    pub local_barrier: usize,
    pub generics_active: Vec<(u32, u32, GenericBindId, bool)>,
    pub generic_barrier: usize,
    pub param_bounds: Vec<(GenericBindId, GlobalDefId)>,
    pub self_ctx: Option<SelfCtx>,
    pub block_ribs: Vec<BlockItemRib>,
    pub block_rib_pool: Vec<BlockItemRib>,
    pub use_arena_scratch: UseArena,
    pub use_toks_scratch: Vec<crate::token::Token>,
    pub use_prefix_scratch: Vec<(u32, u32)>,
    pub block_aliases_scratch: Vec<(GlobalDefId, crate::tree::NodeId)>,
    pub or_pat_scratch: Vec<(u32, u32, LocalBindId)>,
    pub extra_traits_scratch: Vec<ResolvedBinding>,
}

impl Default for RibStack {
    fn default() -> Self {
        Self::new()
    }
}

impl RibStack {
    pub fn new() -> Self {
        Self {
            locals_active: Vec::new(),
            local_barrier: 0,
            generics_active: Vec::new(),
            generic_barrier: 0,
            param_bounds: Vec::new(),
            self_ctx: None,
            block_ribs: Vec::new(),
            block_rib_pool: Vec::new(),
            use_arena_scratch: UseArena::default(),
            use_toks_scratch: Vec::new(),
            use_prefix_scratch: Vec::new(),
            block_aliases_scratch: Vec::new(),
            or_pat_scratch: Vec::new(),
            extra_traits_scratch: Vec::new(),
        }
    }

    #[inline]
    pub fn clear(&mut self) {
        self.locals_active.clear();
        self.local_barrier = 0;
        self.generics_active.clear();
        self.generic_barrier = 0;
        self.param_bounds.clear();
        self.self_ctx = None;
        while let Some(mut r) = self.block_ribs.pop() {
            r.clear();
            self.block_rib_pool.push(r);
        }
        self.use_arena_scratch.clear();
        self.use_toks_scratch.clear();
        self.use_prefix_scratch.clear();
        self.block_aliases_scratch.clear();
        self.or_pat_scratch.clear();
        self.extra_traits_scratch.clear();
    }

    #[inline]
    pub fn pop_block_rib(&mut self) {
        if let Some(mut r) = self.block_ribs.pop() {
            r.clear();
            self.block_rib_pool.push(r);
        }
    }

    #[inline]
    pub fn push_local(&mut self, norm_start: u32, norm_end: u32, id: LocalBindId) {
        self.locals_active.push((norm_start, norm_end, id));
    }

    #[inline]
    pub fn lookup_local(&self, src: &str, norm_name: &str) -> Option<LocalBindId> {
        self.locals_active[self.local_barrier..].iter().rev().find(|&&(s, e, _)| &src[s as usize..e as usize] == norm_name).map(|&(_, _, id)| id)
    }

    #[inline]
    pub fn push_generic(&mut self, norm_start: u32, norm_end: u32, id: GenericBindId, is_const: bool) {
        self.generics_active.push((norm_start, norm_end, id, is_const));
    }

    #[inline]
    pub fn lookup_generic(&self, src: &str, norm_name: &str) -> Option<(GenericBindId, bool)> {
        self.generics_active[self.generic_barrier..].iter().rev().find(|&&(s, e, _, _)| &src[s as usize..e as usize] == norm_name).map(|&(_, _, id, is_const)| (id, is_const))
    }

    #[inline]
    pub fn enter_standalone_item_barrier(&mut self) -> BarrierSave {
        let save = BarrierSave { local_barrier: self.local_barrier, generic_barrier: self.generic_barrier, self_ctx: self.self_ctx };
        self.local_barrier = self.locals_active.len();
        self.generic_barrier = self.generics_active.len();
        self.self_ctx = None;
        save
    }

    #[inline]
    pub fn enter_assoc_item_barrier(&mut self) -> usize {
        let prev = self.local_barrier;
        self.local_barrier = self.locals_active.len();
        prev
    }

    #[inline]
    pub fn restore_barrier(&mut self, save: BarrierSave) {
        self.local_barrier = save.local_barrier;
        self.generic_barrier = save.generic_barrier;
        self.self_ctx = save.self_ctx;
    }

    #[inline]
    pub fn refresh_unnamed_traits_scratch(&mut self) {
        self.extra_traits_scratch.clear();
        for rib in &self.block_ribs {
            self.extra_traits_scratch.extend_from_slice(&rib.unnamed_traits);
        }
    }

    pub fn lookup_block_ribs(&self, norm_name: &str) -> PerNs {
        let mut out = PerNs::EMPTY;
        for rib in self.block_ribs.iter().rev() {
            if let Some(&pns) = rib.entries.get(norm_name) {
                out = out.or_else_ns(pns);
                if out.types.is_some() && out.values.is_some() && out.macros.is_some() {
                    break;
                }
            }
        }
        out
    }

    pub fn lookup_block_enum_variant(&self, enum_def: GlobalDefId, norm_name: &str) -> PerNs {
        for rib in self.block_ribs.iter().rev() {
            for (edef, vname, vb, mask) in &rib.enum_variants {
                if *edef == enum_def && vname.as_str() == norm_name {
                    return PerNs::from_binding(*vb, *mask);
                }
            }
        }
        PerNs::EMPTY
    }

    pub fn has_block_enum(&self, enum_def: GlobalDefId) -> bool {
        self.block_ribs.iter().rev().any(|rib| rib.enum_variants.iter().any(|(edef, _, _, _)| *edef == enum_def))
    }

    pub fn lookup_block_alias_target(&self, alias_def: GlobalDefId) -> Option<ImplTargetKey> {
        for rib in self.block_ribs.iter().rev() {
            if let Some(&(_, target)) = rib.alias_targets.iter().find(|&&(d, _)| d == alias_def) {
                return Some(target);
            }
        }
        None
    }

    pub fn lookup_block_assoc(&self, src: &str, target: ImplTargetKey, trait_def: GlobalDefId, norm_name: &str, ns: NsMask) -> Option<(GlobalDefId, ItemKind)> {
        for rib in self.block_ribs.iter().rev() {
            for a in &rib.assocs {
                if a.kind.ns_mask(false).0 & ns.0 == 0 || &src[a.name_start as usize..a.name_end as usize] != norm_name {
                    continue;
                }
                if (trait_def.is_some() && a.trait_def == trait_def && (target == ImplTargetKey::Unknown || a.target == target))
                    || (!trait_def.is_some() && !a.trait_def.is_some() && a.target == target && target != ImplTargetKey::Unknown)
                {
                    return Some((a.def, a.kind));
                }
            }
        }
        None
    }
}

#[inline]
pub(super) fn norm_byte_span(src: &str, r: std::ops::Range<usize>) -> (u32, u32) {
    if r.end.saturating_sub(r.start) >= 2 && &src.as_bytes()[r.start..r.start + 2] == b"r#" {
        ((r.start + 2) as u32, r.end as u32)
    } else {
        (r.start as u32, r.end as u32)
    }
}

#[inline]
pub(super) fn block_has_inner_items(st: &SyntaxTree<'_>, block_node: NodeId) -> bool {
    st.children(block_node).any(|c| {
        matches!(
            st.kind(c),
            K::Use | K::ExternCrate | K::Fn | K::Struct | K::Enum | K::Union | K::Trait | K::Impl | K::TypeAlias | K::Const | K::Static | K::Module | K::MacroRules
        )
    })
}

impl super::body_walk::FileResolver<'_, '_> {
    #[inline]
    pub(super) fn record_mod_trait_scope(&mut self, container: NodeId, m: super::db::ModuleId) {
        let end = NodeId(self.st.node(container).subtree_end);
        let vis = std::sync::Arc::clone(&self.map.scope(m).visible_traits);
        let inc = !self.mod_tree.module(m).has_no_implicit_prelude();
        self.out.mod_trait_scopes.push((container, end, vis, inc));
    }

    pub(super) fn record_top_block_rib_traits(&mut self, container: NodeId) {
        let Some(rib) = self.ribs.block_ribs.last() else {
            return;
        };
        let end = NodeId(self.st.node(container).subtree_end);
        for tb in &rib.unnamed_traits {
            let eff = super::dirty_overlay::effective_local_or_def(tb.def, tb.local);
            if eff.is_some() {
                self.out.block_trait_imports.push((container, end, eff));
            }
        }
        let named_base = self.out.block_trait_imports.len();
        for pns in rib.entries.values() {
            if let Some(tb) = pns.types.filter(|b| !b.ambiguous && b.kind == ItemKind::Trait) {
                let eff = super::dirty_overlay::effective_local_or_def(tb.def, tb.local);
                if eff.is_some() {
                    self.out.block_trait_imports.push((container, end, eff));
                }
            }
        }
        let tail = &mut self.out.block_trait_imports[named_base..];
        if tail.len() > 1 {
            tail.sort_unstable_by_key(|x| x.2);
            let mut w = named_base + 1;
            for r in (named_base + 1)..self.out.block_trait_imports.len() {
                if self.out.block_trait_imports[r].2 != self.out.block_trait_imports[w - 1].2 {
                    self.out.block_trait_imports[w] = self.out.block_trait_imports[r];
                    w += 1;
                }
            }
            self.out.block_trait_imports.truncate(w);
        }
    }
}
