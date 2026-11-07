/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Unified `rustc_middle::ty::Ty` -> `ridx_format::TyArena` lowerer shared across
//! workspace crates, immutable dependencies, and sysroot (`core`, `alloc`, `std`).

mod bounds;
pub(crate) mod fields;
mod fn_sig;
mod generics;
mod impls;
pub(crate) mod lang_items;
pub(crate) mod uses;

use std::collections::HashMap;

use ridx_format::{FloatTyWidth, FnAbiKind, IndexWriter, IntTyWidth, RawGenericArg, RawTy, RawTyDef, TyDefId, TyId, TyMutability, UintTyWidth, LIFETIME_ELIDED, LIFETIME_STATIC};
use rustc_middle::ty::{self, FloatTy, IntTy, Mutability, Ty, TyCtxt, TyKind, UintTy};
use rustc_span::def_id::{CrateNum, DefId};

/// Per-crate stateful lowerer that caches `Ty<'tcx> -> TyId` and `DefId -> TyDefId`
/// across all structured tables emitted into an `IndexWriter`.
pub(crate) struct TcxTypeLowerer<'tcx, 'a> {
    pub(super) tcx: TyCtxt<'tcx>,
    local_cnum: CrateNum,
    krate_to_unit: &'a HashMap<CrateNum, String>,
    local_def_indices: &'a HashMap<DefId, u32>,
    ty_cache: HashMap<Ty<'tcx>, TyId>,
    def_cache: HashMap<DefId, TyDefId>,
    pub(super) alias_stack: Vec<DefId>,
    pub(super) scratch_inputs: Vec<TyId>,
}

impl<'tcx, 'a> TcxTypeLowerer<'tcx, 'a> {
    pub(crate) fn new(tcx: TyCtxt<'tcx>, local_cnum: CrateNum, krate_to_unit: &'a HashMap<CrateNum, String>, local_def_indices: &'a HashMap<DefId, u32>) -> Self {
        Self {
            tcx,
            local_cnum,
            krate_to_unit,
            local_def_indices,
            ty_cache: HashMap::with_capacity(512),
            def_cache: HashMap::with_capacity(256),
            alias_stack: Vec::new(),
            scratch_inputs: Vec::with_capacity(8),
        }
    }

    pub(crate) fn lower_def(&mut self, did: DefId, writer: &mut IndexWriter) -> TyDefId {
        if let Some(&cached) = self.def_cache.get(&did) {
            return cached;
        }
        let raw_path = rustc_middle::ty::print::with_no_visible_paths!(rustc_middle::ty::print::with_no_trimmed_paths!(self.tcx.def_path_str(did)));
        let path = if did.is_local() && !raw_path.is_empty() && !raw_path.starts_with('<') {
            format!("{}::{raw_path}", self.tcx.crate_name(did.krate))
        } else if raw_path.is_empty() {
            self.tcx.crate_name(did.krate).to_string()
        } else {
            raw_path
        };
        let id = if did.krate == self.local_cnum {
            let local_def_idx = self.local_def_indices.get(&did).copied().unwrap_or(u32::MAX);
            writer.intern_ty_def(RawTyDef::Local { path: &path, local_def_idx })
        } else {
            let cname_sym = self.tcx.crate_name(did.krate);
            let cname = cname_sym.as_str();
            let unit_key = self.krate_to_unit.get(&did.krate).map(String::as_str).or(match cname {
                "core" | "alloc" | "std" | "proc_macro" => Some(cname),
                _ => None,
            });
            writer.intern_ty_def(RawTyDef::External { path: &path, unit_key })
        };
        self.def_cache.insert(did, id);
        id
    }

