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

impl<'tcx> HirVisitor<'tcx> {
    fn current_param_env(&self) -> Option<rustc_middle::ty::ParamEnv<'tcx>> {
        self.current_body_owner.map(|owner| self.tcx.param_env(owner.to_def_id()))
    }

    fn add_ref_from_res(&mut self, span: Span, res: Res) {
        let Some((file, s_line, s_col, e_line, e_col, is_macro)) = resolve_span_coords(self.tcx, span, self.package_root.as_deref()) else {
            return;
        };

        match res {
            Res::Def(kind, def_id) => {
                let target_path = rustc_middle::ty::print::with_no_visible_paths!(rustc_middle::ty::print::with_no_trimmed_paths!(self.tcx.def_path_str(def_id)));
                if !self.seen_refs.insert((file.clone(), s_line, s_col, target_path.clone())) {
                    return;
                }
                let target_span = self.tcx.def_ident_span(def_id).unwrap_or_else(|| self.tcx.def_span(def_id));
                let target_coords = resolve_span_coords(self.tcx, target_span, self.package_root.as_deref());
                let (target_file, target_line, target_col) = match target_coords {
                    Some((f, l, c, _, _, _)) => (Some(f), l, c),
                    None => (None, 0, 0),
                };

                let target_def_id = if def_id.krate == rustc_span::def_id::LOCAL_CRATE { Some(def_id.index.as_u32()) } else { None };

                let target_unit = self.krate_to_unit.get(&def_id.krate).cloned();

                self.writer.add_reference(RawReference {
                    file,
                    start_line: s_line,
                    start_col: s_col,
                    end_line: e_line,
                    end_col: e_col,
                    target_def_id,
                    target_path,
                    target_unit,
                    target_file,
                    target_line,
                    target_col,
                    kind: map_def_kind(kind),
                    is_write: false,
                    is_macro_expansion: is_macro,
                });
            }
            Res::Local(hir_id) => {
                let name = self.tcx.hir_name(hir_id).to_string();
                if !self.seen_refs.insert((file.clone(), s_line, s_col, name.clone())) {
                    return;
                }
                let target_coords = resolve_span_coords(self.tcx, self.tcx.hir_span(hir_id), self.package_root.as_deref());
                let (target_file, target_line, target_col) = match target_coords {
                    Some((f, l, c, _, _, _)) => (Some(f), l, c),
                    None => (None, 0, 0),
                };

                let target_unit = self.krate_to_unit.get(&rustc_span::def_id::LOCAL_CRATE).cloned();

                self.writer.add_reference(RawReference {
                    file,
                    start_line: s_line,
                    start_col: s_col,
                    end_line: e_line,
                    end_col: e_col,
                    target_def_id: None,
                    target_path: name,
                    target_unit,
                    target_file,
                    target_line,
                    target_col,
                    kind: RDefKind::Local,
                    is_write: false,
                    is_macro_expansion: is_macro,
                });
            }
            _ => {}
        }
    }

    fn resolve_trait_instance(&self, expr_hir_id: hir::HirId, def_id: DefId) -> Option<DefId> {
        let is_trait_item = self.tcx.opt_associated_item(def_id).map(|a| matches!(a.container, rustc_middle::ty::AssocContainer::Trait)).unwrap_or(false);
        if !is_trait_item {
            return None;
        }
        let typeck = self.typeck?;
        let args = typeck.node_args(expr_hir_id);
        // Desugared paths (`for`, `?`) record no arguments; resolving with too few panics inside a query.
        if args.len() != self.tcx.generics_of(def_id).count() || args.has_infer() || args.has_escaping_bound_vars() || args.references_error() {
            return None;
        }
        let body_owner = self.tcx.hir_enclosing_body_owner(expr_hir_id).to_def_id();
        let env = rustc_middle::ty::TypingEnv::post_analysis(self.tcx, body_owner);
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let norm_args = self.tcx.try_normalize_erasing_regions(env, rustc_middle::ty::Unnormalized::new(args)).unwrap_or(args);
            rustc_middle::ty::Instance::try_resolve(self.tcx, env, def_id, norm_args)
        }));
        match res {
            Ok(Ok(Some(inst))) => Some(inst.def_id()),
            Ok(_) => None,
            Err(payload) => {
                let cnt = CAUGHT_PANICS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                let panic_msg = if let Some(s) = payload.downcast_ref::<&str>() {
                    s.to_string()
                } else if let Some(s) = payload.downcast_ref::<String>() {
                    s.clone()
                } else {
                    "unknown panic payload".to_string()
                };
                let span = self.tcx.hir_span(expr_hir_id);
                eprintln!(
                    "[ridx-driver] WARNING: caught panic #{cnt} in resolve_trait_instance: hir_id={:?}, def_id={:?}, span={:?}, msg={}",
                    expr_hir_id, def_id, span, panic_msg
                );
                None
            }
        }
    }
}

