/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Unified inherent and trait `impl` header (`ImplEntry`) and associated type binding
//! (`AssocTyEntry`) extraction from `TyCtxt` across workspace, dependency, and sysroot shards.

use ridx_format::{IndexWriter, RawAssocTy, RawImpl, TyId, IMPL_FLAG_CONST, IMPL_FLAG_NEGATIVE, IMPL_FLAG_RESERVATION, IMPL_FLAG_TRAIT, IMPL_FLAG_UNSAFE};
use rustc_hir as hir;
use rustc_hir::def::DefKind;
use rustc_middle::ty;
use rustc_span::def_id::DefId;

use super::{build_raw_generic_args, TcxTypeLowerer};

impl<'tcx, 'a> TcxTypeLowerer<'tcx, 'a> {
    /// Emits structured `ImplEntry` and `AssocTyEntry` rows for every `impl` block in `impl_dids`.
    ///
    /// Never applies `peel_refs` or filters blanket `Param`, `&T`, `&mut T`, tuples, `fn` pointers,
    /// `!`, or marker/auto/unstable traits.
    pub(crate) fn record_impls(&mut self, impl_dids: &[DefId], writer: &mut IndexWriter) {
        for &impl_did in impl_dids {
            self.record_impl(impl_did, writer);
        }
    }

    pub(crate) fn record_impl(&mut self, impl_did: DefId, writer: &mut IndexWriter) {
        let DefKind::Impl { of_trait } = self.tcx.def_kind(impl_did) else {
            return;
        };
        let def_index = self.local_def_indices.get(&impl_did).copied();
        let self_ty = self.lower_ty(self.tcx.type_of(impl_did).skip_binder(), writer);

        let (trait_def, owned_args, arg_exprs, flags) = if of_trait {
            let hdr = self.tcx.impl_trait_header(impl_did);
            let tr = hdr.trait_ref.skip_binder();
            let t_def = self.lower_def(tr.def_id, writer);
            let args_after_self = if tr.args.is_empty() { &[][..] } else { &tr.args[1..] };
            let (oa, ex) = self.lower_generic_args(args_after_self, writer);
            let mut fl = IMPL_FLAG_TRAIT;
            match hdr.polarity {
                ty::ImplPolarity::Positive => {}
                ty::ImplPolarity::Negative => fl |= IMPL_FLAG_NEGATIVE,
                ty::ImplPolarity::Reservation => fl |= IMPL_FLAG_RESERVATION,
            }
            if hdr.safety.is_unsafe() {
                fl |= IMPL_FLAG_UNSAFE;
            }
            if matches!(hdr.constness, hir::Constness::Const { .. }) {
                fl |= IMPL_FLAG_CONST;
            }
            (Some(t_def), oa, ex, fl)
        } else {
            (None, Vec::new(), Vec::new(), 0u8)
        };
        let raw_trait_args = build_raw_generic_args(&owned_args, &arg_exprs);

        let item_dids = self.tcx.associated_item_def_ids(impl_did);
        let mut owned_assocs: Vec<(Option<u32>, String, TyId)> = Vec::new();
        let mut assoc_defs: Vec<u32> = Vec::new();

        for &assoc_did in item_dids {
            match self.tcx.def_kind(assoc_did) {
                DefKind::AssocTy => {
                    // Guard 1: RPITIT synthesizes anonymous `AnonAssocTy` with no identifier.
                    let Some(name_sym) = self.tcx.opt_item_name(assoc_did) else {
                        continue;
                    };
                    // Guard 2: Specialization impls in `core`/`alloc` can declare `default type Foo;` with no RHS.
                    if !self.tcx.defaultness(assoc_did).has_value() {
                        continue;
                    }
                    let rhs = self.lower_ty(self.tcx.type_of(assoc_did).skip_binder(), writer);
                    let a_def_idx = self.local_def_indices.get(&assoc_did).copied();
                    owned_assocs.push((a_def_idx, name_sym.to_string(), rhs));
                }
                DefKind::AssocFn | DefKind::AssocConst { .. } => {
                    if let Some(&d_idx) = self.local_def_indices.get(&assoc_did) {
                        assoc_defs.push(d_idx);
                    }
                }
                _ => {}
            }
        }

        let raw_assocs: Vec<RawAssocTy<'_>> = owned_assocs.iter().map(|(def_index, name, ty)| RawAssocTy { def_index: *def_index, name: name.as_str(), ty: *ty }).collect();

        let pending_idx = writer.add_impl(RawImpl { def_index, self_ty, trait_def, trait_args: &raw_trait_args, assoc_tys: &raw_assocs, assoc_defs: &assoc_defs, flags });
        self.record_impl_generics_and_preds(impl_did, pending_idx, writer);
    }
}
