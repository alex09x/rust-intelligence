/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use super::*;

pub(crate) fn map_def_kind(kind: hir::def::DefKind) -> RDefKind {
    match kind {
        hir::def::DefKind::Mod => RDefKind::Mod,
        hir::def::DefKind::Struct => RDefKind::Struct,
        hir::def::DefKind::Union => RDefKind::Union,
        hir::def::DefKind::Enum => RDefKind::Enum,
        hir::def::DefKind::Variant => RDefKind::Variant,
        hir::def::DefKind::Trait => RDefKind::Trait,
        hir::def::DefKind::TyAlias => RDefKind::TyAlias,
        hir::def::DefKind::ForeignTy => RDefKind::ForeignTy,
        hir::def::DefKind::TraitAlias => RDefKind::TraitAlias,
        hir::def::DefKind::AssocTy => RDefKind::AssocTy,
        hir::def::DefKind::TyParam => RDefKind::TyParam,
        hir::def::DefKind::Fn => RDefKind::Fn,
        hir::def::DefKind::Const { .. } => RDefKind::Const,
        hir::def::DefKind::ConstParam => RDefKind::ConstParam,
        hir::def::DefKind::Static { .. } => RDefKind::Static,
        hir::def::DefKind::Ctor(hir::def::CtorOf::Struct, _) => RDefKind::StructCtor,
        hir::def::DefKind::Ctor(hir::def::CtorOf::Variant, _) => RDefKind::VariantCtor,
        hir::def::DefKind::AssocFn => RDefKind::AssocFn,
        hir::def::DefKind::AssocConst { .. } => RDefKind::AssocConst,
        hir::def::DefKind::Macro(_) => RDefKind::Macro,
        hir::def::DefKind::ExternCrate => RDefKind::ExternCrate,
        hir::def::DefKind::Use => RDefKind::Use,
        hir::def::DefKind::ForeignMod => RDefKind::ForeignMod,
        hir::def::DefKind::AnonConst => RDefKind::AnonConst,
        hir::def::DefKind::OpaqueTy => RDefKind::OpaqueTy,
        hir::def::DefKind::Field => RDefKind::Field,
        hir::def::DefKind::LifetimeParam => RDefKind::LifetimeParam,
        hir::def::DefKind::GlobalAsm => RDefKind::GlobalAsm,
        hir::def::DefKind::Impl { .. } => RDefKind::Impl,
        hir::def::DefKind::Closure => RDefKind::Closure,
        _ => RDefKind::Other,
    }
}

pub(crate) fn resolve_span_coords(
    tcx: TyCtxt<'_>,
    span: Span,
    package_root: Option<&Path>,
) -> Option<(String, u32, u32, u32, u32, bool)> {
    if span.is_dummy() {
        return None;
    }
    let is_macro = span.from_expansion();
    // Remap macro expansions to their outer call-site in source code
    let actual_span = if is_macro {
        span.source_callsite()
    } else {
        span
    };
    if actual_span.is_dummy() {
        return None;
    }
    let source_map = tcx.sess.source_map();
    let (lo, hi) = (
        source_map.lookup_char_pos(actual_span.lo()),
        source_map.lookup_char_pos(actual_span.hi()),
    );
    let full_file = source_map
        .filename_for_diagnostics(&lo.file.name)
        .to_string();
    let file = if let Some(root) = package_root {
        let p = Path::new(&full_file);
        if let Ok(rel) = p.strip_prefix(root) {
            rel.to_string_lossy().into_owned()
        } else {
            full_file
        }
    } else {
        full_file
    };
    Some((
        file,
        lo.line as u32,
        (lo.col.0 + 1) as u32,
        hi.line as u32,
        (hi.col.0 + 1) as u32,
        is_macro,
    ))
}
