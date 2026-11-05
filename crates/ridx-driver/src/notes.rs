/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Notes hover shows as the compiler knows them, written in rust-analyzer's wording: the generic arguments of a
//! call (`` `T` = `String`, `A` = `Global` ``) and the memory layout of types, fields, variants and bindings
//! (`size = 24 (0x18), align = 0x8, needs Drop`).

use super::*;
use ridx_format::{RawNote, NOTE_CALL_ARGS, NOTE_LAYOUT};
use rustc_middle::ty::{GenericArgsRef, Ty, TypingEnv};

/// `ty` printed with its paths cut to their last segment, as rust-analyzer prints types; closures as `{closure}`.
pub(crate) fn short_ty<'tcx>(ty: Ty<'tcx>) -> String {
    if ty.is_closure() {
        return "{closure}".to_string();
    }
    short_path_text(&ty.to_string())
}

/// `text` with every `a::b::` path prefix removed: `std::vec::Vec<std::string::String>` → `Vec<String>`.
pub(crate) fn short_path_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut word_start = 0;
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == ':' && chars.get(i + 1) == Some(&':') && out.len() > word_start && chars.get(i + 2).is_some_and(|n| n.is_alphabetic() || *n == '_') {
            out.truncate(word_start);
            i += 2;
            continue;
        }
        out.push(c);
        if !(c.is_alphanumeric() || c == '_') {
            word_start = out.len();
        }
        i += 1;
    }
    out
}

/// `` `T` = `String`, `A` = `Global` `` for a call of `def_id` with `args`; `None` when it has no type or const
/// parameters (lifetimes and `Self` are not listed) or an argument is not known.
pub(crate) fn call_args_text<'tcx>(tcx: TyCtxt<'tcx>, def_id: DefId, args: GenericArgsRef<'tcx>) -> Option<String> {
    if args.is_empty() || args.has_infer() || args.references_error() {
        return None;
    }
    let generics = tcx.generics_of(def_id);
    if generics.count() != args.len() {
        return None;
    }
    let mut parts = Vec::new();
    for (i, arg) in args.iter().enumerate() {
        let param = generics.param_at(i, tcx);
        if param.name == rustc_span::symbol::kw::SelfUpper {
            continue;
        }
        let shown = if let Some(ty) = arg.as_type() {
            short_ty(ty)
        } else if let Some(ct) = arg.as_const() {
            ct.to_string()
        } else {
            continue;
        };
        parts.push(format!("`{}` = `{shown}`", param.name));
    }
    (!parts.is_empty()).then(|| parts.join(", "))
}

/// rust-analyzer's number formatting in layouts: decimal, with the hex form added from 10 on.
fn size_text(n: u64) -> String {
    if n >= 10 {
        format!("{n} ({n:#x})")
    } else {
        n.to_string()
    }
}

/// `size = 8, align = 0x8[, offset = 0x0], no Drop` for `ty`; `None` when its layout is not computable here.
pub(crate) fn layout_text<'tcx>(tcx: TyCtxt<'tcx>, env: TypingEnv<'tcx>, ty: Ty<'tcx>, offset: Option<u64>) -> Option<String> {
    if ty.has_param() || ty.has_infer() || ty.references_error() || ty.has_escaping_bound_vars() {
        return None;
    }
    let computed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let layout = tcx.layout_of(env.as_query_input(ty)).ok()?;
        let drops = ty.needs_drop(tcx, env);
        Some((layout.size.bytes(), layout.align.abi.bytes(), drops))
    }))
    .ok()
    .flatten()?;
    let (size, align, drops) = computed;
    let offset = offset.map(|o| format!(", offset = {o:#x}")).unwrap_or_default();
    Some(format!("size = {}, align = {align:#x}{offset}, {}", size_text(size), if drops { "needs Drop" } else { "no Drop" }))
}

impl<'tcx> HirVisitor<'tcx> {
    /// Records a note of `kind` at `span` unless the span comes from a macro expansion.
    pub(crate) fn add_note(&mut self, span: Span, kind: u32, text: String) {
        let Some((file, start_line, start_col, end_line, end_col, is_macro)) = resolve_span_coords(self.tcx, span, self.package_root.as_deref()) else {
            return;
        };
        if is_macro {
            return;
        }
        self.writer.add_note(RawNote { file, start_line, start_col, end_line, end_col, kind, text });
    }

    /// The generic arguments of the call whose callee name is at `name_span`.
    pub(crate) fn note_call_args(&mut self, name_span: Span, def_id: DefId, call_id: HirId) {
        let Some(typeck) = self.typeck else { return };
        if let Some(text) = call_args_text(self.tcx, def_id, typeck.node_args(call_id)) {
            self.add_note(name_span, NOTE_CALL_ARGS, text);
        }
    }

    /// The layout of a `let` or pattern binding's type, at the binding.
    pub(crate) fn note_binding_layout(&mut self, pat: &hir::Pat<'tcx>, ty: Ty<'tcx>) {
        let Some(owner) = self.current_body_owner else { return };
        let env = TypingEnv::post_analysis(self.tcx, owner.to_def_id());
        if let Some(text) = layout_text(self.tcx, env, ty, None) {
            self.add_note(pat.span, NOTE_LAYOUT, text);
        }
    }
}

/// Layouts of the crate's non-generic structs, enums and unions, their fields (with offsets) and enum variants,
/// each at its name.
pub(crate) fn record_adt_layouts(tcx: TyCtxt<'_>, visitor: &mut HirVisitor<'_>) {
    let env = TypingEnv::fully_monomorphized();
    for local in tcx.hir_crate_items(()).definitions() {
        let def_id = local.to_def_id();
        if !matches!(tcx.def_kind(def_id), hir::def::DefKind::Struct | hir::def::DefKind::Enum | hir::def::DefKind::Union) {
            continue;
        }
        if tcx.generics_of(def_id).count() != 0 {
            continue;
        }
        let ty = tcx.type_of(def_id).skip_binder();
        let Some(text) = layout_text(tcx, env, ty, None) else { continue };
        if let Some(span) = tcx.def_ident_span(def_id) {
            visitor.add_note(span, NOTE_LAYOUT, text.clone());
        }
        let adt = tcx.adt_def(def_id);
        if adt.is_enum() {
            for variant in adt.variants() {
                if let Some(span) = tcx.def_ident_span(variant.def_id) {
                    visitor.add_note(span, NOTE_LAYOUT, text.clone());
                }
            }
            continue;
        }
        let Ok(layout) = tcx.layout_of(env.as_query_input(ty)) else { continue };
        for (i, field) in adt.non_enum_variant().fields.iter().enumerate() {
            let field_ty = tcx.type_of(field.did).skip_binder();
            let offset = if adt.is_union() { 0 } else { layout.fields.offset(i).bytes() };
            if let (Some(text), Some(span)) = (layout_text(tcx, env, field_ty, Some(offset)), tcx.def_ident_span(field.did)) {
                visitor.add_note(span, NOTE_LAYOUT, text);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{short_path_text, size_text};

    #[test]
    fn test_short_paths_and_sizes_read_like_rust_analyzer() {
        assert_eq!(short_path_text("std::vec::Vec<std::string::String>"), "Vec<String>");
        assert_eq!(short_path_text("&'a alloc::string::String"), "&'a String");
        assert_eq!(size_text(8), "8");
        assert_eq!(size_text(24), "24 (0x18)");
    }
}
