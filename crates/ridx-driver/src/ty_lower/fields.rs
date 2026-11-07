/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Unified ADT/variant field (`FieldEntry`), constructor (`CtorShape` + tuple ctor `FnSigEntry`),
//! `TyAlias` RHS, and `Const`/`Static`/`AssocConst` (`DefTyEntry`) extraction across
//! workspace crates, external dependencies, and sysroot.

use ridx_format::{
    CtorShape, FnAbiKind, IndexWriter, ItemVis, RawDefTy, RawField, RawFnSig, TyId, DEF_TY_FLAG_FUNDAMENTAL, DEF_TY_FLAG_MUT_STATIC, DEF_TY_FLAG_NON_EXHAUSTIVE,
    DEF_TY_FLAG_UNSAFE_STATIC, FIELD_FLAG_TUPLE, FIELD_FLAG_UNSAFE, FN_FLAG_CONST,
};
use rustc_hir as hir;
use rustc_hir::def::{CtorKind, DefKind};
use rustc_middle::ty::{self, TyCtxt, VariantDef};
use rustc_span::def_id::{DefId, LocalDefId};

use super::TcxTypeLowerer;

/// Maps `rustc`'s semantic `Visibility<DefId>` onto `.ridx`'s 4-level `ItemVis`
/// (`0 = Public, 1 = Crate, 2 = Restricted, 3 = Private`).
pub(crate) fn lower_item_vis(tcx: TyCtxt<'_>, item_did: DefId, vis: ty::Visibility<DefId>) -> ItemVis {
    let ty::Visibility::Restricted(vis_mod) = vis else {
        return ItemVis::Public;
    };
    if !vis_mod.is_crate_root() && tcx.def_kind(vis_mod) != DefKind::Mod {
        return ItemVis::Private;
    }
    let mut enc_mod = item_did;
    while !enc_mod.is_crate_root() {
        enc_mod = tcx.parent(enc_mod);
        if tcx.def_kind(enc_mod) == DefKind::Mod {
            break;
        }
    }
    if !enc_mod.is_crate_root() {
        if vis_mod.is_crate_root() {
            ItemVis::Crate
        } else if vis_mod == enc_mod {
            ItemVis::Private
        } else {
            ItemVis::Restricted
        }
    } else if let Some(local_did) = item_did.as_local() {
        match hir_vis_span(tcx, local_did) {
            Some(sp) if !sp.is_empty() => {
                if tcx.sess.source_map().span_to_snippet(sp).is_ok_and(|s| s.contains("self")) {
                    ItemVis::Private
                } else {
                    ItemVis::Crate
                }
            }
            _ => ItemVis::Private,
        }
    } else {
        ItemVis::Private
    }
}

/// Safely computes `ItemVis` for any `DefEntry` candidate `did` without triggering
/// `tcx.visibility` panics on `Impl`, generic parameters, closures, or nested statics.
pub(crate) fn def_visibility(tcx: TyCtxt<'_>, did: DefId, kind: DefKind) -> ItemVis {
    if did.is_crate_root() {
        return ItemVis::Public;
    }
    match kind {
        DefKind::Mod
        | DefKind::Struct
        | DefKind::Union
        | DefKind::Enum
        | DefKind::Variant
        | DefKind::Trait
        | DefKind::TyAlias
        | DefKind::ForeignTy
        | DefKind::TraitAlias
        | DefKind::Use
        | DefKind::Fn
        | DefKind::Const { .. }
        | DefKind::Static { nested: false, .. }
        | DefKind::Ctor(..)
        | DefKind::AssocFn
        | DefKind::AssocConst { .. }
        | DefKind::Macro(..)
        | DefKind::Field
        | DefKind::ExternCrate => {}
        DefKind::AssocTy => {
            if tcx.is_impl_trait_in_trait(did) {
                let parent = tcx.parent(did);
                return def_visibility(tcx, parent, tcx.def_kind(parent));
            }
        }
        _ => return ItemVis::Private,
    }
    let owner_did = match kind {
        DefKind::Variant => tcx.parent(did),
        DefKind::Field => {
            let parent = tcx.parent(did);
            if tcx.def_kind(parent) == DefKind::Variant {
                tcx.parent(parent)
            } else {
                did
            }
        }
        DefKind::AssocFn | DefKind::AssocConst { .. } | DefKind::AssocTy => {
            let parent = tcx.parent(did);
            match tcx.def_kind(parent) {
                DefKind::Trait => parent,
                DefKind::Impl { of_trait: true } => tcx.impl_trait_ref(parent).skip_binder().def_id,
                _ => did,
            }
        }
        _ => did,
    };
    lower_item_vis(tcx, owner_did, tcx.visibility(did))
}