    pub(crate) fn lower_ty(&mut self, ty: Ty<'tcx>, writer: &mut IndexWriter) -> TyId {
        if let Some(&cached) = self.ty_cache.get(&ty) {
            return cached;
        }
        let id = match *ty.kind() {
            TyKind::Bool => writer.intern_ty(RawTy::Bool),
            TyKind::Char => writer.intern_ty(RawTy::Char),
            TyKind::Str => writer.intern_ty(RawTy::Str),
            TyKind::Never => writer.intern_ty(RawTy::Never),
            TyKind::Int(i) => writer.intern_ty(RawTy::Int(map_int_ty(i))),
            TyKind::Uint(u) => writer.intern_ty(RawTy::Uint(map_uint_ty(u))),
            TyKind::Float(f) => writer.intern_ty(RawTy::Float(map_float_ty(f))),
            TyKind::Tuple(list) => {
                if list.is_empty() {
                    writer.intern_ty(RawTy::Unit)
                } else {
                    let elems: Vec<TyId> = list.iter().map(|t| self.lower_ty(t, writer)).collect();
                    writer.intern_ty(RawTy::Tuple(&elems))
                }
            }
            TyKind::Slice(elem) => {
                let elem_id = self.lower_ty(elem, writer);
                writer.intern_ty(RawTy::Slice(elem_id))
            }
            TyKind::Array(elem, cst) => {
                let elem_id = self.lower_ty(elem, writer);
                self.lower_array_ty(elem_id, cst, writer)
            }
            TyKind::RawPtr(inner, mutbl) => {
                let inner_id = self.lower_ty(inner, writer);
                writer.intern_ty(RawTy::Ptr { mutability: map_mutability(mutbl), inner: inner_id })
            }
            TyKind::Ref(reg, inner, mutbl) => {
                let inner_id = self.lower_ty(inner, writer);
                writer.intern_ty(RawTy::Ref { mutability: map_mutability(mutbl), inner: inner_id, lifetime: lower_region(reg) })
            }
            TyKind::Pat(base, _) => self.lower_ty(base, writer),
            TyKind::Foreign(did) => {
                let def = self.lower_def(did, writer);
                writer.intern_ty(RawTy::Adt { def, args: &[] })
            }
            TyKind::Adt(adt_def, args) => {
                let def = self.lower_def(adt_def.did(), writer);
                let (owned_args, exprs) = self.lower_generic_args(args, writer);
                let raw_args = build_raw_generic_args(&owned_args, &exprs);
                writer.intern_ty(RawTy::Adt { def, args: &raw_args })
            }
            TyKind::FnDef(did, args) => {
                let def = self.lower_def(did, writer);
                let (owned_args, exprs) = self.lower_generic_args(args, writer);
                let raw_args = build_raw_generic_args(&owned_args, &exprs);
                writer.intern_ty(RawTy::FnDef { def, args: &raw_args })
            }
            TyKind::FnPtr(sig_tys, hdr) => {
                let s = sig_tys.skip_binder();
                let inputs: Vec<TyId> = s.inputs().iter().map(|&t| self.lower_ty(t, writer)).collect();
                let output = self.lower_ty(s.output(), writer);
                writer.intern_ty(RawTy::FnPtr { inputs: &inputs, output, abi: lower_abi(hdr.abi()), is_unsafe: hdr.safety().is_unsafe(), is_variadic: hdr.c_variadic() })
            }
            TyKind::Param(p) => writer.intern_ty(RawTy::Param { index: p.index, name: p.name.as_str() }),
            TyKind::Dynamic(preds, reg) => self.lower_dynamic_ty(preds, reg, writer),
            TyKind::Alias(_is_rigid, alias_ty) => self.lower_alias_ty(alias_ty, writer),
            _ => writer.intern_ty(RawTy::Unknown),
        };
        self.ty_cache.insert(ty, id);
        id
    }

    fn lower_array_ty(&self, elem: TyId, cst: ty::Const<'tcx>, writer: &mut IndexWriter) -> TyId {
        if let ty::ConstKind::Param(cp) = cst.kind() {
            return writer.intern_ty(RawTy::ArrayConstParam { elem, param_idx: cp.index });
        }
        if let Some(v) = cst.try_to_target_usize(self.tcx) {
            if let Ok(v32) = u32::try_from(v) {
                if v32 <= 0x1fff_ffff {
                    return writer.intern_ty(RawTy::ArrayVal { elem, len: v32 });
                }
            }
        }
        let s = cst.to_string();
        writer.intern_ty(RawTy::ArrayConstExpr { elem, expr: &s })
    }

