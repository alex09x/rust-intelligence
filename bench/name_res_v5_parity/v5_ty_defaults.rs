/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Default generic type parameter lowering (`T = Default`) and stdlib macro result type
//! construction (`StdMacroTy`) for [`crate::v5_db::RidxV5ItemDb`] (`0` steady-state heap allocs).

use ridx_format::GenericParamKind;
use ridx_syntax::name_res::GlobalDefId;
use ridx_syntax::ty_infer::{InferTyArena, InferTyId, StdMacroTy, TyDefDb, TyLangItem};

use crate::v5_db::{RidxV5ItemDb, CRATE_ROOT_DEF_IDX, IMPL_SYNTH_BIT, PROC_MACRO_SYNTH_BIT};
use crate::v5_ty_lower::{lower_v5_ty, ParamRemap};

impl<'w> RidxV5ItemDb<'w> {
    pub(crate) fn populate_v5_std_macro_gids(&mut self) {
        if let Some(&alloc_r) = self.sysroot_by_name.get("alloc") {
            let vg = self.find_def_in_reader_by_path(alloc_r, "alloc::vec::Vec");
            if vg.is_some() {
                self.ty_tables.vec_gid = Some(self.canonical_def(vg));
            }
            let gg = self.find_def_in_reader_by_path(alloc_r, "alloc::alloc::Global");
            if gg.is_some() {
                self.ty_tables.global_gid = Some(self.canonical_def(gg));
            }
        }
        if let Some(&core_r) = self.sysroot_by_name.get("core") {
            let ag = self.find_def_in_reader_by_path(core_r, "core::fmt::Arguments");
            let ag2 = if ag.is_some() { ag } else { self.find_def_in_reader_by_path(core_r, "core::fmt::rt::Arguments") };
            if ag2.is_some() {
                self.ty_tables.fmt_args_gid = Some(self.canonical_def(ag2));
            }
            let eg = self.find_def_in_reader_by_path(core_r, "core::fmt::Error");
            if eg.is_some() {
                self.ty_tables.fmt_err_gid = Some(self.canonical_def(eg));
            }
        }
    }

    pub(crate) fn v5_generic_param_default_ty(&self, raw_def: GlobalDefId, param_idx: u16, arena: &mut InferTyArena) -> Option<InferTyId> {
        let def = self.canonical_def(raw_def);
        if !def.is_some() || def.index == CRATE_ROOT_DEF_IDX || (def.index & (IMPL_SYNTH_BIT | PROC_MACRO_SYNTH_BIT)) != 0 {
            return None;
        }
        let r_idx = def.krate.0 as usize;
        let reader = self.ws.readers().get(r_idx)?;
        let param = reader.generic_params_for_def(def.index).filter(|p| !matches!(p.kind, GenericParamKind::Lifetime) && !p.is_self()).nth(param_idx as usize)?;
        let raw_ty = param.default_ty()?;
        let remap = ParamRemap::for_def(self, r_idx, def.index, false, arena);
        Some(lower_v5_ty(self, r_idx, reader.ty_arena(), raw_ty, &remap, arena))
    }

    pub(crate) fn v5_std_macro_ty(&self, kind: StdMacroTy, arena: &mut InferTyArena) -> Option<InferTyId> {
        match kind {
            StdMacroTy::Vec(elem) => {
                let vd = self.ty_tables.vec_gid?;
                if let Some(gd) = self.ty_tables.global_gid {
                    let gty = arena.mk_adt(gd, &[]);
                    Some(arena.mk_adt(vd, &[elem, gty]))
                } else {
                    Some(arena.mk_adt(vd, &[elem]))
                }
            }
            StdMacroTy::String => self.lang_item(TyLangItem::String).map(|d| arena.mk_adt(d, &[])),
            StdMacroTy::FmtArguments => self.ty_tables.fmt_args_gid.map(|d| arena.mk_adt(d, &[])),
            StdMacroTy::FmtResult => {
                let res_d = self.lang_item(TyLangItem::Result)?;
                let err_ty = self.ty_tables.fmt_err_gid.map_or(InferTyId::UNKNOWN, |d| arena.mk_adt(d, &[]));
                Some(arena.mk_adt(res_d, &[InferTyId::UNIT, err_ty]))
            }
            StdMacroTy::OptionStr => self.lang_item(TyLangItem::Option).map(|d| arena.mk_adt(d, &[InferTyId::STR_REF])),
        }
    }
}

pub(crate) const fn assoc_lang_to_trait_and_name(item: TyLangItem) -> Option<(TyLangItem, &'static str)> {
    match item {
        TyLangItem::DerefTarget => Some((TyLangItem::Deref, "Target")),
        TyLangItem::TryOutput => Some((TyLangItem::Try, "Output")),
        TyLangItem::FutureOutput => Some((TyLangItem::Future, "Output")),
        TyLangItem::IntoFutureOutput => Some((TyLangItem::IntoFuture, "Output")),
        TyLangItem::IndexOutput => Some((TyLangItem::Index, "Output")),
        TyLangItem::IntoIteratorItem => Some((TyLangItem::IntoIterator, "Item")),
        TyLangItem::IteratorItem => Some((TyLangItem::Iterator, "Item")),
        TyLangItem::FnOnceOutput => Some((TyLangItem::FnOnce, "Output")),
        TyLangItem::AsyncFnOnceOutput => Some((TyLangItem::AsyncFnOnce, "Output")),
        TyLangItem::Add
        | TyLangItem::Sub
        | TyLangItem::Mul
        | TyLangItem::Div
        | TyLangItem::Rem
        | TyLangItem::Neg
        | TyLangItem::Not
        | TyLangItem::BitXor
        | TyLangItem::BitAnd
        | TyLangItem::BitOr
        | TyLangItem::Shl
        | TyLangItem::Shr => Some((item, "Output")),
        _ => None,
    }
}
