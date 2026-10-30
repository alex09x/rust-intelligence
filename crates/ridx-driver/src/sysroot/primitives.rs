/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use std::collections::HashSet;
use std::path::Path;

use ridx_format::{IndexWriter, RawDefinition};
use rustc_hir::def::DefKind;
use rustc_middle::ty::fast_reject::SimplifiedType;
use rustc_middle::ty::{FloatTy, IntTy, Mutability, TyCtxt, UintTy};
use rustc_span::def_id::DefId;

use super::util::{format_assoc_fn_sig, is_publicly_visible, is_stable_api, remap_sysroot_path};
use crate::map_def_kind;
use crate::sanitize::sanitize_type_name;

/// Extracts inherent methods for primitive types (e.g. `impl str`, `impl [T]`)
/// with domain owner strings matching `resolve_owner_from_ty` (`"str"`, `"slice"`, etc.).
pub fn extract_primitive_inherent_impls<'tcx>(
    tcx: TyCtxt<'tcx>,
    cnum: rustc_span::def_id::CrateNum,
    writer: &mut IndexWriter,
    visited: &mut HashSet<DefId>,
    total_definitions: &mut usize,
    sysroot_src: Option<&Path>,
) {
    let primitives = [
        (SimplifiedType::Str, "str"),
        (SimplifiedType::Slice, "slice"),
        (SimplifiedType::Bool, "bool"),
        (SimplifiedType::Char, "char"),
        (SimplifiedType::Uint(UintTy::U8), "u8"),
        (SimplifiedType::Uint(UintTy::U16), "u16"),
        (SimplifiedType::Uint(UintTy::U32), "u32"),
        (SimplifiedType::Uint(UintTy::U64), "u64"),
        (SimplifiedType::Uint(UintTy::U128), "u128"),
        (SimplifiedType::Uint(UintTy::Usize), "usize"),
        (SimplifiedType::Int(IntTy::I8), "i8"),
        (SimplifiedType::Int(IntTy::I16), "i16"),
        (SimplifiedType::Int(IntTy::I32), "i32"),
        (SimplifiedType::Int(IntTy::I64), "i64"),
        (SimplifiedType::Int(IntTy::I128), "i128"),
        (SimplifiedType::Int(IntTy::Isize), "isize"),
        (SimplifiedType::Float(FloatTy::F32), "f32"),
        (SimplifiedType::Float(FloatTy::F64), "f64"),
        (SimplifiedType::Array, "array"),
        (SimplifiedType::Ptr(Mutability::Not), "pointer"),
        (SimplifiedType::Ptr(Mutability::Mut), "pointer"),
    ];

    for (simp_ty, owner_str) in primitives {
        for &impl_did in tcx.incoherent_impls(simp_ty) {
            if impl_did.krate != cnum {
                continue;
            }
            for assoc_did in tcx.associated_item_def_ids(impl_did) {
                let assoc_did = *assoc_did;
                if !visited.insert(assoc_did) {
                    continue;
                }
                if !is_publicly_visible(tcx, assoc_did) || !is_stable_api(tcx, assoc_did) {
                    continue;
                }
                let Some(name_sym) = tcx.opt_item_name(assoc_did) else {
                    continue;
                };
                let a_name = name_sym.to_string();
                if a_name.starts_with("__rust_") || a_name.starts_with("__") {
                    continue;
                }
                let a_kind = tcx.def_kind(assoc_did);
                let a_path = rustc_middle::ty::print::with_no_visible_paths!(rustc_middle::ty::print::with_no_trimmed_paths!(tcx.def_path_str(assoc_did)));
                let a_sig = match a_kind {
                    DefKind::Fn => Some(sanitize_type_name(&tcx.fn_sig(assoc_did).skip_binder().to_string())),
                    DefKind::AssocFn => Some(format_assoc_fn_sig(tcx, assoc_did)),
                    _ => None,
                };
                let a_span = tcx.def_ident_span(assoc_did).unwrap_or_else(|| tcx.def_span(assoc_did));
                let (a_file, a_sl, a_sc, a_el, a_ec) = if !a_span.is_dummy() {
                    let sm = tcx.sess.source_map();
                    let lo = sm.lookup_char_pos(a_span.lo());
                    let hi = sm.lookup_char_pos(a_span.hi());
                    let raw_file = sm.filename_for_diagnostics(&lo.file.name).to_string();
                    let remapped = remap_sysroot_path(&raw_file, sysroot_src);
                    (remapped, lo.line as u32, (lo.col.0 + 1) as u32, hi.line as u32, (hi.col.0 + 1) as u32)
                } else {
                    (String::new(), 0, 0, 0, 0)
                };

                writer.add_definition_with_owners(
                    RawDefinition {
                        def_id: *total_definitions as u32,
                        kind: map_def_kind(a_kind),
                        visibility: 0,
                        is_macro_expansion: false,
                        name: a_name,
                        path: a_path,
                        signature: a_sig,
                        file: a_file,
                        start_line: a_sl,
                        start_col: a_sc,
                        end_line: a_el,
                        end_col: a_ec,
                    },
                    vec![owner_str.to_string()],
                );
                *total_definitions += 1;
            }
        }
    }
}