    pub(super) fn lower_generic_args(&mut self, args: &[ty::GenericArg<'tcx>], writer: &mut IndexWriter) -> (Vec<OwnedArg>, Vec<String>) {
        let mut out = Vec::with_capacity(args.len());
        let mut exprs = Vec::new();
        for &ga in args {
            match ga.kind() {
                ty::GenericArgKind::Type(t) => out.push(OwnedArg::Type(self.lower_ty(t, writer))),
                ty::GenericArgKind::Lifetime(r) => out.push(OwnedArg::Lifetime(lower_region(r))),
                ty::GenericArgKind::Const(c) => match c.kind() {
                    ty::ConstKind::Param(cp) => out.push(OwnedArg::ConstParam(cp.index)),
                    _ => {
                        if let Some(v) = c.try_to_target_usize(self.tcx).and_then(|n| u32::try_from(n).ok()).filter(|&n| n <= 0x1fff_ffff) {
                            out.push(OwnedArg::ConstVal(v));
                        } else {
                            let idx = exprs.len();
                            exprs.push(c.to_string());
                            out.push(OwnedArg::ConstExpr(idx));
                        }
                    }
                },
            }
        }
        (out, exprs)
    }
}

#[derive(Clone, Copy)]
pub(super) enum OwnedArg {
    Type(TyId),
    ConstVal(u32),
    ConstParam(u32),
    ConstExpr(usize),
    Lifetime(u32),
}

pub(super) fn build_raw_generic_args<'a>(owned: &[OwnedArg], exprs: &'a [String]) -> Vec<RawGenericArg<'a>> {
    owned
        .iter()
        .map(|&a| match a {
            OwnedArg::Type(t) => RawGenericArg::Type(t),
            OwnedArg::ConstVal(v) => RawGenericArg::ConstValue(v),
            OwnedArg::ConstParam(p) => RawGenericArg::ConstParam(p),
            OwnedArg::ConstExpr(i) => RawGenericArg::ConstExpr(exprs[i].as_str()),
            OwnedArg::Lifetime(lt) => RawGenericArg::Lifetime(lt),
        })
        .collect()
}

#[inline]
pub(super) fn lower_region(reg: ty::Region<'_>) -> u32 {
    match reg.kind() {
        ty::RegionKind::ReStatic => LIFETIME_STATIC,
        ty::RegionKind::ReEarlyParam(ebr) => (ebr.index + 1).min(LIFETIME_STATIC - 1),
        _ => LIFETIME_ELIDED,
    }
}

#[inline]
fn map_mutability(m: Mutability) -> TyMutability {
    match m {
        Mutability::Not => TyMutability::Shared,
        Mutability::Mut => TyMutability::Mut,
    }
}

#[inline]
fn map_int_ty(i: IntTy) -> IntTyWidth {
    match i {
        IntTy::Isize => IntTyWidth::Isize,
        IntTy::I8 => IntTyWidth::I8,
        IntTy::I16 => IntTyWidth::I16,
        IntTy::I32 => IntTyWidth::I32,
        IntTy::I64 => IntTyWidth::I64,
        IntTy::I128 => IntTyWidth::I128,
    }
}

#[inline]
fn map_uint_ty(u: UintTy) -> UintTyWidth {
    match u {
        UintTy::Usize => UintTyWidth::Usize,
        UintTy::U8 => UintTyWidth::U8,
        UintTy::U16 => UintTyWidth::U16,
        UintTy::U32 => UintTyWidth::U32,
        UintTy::U64 => UintTyWidth::U64,
        UintTy::U128 => UintTyWidth::U128,
    }
}

#[inline]
fn map_float_ty(f: FloatTy) -> FloatTyWidth {
    match f {
        FloatTy::F16 => FloatTyWidth::F16,
        FloatTy::F32 => FloatTyWidth::F32,
        FloatTy::F64 => FloatTyWidth::F64,
        FloatTy::F128 => FloatTyWidth::F128,
    }
}

#[inline]
pub(super) fn lower_abi(abi: rustc_abi::ExternAbi) -> FnAbiKind {
    match abi {
        rustc_abi::ExternAbi::Rust => FnAbiKind::Rust,
        rustc_abi::ExternAbi::C { .. } => FnAbiKind::C,
        rustc_abi::ExternAbi::System { .. } => FnAbiKind::System,
        rustc_abi::ExternAbi::RustCall => FnAbiKind::RustCall,
        _ => FnAbiKind::Other,
    }
}
