/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Pattern walker (`walk_pat`) and record/struct field owner resolution for [`FileResolver`].

use super::body_ribs::norm_byte_span;
use super::body_walk::FileResolver;
use super::db::{GlobalDefId, ItemKind};
use super::db_impls::ImplTargetKey;
use super::path_res::{PathCtx, ResTarget};
use super::per_ns::LocalItemId;
use crate::sema_fn::norm_ident;
use crate::syntax_kind::SyntaxKind as K;
use crate::token::TokenKind as T;
use crate::tree::NodeId;

impl<'a, 's> FileResolver<'a, 's> {
    pub(super) fn walk_pat(&mut self, pat: NodeId, or_base: Option<usize>, force_local: bool) {
        match self.st.kind(pat) {
            K::OrPat => {
                let mark = self.ribs.or_pat_scratch.len();
                let base = or_base.unwrap_or(mark);
                for alt in self.st.children(pat).filter(|&c| self.st.kind(c).is_pat()) {
                    self.walk_pat(alt, Some(base), false);
                }
                if or_base.is_none() {
                    self.ribs.or_pat_scratch.truncate(mark);
                }
            }
            K::IdentPat => {
                let has_ref_mut = self.st.own_tokens(pat).any(|(_, t)| matches!(t.kind, T::Ref | T::Mut));
                let is_mut = self.st.own_tokens(pat).any(|(_, t)| t.kind == T::Mut);
                let subpat = self.st.children(pat).find(|&c| self.st.kind(c).is_pat());
                if let Some(nn) = self.st.child(pat, K::Name) {
                    let r = self.st.range(nn);
                    let (ns, ne) = norm_byte_span(self.st.src, r.clone());
                    let norm_name = &self.st.src[ns as usize..ne as usize];
                    if !force_local && !has_ref_mut && subpat.is_none() {
                        if let Some(item_rt) = self.resolve_bare_ident_pat_item(norm_name) {
                            self.emit_ref(r.start as u32, r.end as u32, NodeId(u32::MAX), item_rt, PathCtx::PatValue, true);
                            return;
                        }
                    }
                    if norm_name != "_" {
                        let existing = or_base
                            .and_then(|base| self.ribs.or_pat_scratch[base..].iter().find(|&&(s, e, _)| &self.st.src[s as usize..e as usize] == norm_name).map(|&(_, _, id)| id));
                        if existing.is_none() {
                            let lid = self.alloc_local(pat, r.start as u32, r.end as u32, false, is_mut);
                            self.ribs.push_local(ns, ne, lid);
                            if or_base.is_some() {
                                self.ribs.or_pat_scratch.push((ns, ne, lid));
                            }
                        }
                    }
                }
                if let Some(sp) = subpat {
                    self.walk_pat(sp, or_base, false);
                }
            }
            K::PathPat => {
                if let Some(p) = self.st.child(pat, K::Path) {
                    self.resolve_cst_path(p, PathCtx::PatValue);
                }
            }
            K::TupleStructPat => {
                if let Some(p) = self.st.child(pat, K::Path) {
                    self.resolve_cst_path(p, PathCtx::PatValue);
                }
                for sp in self.st.children(pat).filter(|&c| self.st.kind(c).is_pat()) {
                    self.walk_pat(sp, or_base, false);
                }
            }
            K::RecordPat => {
                let owner_def =
                    self.st.child(pat, K::Path).and_then(|p| self.resolve_cst_path(p, PathCtx::PatRecord)).map_or(GlobalDefId::NONE, |rt| self.adt_or_variant_def_from_target(rt));
                for ch in self.st.children(pat) {
                    if self.st.kind(ch) == K::RecordPatField {
                        if !self.expr_cfg_enabled(ch) {
                            continue;
                        }
                        let has_colon = self.st.own_tokens(ch).any(|(_, t)| t.kind == T::Colon);
                        if has_colon && owner_def.is_some() {
                            if let Some((_, ftok)) = self.st.own_tokens(ch).find(|(_, t)| matches!(t.kind, T::Ident | T::RawIdent | T::Int)) {
                                if let Some(fdef) = self.db.adt_field(owner_def, norm_ident(ftok.text(self.st.src))) {
                                    let rt = ResTarget::Def { def: fdef, local: LocalItemId::NONE, kind: ItemKind::Field };
                                    self.emit_ref(ftok.start, ftok.end, NodeId(u32::MAX), rt, PathCtx::PatRecord, false);
                                }
                            }
                        }
                        if let Some(sp) = self.st.children(ch).find(|&c| self.st.kind(c).is_pat()) {
                            self.walk_pat(sp, or_base, !has_colon);
                        }
                    }
                }
            }
            K::RangePat => {
                if let Some(nn) = self.st.child(pat, K::Name) {
                    let r = self.st.range(nn);
                    if let Some(rt) = self.resolve_bare_ident_pat_item(norm_ident(self.st.text(nn))) {
                        self.emit_ref(r.start as u32, r.end as u32, NodeId(u32::MAX), rt, PathCtx::PatValue, true);
                    }
                }
                for sp in self.st.children(pat).filter(|&c| self.st.kind(c).is_pat()) {
                    self.walk_pat(sp, or_base, false);
                }
            }
            K::ConstBlockPat => {
                let lb = self.ribs.enter_assoc_item_barrier();
                if let Some(blk) = self.st.child(pat, K::BlockExpr) {
                    self.walk_block_expr(blk, false);
                }
                self.ribs.local_barrier = lb;
            }
            K::MacroPat => {
                if let Some(p) = self.st.child(pat, K::Path) {
                    self.resolve_cst_path(p, PathCtx::MacroCall);
                }
            }
            _ => {
                for sp in self.st.children(pat).filter(|&c| self.st.kind(c).is_pat()) {
                    self.walk_pat(sp, or_base, false);
                }
            }
        }
    }

    pub(super) fn adt_or_variant_def_from_target(&self, rt: ResTarget) -> GlobalDefId {
        match rt {
            ResTarget::Def { def, kind: ItemKind::Struct | ItemKind::Union | ItemKind::Variant, .. } => def,
            ResTarget::Def { def, kind: ItemKind::TypeAlias, .. } if def.is_some() => match self.map.resolve_alias_target(self.db, def) {
                Some(ImplTargetKey::Adt(adt_def)) => adt_def,
                _ => GlobalDefId::NONE,
            },
            ResTarget::SelfTy { target: ImplTargetKey::Adt(adt_def), .. } => adt_def,
            _ => GlobalDefId::NONE,
        }
    }
}