impl<'tcx> Visitor<'tcx> for HirVisitor<'tcx> {
    type NestedFilter = rustc_middle::hir::nested_filter::OnlyBodies;

    fn maybe_tcx(&mut self) -> Self::MaybeTyCtxt {
        self.tcx
    }

    fn visit_nested_body(&mut self, id: hir::BodyId) {
        let prev = self.typeck.replace(self.tcx.typeck_body(id));
        let prev_owner = self.current_body_owner.replace(self.tcx.hir_body_owner_def_id(id));
        self.visit_body(self.tcx.hir_body(id));
        self.current_body_owner = prev_owner;
        self.typeck = prev;
    }

    fn visit_path(&mut self, path: &hir::Path<'tcx>, hir_id: HirId) {
        let span = if let Some(last_seg) = path.segments.last() { last_seg.ident.span } else { path.span };
        // A trait function named by a path in a body (`From::from`, `Default::default()`) is the implementation the
        // arguments select, as for a method call. Paths a macro or a desugaring wrote (a derive's
        // `Default::default()`) keep naming the trait's function: no implementation's name is written there.
        let res = match path.res {
            Res::Def(hir::def::DefKind::AssocFn, did) if !path.span.from_expansion() && self.typeck.is_some_and(|t| t.hir_owner == hir_id.owner) => {
                self.resolve_trait_instance(hir_id, did).map_or(path.res, |i| Res::Def(self.tcx.def_kind(i), i))
            }
            _ => path.res,
        };
        self.add_ref_from_res(span, res);
        // Module segments of a path (`std::ops` in `std::ops::Bound`) are references to the modules.
        for seg in path.segments.iter().take(path.segments.len().saturating_sub(1)) {
            if let Res::Def(hir::def::DefKind::Mod, _) = seg.res {
                self.add_ref_from_res(seg.ident.span, seg.res);
            }
        }
        // `Trait<Assoc = T>`: the constraint's name refers to the trait's associated type.
        if let Res::Def(hir::def::DefKind::Trait, trait_did) = path.res {
            if let Some(args) = path.segments.last().and_then(|seg| seg.args) {
                for c in args.constraints {
                    if let Some(d) = crate::assoc_res::trait_assoc_ty(self.tcx, trait_did, c.ident.name) {
                        self.add_ref_from_res(c.ident.span, Res::Def(hir::def::DefKind::AssocTy, d));
                    }
                }
            }
        }
        // `Self` in an impl names the implementing type; rust-analyzer lists it among the type's references.
        if let Res::SelfTyAlias { alias_to, .. } | Res::SelfCtor(alias_to) = path.res {
            if let Some(adt) = self.tcx.type_of(alias_to).skip_binder().ty_adt_def() {
                if let Some(first) = path.segments.first().filter(|s| s.ident.name == rustc_span::symbol::kw::SelfUpper) {
                    self.add_ref_from_res(first.ident.span, Res::Def(self.tcx.def_kind(adt.did()), adt.did()));
                }
            }
        }

        if path.segments.len() >= 2 {
            let pen_seg = &path.segments[path.segments.len() - 2];
            match path.res {
                Res::Def(hir::def::DefKind::Ctor(hir::def::CtorOf::Variant, _), ctor_did) => {
                    let var_did = self.tcx.parent(ctor_did);
                    let enum_did = self.tcx.parent(var_did);
                    self.add_ref_from_res(pen_seg.ident.span, Res::Def(hir::def::DefKind::Enum, enum_did));
                }
                Res::Def(hir::def::DefKind::Variant, var_did) => {
                    let enum_did = self.tcx.parent(var_did);
                    self.add_ref_from_res(pen_seg.ident.span, Res::Def(hir::def::DefKind::Enum, enum_did));
                }
                Res::Def(hir::def::DefKind::AssocFn, fn_did) | Res::Def(hir::def::DefKind::AssocConst { .. }, fn_did) => {
                    let parent_did = self.tcx.parent(fn_did);
                    let parent_kind = self.tcx.def_kind(parent_did);
                    let target_res = if matches!(parent_kind, hir::def::DefKind::Impl { .. }) {
                        let self_ty = self.tcx.type_of(parent_did).skip_binder();
                        if let Some(adt) = self_ty.ty_adt_def() {
                            let adt_did = adt.did();
                            Res::Def(self.tcx.def_kind(adt_did), adt_did)
                        } else {
                            Res::Def(parent_kind, parent_did)
                        }
                    } else {
                        Res::Def(parent_kind, parent_did)
                    };
                    self.add_ref_from_res(pen_seg.ident.span, target_res);
                }
                _ => {}
            }
        }

        intravisit::walk_path(self, path);
    }

    fn visit_ty(&mut self, ty: &'tcx hir::Ty<'tcx, hir::AmbigArg>) {
        if let hir::TyKind::Path(hir::QPath::TypeRelative(qself, segment)) = ty.kind {
            // Inside a body typeck resolves the path; in a signature the trait is found from `Self` or the parameter.
            let res = match self.typeck {
                Some(typeck) => typeck.qpath_res(&hir::QPath::TypeRelative(qself, segment), ty.hir_id),
                None => crate::assoc_res::type_relative_assoc_res(self.tcx, qself, segment.ident.name),
            };
            self.add_ref_from_res(segment.ident.span, res);
        }
        intravisit::walk_ty(self, ty);
    }

    fn visit_expr(&mut self, expr: &'tcx hir::Expr<'tcx>) {
        if let Some(typeck) = self.typeck {
            match expr.kind {
                hir::ExprKind::MethodCall(segment, ..) => {
                    if let Some(def_id) = typeck.type_dependent_def_id(expr.hir_id) {
                        let final_def_id = self.resolve_trait_instance(expr.hir_id, def_id).unwrap_or(def_id);
                        self.add_ref_from_res(segment.ident.span, Res::Def(self.tcx.def_kind(final_def_id), final_def_id));
                        self.note_call_args(segment.ident.span, def_id, expr.hir_id);
                    }
                }
                hir::ExprKind::Path(hir::QPath::Resolved(_, path)) => {
                    if let (Res::Def(hir::def::DefKind::Fn | hir::def::DefKind::AssocFn, def_id), Some(last)) = (path.res, path.segments.last()) {
                        self.note_call_args(last.ident.span, def_id, expr.hir_id);
                    }
                }
                hir::ExprKind::Path(hir::QPath::TypeRelative(_, segment)) => {
                    if let hir::ExprKind::Path(ref qpath) = expr.kind {
                        let mut res = typeck.qpath_res(qpath, expr.hir_id);
                        if let Res::Def(hir::def::DefKind::AssocFn, def_id) = res {
                            self.note_call_args(segment.ident.span, def_id, expr.hir_id);
                            if let Some(resolved_did) = self.resolve_trait_instance(expr.hir_id, def_id) {
                                res = Res::Def(self.tcx.def_kind(resolved_did), resolved_did);
                            }
                        }
                        self.add_ref_from_res(segment.ident.span, res);
                    }
                }
                hir::ExprKind::Field(base, field) => {
                    if let Some(adt) = typeck.expr_ty_adjusted(base).ty_adt_def() {
                        if !adt.is_enum() {
                            if let Some(def) = adt.non_enum_variant().fields.iter().find(|it| it.name == field.name) {
                                self.add_ref_from_res(field.span, Res::Def(self.tcx.def_kind(def.did), def.did));
                            }
                        }
                    }
                }
                hir::ExprKind::Struct(qpath, fields, _) => {
                    if let hir::QPath::TypeRelative(_, segment) = qpath {
                        self.add_ref_from_res(segment.ident.span, typeck.qpath_res(qpath, expr.hir_id));
                    }
                    if let Some(variant) = typeck.expr_ty_opt(expr).and_then(|t| t.ty_adt_def()).and_then(|adt| literal_variant(adt, typeck.qpath_res(qpath, expr.hir_id))) {
                        for f in fields {
                            if !f.is_shorthand {
                                if let Some(def) = variant.fields.iter().find(|it| it.name == f.ident.name) {
                                    self.add_ref_from_res(f.ident.span, Res::Def(self.tcx.def_kind(def.did), def.did));
                                }
                            }
                        }
                    }
                }
                _ => {}
            }

            // Inferred expression type
            if !expr.span.from_expansion() {
                if let Some((file, s_line, s_col, e_line, e_col, _)) = resolve_span_coords(self.tcx, expr.span, self.package_root.as_deref()) {
                    if let Some(ty) = typeck.expr_ty_opt(expr) {
                        let owner_path = resolve_owner_from_ty(self.tcx, ty, self.current_param_env());
                        self.writer.add_type_with_owner(
                            RawType { file, start_line: s_line, start_col: s_col, end_line: e_line, end_col: e_col, type_name: sanitize_type_name(&ty.to_string()) },
                            owner_path,
                        );
                    }
                }
            }
        }
        intravisit::walk_expr(self, expr);
    }

    fn visit_pat(&mut self, pat: &'tcx hir::Pat<'tcx>) {
        if !pat.span.from_expansion() {
            if let Some(typeck) = self.typeck {
                match pat.kind {
                    hir::PatKind::Binding(..) => {
                        if let Some((file, s_line, s_col, e_line, e_col, _)) = resolve_span_coords(self.tcx, pat.span, self.package_root.as_deref()) {
                            let ty = typeck.pat_ty(pat);
                            let owner_path = resolve_owner_from_ty(self.tcx, ty, self.current_param_env());
                            self.writer.add_type_with_owner(
                                RawType { file, start_line: s_line, start_col: s_col, end_line: e_line, end_col: e_col, type_name: sanitize_type_name(&ty.to_string()) },
                                owner_path,
                            );
                            self.note_binding_layout(pat, ty);
                        }
                    }
                    // `Self::SetTrue =>`, `Self::Command(_) =>`: a type-relative path resolved by typeck (a
                    // resolved path is a ref through `visit_path`).
                    hir::PatKind::Expr(hir::PatExpr { kind: hir::PatExprKind::Path(qpath @ hir::QPath::TypeRelative(_, segment)), hir_id, .. }) => {
                        self.add_ref_from_res(segment.ident.span, typeck.qpath_res(qpath, *hir_id));
                    }
                    hir::PatKind::TupleStruct(ref qpath @ hir::QPath::TypeRelative(_, segment), ..) => {
                        self.add_ref_from_res(segment.ident.span, typeck.qpath_res(qpath, pat.hir_id));
                    }
                    hir::PatKind::Struct(ref qpath, fields, _) => {
                        if let hir::QPath::TypeRelative(_, segment) = qpath {
                            self.add_ref_from_res(segment.ident.span, typeck.qpath_res(qpath, pat.hir_id));
                        }
                        if let Some(variant) = typeck.pat_ty(pat).ty_adt_def().and_then(|adt| literal_variant(adt, typeck.qpath_res(qpath, pat.hir_id))) {
                            for f in fields {
                                if !f.is_shorthand {
                                    if let Some(def) = variant.fields.iter().find(|it| it.name == f.ident.name) {
                                        self.add_ref_from_res(f.ident.span, Res::Def(self.tcx.def_kind(def.did), def.did));
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        intravisit::walk_pat(self, pat);
    }
}

/// The variant a struct literal or struct pattern names: the struct's only variant, or the enum variant its path
/// resolves to (`Kind::Cornered { corners }`, `Self::Cornered { .. }`). `None` when the path names no variant.
fn literal_variant<'tcx>(adt: rustc_middle::ty::AdtDef<'tcx>, res: Res) -> Option<&'tcx rustc_middle::ty::VariantDef> {
    if !adt.is_enum() {
        return Some(adt.non_enum_variant());
    }
    match res {
        Res::Def(hir::def::DefKind::Variant | hir::def::DefKind::Ctor(hir::def::CtorOf::Variant, _), _) => Some(adt.variant_of_res(res)),
        _ => None,
    }
}