fn hir_vis_span(tcx: TyCtxt<'_>, local_did: LocalDefId) -> Option<rustc_span::Span> {
    let hir_id = tcx.local_def_id_to_hir_id(local_did);
    match tcx.hir_node(hir_id) {
        hir::Node::Field(f) => Some(f.vis_span),
        hir::Node::Item(item) => Some(item.vis_span),
        hir::Node::ForeignItem(fi) => Some(fi.vis_span),
        hir::Node::ImplItem(ii) => match ii.impl_kind {
            hir::ImplItemImplKind::Inherent { vis_span } => Some(vis_span),
            hir::ImplItemImplKind::Trait { .. } => None,
        },
        _ => None,
    }
}

#[inline]
fn map_ctor_shape(ctor: Option<(CtorKind, DefId)>) -> CtorShape {
    match ctor {
        None => CtorShape::Record,
        Some((CtorKind::Fn, _)) => CtorShape::Tuple,
        Some((CtorKind::Const, _)) => CtorShape::Unit,
    }
}

impl<'tcx, 'a> TcxTypeLowerer<'tcx, 'a> {
    /// Emits `FieldEntry`, `DefTyEntry`, and tuple-ctor `FnSigEntry` rows for primary ADTs
    /// (`Struct`, `Union`, `Enum`, and all indexed `Enum` `Variant`s).
    pub(crate) fn record_adts(&mut self, adts: &[(DefId, u32)], writer: &mut IndexWriter) {
        for &(did, def_idx) in adts {
            self.record_adt(did, def_idx, writer);
        }
    }

