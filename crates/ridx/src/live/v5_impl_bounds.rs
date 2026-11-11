/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! The trait bounds of the `impl` a method candidate comes from, lowered against the same substitution order as
//! [`RidxV5ItemDb::v5_check_blanket_method_bounds`], so the inference side can check the ones that land on its own
//! type parameters.

use ridx_format::WherePredKind;
use ridx_syntax::ty_infer::uf_table::InlineTyBuf;
use ridx_syntax::ty_infer::{InferTyArena, MethodCand, TyLangItem, WhereObligation};

use super::v5_db::RidxV5ItemDb;
use super::v5_ty_lower::{lower_generic_args, lower_v5_ty, ParamRemap};

impl<'w> RidxV5ItemDb<'w> {
    pub fn v5_impl_trait_bounds(&self, cand: &MethodCand, arena: &mut InferTyArena, out: &mut Vec<WhereObligation>) {
        let (r_idx, impl_idx) = if cand.impl_owner.is_some() && !cand.impl_owner.is_overlay() {
            (cand.impl_owner.krate.0 as usize, cand.impl_owner.index)
        } else {
            let mdef = self.canonical_def(cand.method_def);
            let Some(idx) = self.shards.get(mdef.krate.0 as usize).and_then(|s| s.lookup_assoc_impl(mdef.index)) else { return };
            (mdef.krate.0 as usize, idx)
        };
        let Some(reader) = self.ws.readers().get(r_idx) else { return };
        let va = reader.ty_arena();
        let remap = ParamRemap::for_impl(self, r_idx, impl_idx, arena);
        for wp in reader.where_preds_for_impl(impl_idx) {
            if wp.kind() != WherePredKind::TraitBound || wp.is_negative() {
                continue;
            }
            let Some(td) = wp.trait_def() else { continue };
            let tgid = self.ty_def_gid(r_idx, td);
            if !tgid.is_some()
                || (tgid.index & super::v5_db::TY_DEF_SYNTH_BIT) != 0
                || matches!(self.ty_tables.gid_to_lang.get(&self.canonical_def(tgid)).copied(), Some(TyLangItem::Sized | TyLangItem::Sync | TyLangItem::Freeze | TyLangItem::Unpin))
            {
                continue;
            }
            let lhs_ty = lower_v5_ty(self, r_idx, va, wp.lhs_ty(), &remap, arena);
            let mut targs = InlineTyBuf::new();
            lower_generic_args(self, r_idx, va, wp.trait_args(), &remap, arena, &mut targs, 1);
            let trait_args = arena.intern_slice(targs.as_slice());
            out.push(WhereObligation::TraitBound { lhs_ty, trait_def: tgid, trait_args });
        }
    }
}