    fn record_adt(&mut self, did: DefId, def_idx: u32, writer: &mut IndexWriter) {
        let adt_def = self.tcx.adt_def(did);
        let self_ty = self.lower_ty(self.tcx.type_of(did).skip_binder(), writer);
        let mut adt_flags = 0u8;
        if adt_def.is_fundamental() {
            adt_flags |= DEF_TY_FLAG_FUNDAMENTAL;
        }
        if adt_def.is_enum() {
            if adt_def.is_variant_list_non_exhaustive() {
                adt_flags |= DEF_TY_FLAG_NON_EXHAUSTIVE;
            }
            writer.add_def_ty(RawDefTy { def_index: def_idx, ty: self_ty, parent_def_index: None, variant_idx: None, ctor_shape: CtorShape::None, flags: adt_flags });
            for (var_idx, variant) in adt_def.variants().iter_enumerated() {
                let Some(&var_def_idx) = self.local_def_indices.get(&variant.def_id) else {
                    continue;
                };
                let v_idx = u16::try_from(var_idx.as_u32()).unwrap_or(u16::MAX);
                self.record_variant_fields_and_ctor(variant, var_def_idx, Some(def_idx), v_idx, self_ty, 0, writer);
            }
        } else {
            let variant = adt_def.non_enum_variant();
            self.record_variant_fields_and_ctor(variant, def_idx, None, 0, self_ty, adt_flags, writer);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn record_variant_fields_and_ctor(
        &mut self,
        variant: &'tcx VariantDef,
        owner_def_idx: u32,
        parent_def_index: Option<u32>,
        variant_idx: u16,
        adt_self_ty: TyId,
        base_flags: u8,
        writer: &mut IndexWriter,
    ) {
        let ctor_shape = map_ctor_shape(variant.ctor);
        let mut flags = base_flags;
        if variant.is_field_list_non_exhaustive() {
            flags |= DEF_TY_FLAG_NON_EXHAUSTIVE;
        }
        writer.add_def_ty(RawDefTy { def_index: owner_def_idx, ty: adt_self_ty, parent_def_index, variant_idx: Some(variant_idx), ctor_shape, flags });

        let is_tuple = matches!(ctor_shape, CtorShape::Tuple);
        let mut field_tys = std::mem::take(&mut self.scratch_inputs);
        field_tys.clear();
        field_tys.reserve(variant.fields.len());

        for (f_idx, field) in variant.fields.iter_enumerated() {
            let f_ty = self.lower_ty(self.tcx.type_of(field.did).skip_binder(), writer);
            field_tys.push(f_ty);
            let vis = lower_item_vis(self.tcx, field.did, field.vis);
            let mut f_flags = 0u8;
            if is_tuple {
                f_flags |= FIELD_FLAG_TUPLE;
            }
            if field.safety.is_unsafe() {
                f_flags |= FIELD_FLAG_UNSAFE;
            }
            let f_def_idx = self.local_def_indices.get(&field.did).copied();
            let field_pos = u16::try_from(f_idx.as_u32()).unwrap_or(u16::MAX);
            writer.add_field(RawField { owner_def_index: owner_def_idx, def_index: f_def_idx, name: field.name.as_str(), ty: f_ty, field_idx: field_pos, vis, flags: f_flags });
        }

        if let Some((ctor_kind, ctor_did)) = variant.ctor {
            if ctor_kind == CtorKind::Fn {
                writer.add_fn_sig(RawFnSig { def_index: owner_def_idx, inputs: &field_tys, output: adt_self_ty, abi: FnAbiKind::Rust, flags: FN_FLAG_CONST });
            }
            if let Some(&ctor_def_idx) = self.local_def_indices.get(&ctor_did) {
                if ctor_def_idx != owner_def_idx {
                    writer.add_def_ty(RawDefTy { def_index: ctor_def_idx, ty: adt_self_ty, parent_def_index, variant_idx: Some(variant_idx), ctor_shape, flags });
                    if ctor_kind == CtorKind::Fn {
                        writer.add_fn_sig(RawFnSig { def_index: ctor_def_idx, inputs: &field_tys, output: adt_self_ty, abi: FnAbiKind::Rust, flags: FN_FLAG_CONST });
                    }
                }
            }
        }
        self.scratch_inputs = field_tys;
    }

    /// Emits `DefTyEntry` (and tuple-ctor `FnSigEntry` for re-exported tuple ADTs/variants)
    /// for `TyAlias`, `Const`, `Static`, `AssocConst`, and re-exported `Struct`/`Union`/`Enum`/`Variant`.
    pub(crate) fn record_def_tys(&mut self, items: &[(DefId, u32)], writer: &mut IndexWriter) {
        for &(did, def_idx) in items {
            match self.tcx.def_kind(did) {
                DefKind::TyAlias | DefKind::Const { .. } | DefKind::AssocConst { .. } => {
                    let ty = self.lower_ty(self.tcx.type_of(did).skip_binder(), writer);
                    writer.add_def_ty(RawDefTy { def_index: def_idx, ty, parent_def_index: None, variant_idx: None, ctor_shape: CtorShape::None, flags: 0 });
                }
                DefKind::Static { safety, mutability, .. } => {
                    let ty = self.lower_ty(self.tcx.type_of(did).skip_binder(), writer);
                    let mut flags = 0u8;
                    if matches!(mutability, hir::Mutability::Mut) {
                        flags |= DEF_TY_FLAG_MUT_STATIC;
                    }
                    if safety.is_unsafe() {
                        flags |= DEF_TY_FLAG_UNSAFE_STATIC;
                    }
                    writer.add_def_ty(RawDefTy { def_index: def_idx, ty, parent_def_index: None, variant_idx: None, ctor_shape: CtorShape::None, flags });
                }
                DefKind::Struct | DefKind::Union | DefKind::Enum => {
                    self.record_reexported_adt(did, def_idx, writer);
                }
                DefKind::Variant => {
                    self.record_reexported_variant(did, def_idx, writer);
                }
                _ => {}
            }
        }
    }

    fn record_reexported_adt(&mut self, did: DefId, def_idx: u32, writer: &mut IndexWriter) {
        let adt_def = self.tcx.adt_def(did);
        let self_ty = self.lower_ty(self.tcx.type_of(did).skip_binder(), writer);
        let mut flags = if adt_def.is_fundamental() { DEF_TY_FLAG_FUNDAMENTAL } else { 0 };
        if adt_def.is_enum() {
            if adt_def.is_variant_list_non_exhaustive() {
                flags |= DEF_TY_FLAG_NON_EXHAUSTIVE;
            }
            writer.add_def_ty(RawDefTy { def_index: def_idx, ty: self_ty, parent_def_index: None, variant_idx: None, ctor_shape: CtorShape::None, flags });
        } else {
            let variant = adt_def.non_enum_variant();
            let ctor_shape = map_ctor_shape(variant.ctor);
            if variant.is_field_list_non_exhaustive() {
                flags |= DEF_TY_FLAG_NON_EXHAUSTIVE;
            }
            writer.add_def_ty(RawDefTy { def_index: def_idx, ty: self_ty, parent_def_index: None, variant_idx: Some(0), ctor_shape, flags });
            if matches!(variant.ctor, Some((CtorKind::Fn, _))) {
                let inputs: Vec<TyId> = variant.fields.iter().map(|f| self.lower_ty(self.tcx.type_of(f.did).skip_binder(), writer)).collect();
                writer.add_fn_sig(RawFnSig { def_index: def_idx, inputs: &inputs, output: self_ty, abi: FnAbiKind::Rust, flags: FN_FLAG_CONST });
            }
        }
    }

    fn record_reexported_variant(&mut self, var_did: DefId, def_idx: u32, writer: &mut IndexWriter) {
        let enum_did = self.tcx.parent(var_did);
        let is_primary_var = self.local_def_indices.get(&var_did) == Some(&def_idx);
        if is_primary_var && self.local_def_indices.contains_key(&enum_did) {
            return;
        }
        let adt_def = self.tcx.adt_def(enum_did);
        let variant = adt_def.variant_with_id(var_did);
        let enum_ty = self.lower_ty(self.tcx.type_of(enum_did).skip_binder(), writer);
        let var_idx = adt_def.variants().iter_enumerated().find_map(|(i, v)| (v.def_id == var_did).then(|| u16::try_from(i.as_u32()).unwrap_or(u16::MAX))).unwrap_or(0);
        if is_primary_var {
            self.record_variant_fields_and_ctor(variant, def_idx, None, var_idx, enum_ty, 0, writer);
            return;
        }
        let ctor_shape = map_ctor_shape(variant.ctor);
        let flags = if variant.is_field_list_non_exhaustive() { DEF_TY_FLAG_NON_EXHAUSTIVE } else { 0 };
        let parent_def_index = self.local_def_indices.get(&enum_did).copied();
        writer.add_def_ty(RawDefTy { def_index: def_idx, ty: enum_ty, parent_def_index, variant_idx: Some(var_idx), ctor_shape, flags });
        if matches!(variant.ctor, Some((CtorKind::Fn, _))) {
            let inputs: Vec<TyId> = variant.fields.iter().map(|f| self.lower_ty(self.tcx.type_of(f.did).skip_binder(), writer)).collect();
            writer.add_fn_sig(RawFnSig { def_index: def_idx, inputs: &inputs, output: enum_ty, abi: FnAbiKind::Rust, flags: FN_FLAG_CONST });
        }
    }
}
