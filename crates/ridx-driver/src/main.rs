/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Compiler driver wrapper that intercepts compilation during `cargo check` and writes
//! a zero-copy `.ridx` binary index file for the compiled crate.
#![feature(rustc_private)]

extern crate rustc_driver;
extern crate rustc_hir;
extern crate rustc_interface;
extern crate rustc_middle;
extern crate rustc_session;
extern crate rustc_span;

use std::fs::{self, File};
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::time::Instant;

use ridx_format::{DefKind as RDefKind, IndexWriter, RawDefinition, RawReference, RawType};
use rustc_driver::{Callbacks, Compilation};
use rustc_hir::{
    self as hir,
    def::Res,
    intravisit::{self, Visitor},
    HirId,
};
use rustc_middle::ty::{TyCtxt, TypeVisitableExt, TypeckResults};
use rustc_span::Span;

struct IndexCallbacks {
    out_dir: PathBuf,
    disambiguator: Option<String>,
    compiler_args: Vec<String>,
}

impl Callbacks for IndexCallbacks {
    fn after_analysis<'tcx>(
        &mut self,
        _compiler: &rustc_interface::interface::Compiler,
        tcx: TyCtxt<'tcx>,
    ) -> Compilation {
        build_crate_index(
            tcx,
            &self.out_dir,
            self.disambiguator.as_deref(),
            &self.compiler_args,
        );
        Compilation::Continue
    }
}

fn map_def_kind(kind: hir::def::DefKind) -> RDefKind {
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

fn resolve_span_coords(
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

struct HirVisitor<'tcx> {
    tcx: TyCtxt<'tcx>,
    typeck: Option<&'tcx TypeckResults<'tcx>>,
    package_root: Option<PathBuf>,
    writer: IndexWriter,
    krate_to_unit: std::collections::HashMap<rustc_span::def_id::CrateNum, String>,
    seen_refs: std::collections::HashSet<(String, u32, u32, String)>,
}

impl<'tcx> HirVisitor<'tcx> {
    fn add_ref_from_res(&mut self, span: Span, res: Res) {
        let Some((file, s_line, s_col, e_line, e_col, is_macro)) =
            resolve_span_coords(self.tcx, span, self.package_root.as_deref())
        else {
            return;
        };

        match res {
            Res::Def(kind, def_id) => {
                let target_path = self.tcx.def_path_str(def_id);
                if !self
                    .seen_refs
                    .insert((file.clone(), s_line, s_col, target_path.clone()))
                {
                    return;
                }
                let target_span = self
                    .tcx
                    .def_ident_span(def_id)
                    .unwrap_or_else(|| self.tcx.def_span(def_id));
                let target_coords =
                    resolve_span_coords(self.tcx, target_span, self.package_root.as_deref());
                let (target_file, target_line, target_col) = match target_coords {
                    Some((f, l, c, _, _, _)) => (Some(f), l, c),
                    None => (None, 0, 0),
                };

                let target_def_id = if def_id.krate == rustc_span::def_id::LOCAL_CRATE {
                    Some(def_id.index.as_u32())
                } else {
                    None
                };

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
                if !self
                    .seen_refs
                    .insert((file.clone(), s_line, s_col, name.clone()))
                {
                    return;
                }
                let target_coords = resolve_span_coords(
                    self.tcx,
                    self.tcx.hir_span(hir_id),
                    self.package_root.as_deref(),
                );
                let (target_file, target_line, target_col) = match target_coords {
                    Some((f, l, c, _, _, _)) => (Some(f), l, c),
                    None => (None, 0, 0),
                };

                let target_unit = self
                    .krate_to_unit
                    .get(&rustc_span::def_id::LOCAL_CRATE)
                    .cloned();

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
}

impl<'tcx> Visitor<'tcx> for HirVisitor<'tcx> {
    type NestedFilter = rustc_middle::hir::nested_filter::OnlyBodies;

    fn maybe_tcx(&mut self) -> Self::MaybeTyCtxt {
        self.tcx
    }

    fn visit_nested_body(&mut self, id: hir::BodyId) {
        let prev = self.typeck.replace(self.tcx.typeck_body(id));
        self.visit_body(self.tcx.hir_body(id));
        self.typeck = prev;
    }

    fn visit_path(&mut self, path: &hir::Path<'tcx>, _: HirId) {
        let span = if let Some(last_seg) = path.segments.last() {
            last_seg.ident.span
        } else {
            path.span
        };
        self.add_ref_from_res(span, path.res);

        if path.segments.len() >= 2 {
            let pen_seg = &path.segments[path.segments.len() - 2];
            match path.res {
                Res::Def(hir::def::DefKind::Ctor(hir::def::CtorOf::Variant, _), ctor_did) => {
                    let var_did = self.tcx.parent(ctor_did);
                    let enum_did = self.tcx.parent(var_did);
                    self.add_ref_from_res(
                        pen_seg.ident.span,
                        Res::Def(hir::def::DefKind::Enum, enum_did),
                    );
                }
                Res::Def(hir::def::DefKind::Variant, var_did) => {
                    let enum_did = self.tcx.parent(var_did);
                    self.add_ref_from_res(
                        pen_seg.ident.span,
                        Res::Def(hir::def::DefKind::Enum, enum_did),
                    );
                }
                Res::Def(hir::def::DefKind::AssocFn, fn_did)
                | Res::Def(hir::def::DefKind::AssocConst { .. }, fn_did) => {
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
        if let Some(typeck) = self.typeck {
            if let hir::TyKind::Path(hir::QPath::TypeRelative(qself, segment)) = ty.kind {
                let res = typeck.qpath_res(&hir::QPath::TypeRelative(qself, segment), ty.hir_id);
                self.add_ref_from_res(segment.ident.span, res);
            }
        }
        intravisit::walk_ty(self, ty);
    }

    fn visit_expr(&mut self, expr: &'tcx hir::Expr<'tcx>) {
        if let Some(typeck) = self.typeck {
            match expr.kind {
                hir::ExprKind::MethodCall(segment, ..) => {
                    if let Some(def_id) = typeck.type_dependent_def_id(expr.hir_id) {
                        let is_trait_item = self
                            .tcx
                            .opt_associated_item(def_id)
                            .map(|a| matches!(a.container, rustc_middle::ty::AssocContainer::Trait))
                            .unwrap_or(false);
                        let final_def_id = if is_trait_item {
                            let args = typeck.node_args(expr.hir_id);
                            if !args.has_param()
                                && !args.has_infer()
                                && !args.has_escaping_bound_vars()
                                && !args.references_error()
                            {
                                let body_owner =
                                    self.tcx.hir_enclosing_body_owner(expr.hir_id).to_def_id();
                                let env = rustc_middle::ty::TypingEnv::post_analysis(
                                    self.tcx, body_owner,
                                );
                                let norm_args = self
                                    .tcx
                                    .try_normalize_erasing_regions(
                                        env,
                                        rustc_middle::ty::Unnormalized::new(args),
                                    )
                                    .unwrap_or(args);
                                if let Ok(Some(inst)) = rustc_middle::ty::Instance::try_resolve(
                                    self.tcx, env, def_id, norm_args,
                                ) {
                                    inst.def_id()
                                } else {
                                    def_id
                                }
                            } else {
                                def_id
                            }
                        } else {
                            def_id
                        };
                        self.add_ref_from_res(
                            segment.ident.span,
                            Res::Def(self.tcx.def_kind(final_def_id), final_def_id),
                        );
                    }
                }
                hir::ExprKind::Path(hir::QPath::TypeRelative(_, segment)) => {
                    if let hir::ExprKind::Path(ref qpath) = expr.kind {
                        self.add_ref_from_res(
                            segment.ident.span,
                            typeck.qpath_res(qpath, expr.hir_id),
                        );
                    }
                }
                hir::ExprKind::Field(base, field) => {
                    if let Some(adt) = typeck.expr_ty_adjusted(base).ty_adt_def() {
                        if !adt.is_enum() {
                            if let Some(def) = adt
                                .non_enum_variant()
                                .fields
                                .iter()
                                .find(|it| it.name == field.name)
                            {
                                self.add_ref_from_res(
                                    field.span,
                                    Res::Def(self.tcx.def_kind(def.did), def.did),
                                );
                            }
                        }
                    }
                }
                _ => {}
            }

            // Inferred expression type
            if let Some((file, s_line, s_col, e_line, e_col, _)) =
                resolve_span_coords(self.tcx, expr.span, self.package_root.as_deref())
            {
                if let Some(ty) = typeck.expr_ty_opt(expr) {
                    self.writer.add_type(RawType {
                        file,
                        start_line: s_line,
                        start_col: s_col,
                        end_line: e_line,
                        end_col: e_col,
                        type_name: ty.to_string(),
                    });
                }
            }
        }
        intravisit::walk_expr(self, expr);
    }
}

fn extract_target_disambiguator(args: &[String]) -> Option<String> {
    for i in 0..args.len() {
        let arg = &args[i];
        if let Some(rest) = arg.strip_prefix("-Cextra-filename=") {
            if !rest.is_empty() {
                return Some(rest.to_string());
            }
        }
        if arg == "-C" && i + 1 < args.len() {
            if let Some(rest) = args[i + 1].strip_prefix("extra-filename=") {
                if !rest.is_empty() {
                    return Some(rest.to_string());
                }
            }
        }
    }
    for i in 0..args.len() {
        let arg = &args[i];
        if let Some(rest) = arg.strip_prefix("--crate-type=") {
            if !rest.is_empty() {
                return Some(format!("_{rest}"));
            }
        }
        if arg == "--crate-type" && i + 1 < args.len() {
            return Some(format!("_{}", args[i + 1]));
        }
    }
    None
}

fn hash_directory_contents(dir: &Path) -> Option<u64> {
    if !dir.is_dir() {
        return None;
    }
    let mut entries = Vec::new();
    fn visit(d: &Path, base: &Path, list: &mut Vec<(String, u64)>) {
        if let Ok(rd) = fs::read_dir(d) {
            for entry in rd.flatten() {
                let p = entry.path();
                if p.is_file() {
                    let rel = p
                        .strip_prefix(base)
                        .unwrap_or(&p)
                        .to_string_lossy()
                        .to_string();
                    let hash = fs::read(&p).map(|b| ridx_format::fnv1a_64(&b)).unwrap_or(0);
                    list.push((rel, hash));
                } else if p.is_dir() {
                    visit(&p, base, list);
                }
            }
        }
    }
    visit(dir, dir, &mut entries);
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let mut hasher_input = Vec::new();
    for (rel, h) in entries {
        hasher_input.extend_from_slice(rel.as_bytes());
        hasher_input.extend_from_slice(&h.to_le_bytes());
    }
    Some(ridx_format::fnv1a_64(&hasher_input))
}

fn hash_file_contents(path: &Path) -> Option<u64> {
    let bytes = fs::read(path).ok()?;
    Some(ridx_format::fnv1a_64(&bytes))
}

fn compute_dependency_unit_key(
    crate_name: &str,
    crate_version: &str,
    disambiguator: Option<&str>,
    manifest_dir: Option<&Path>,
    compiler_args: &[String],
) -> String {
    let mut key_str = format!(
        "{}:{}:{}:v{}:ext{}:drv={:016x}",
        crate_name,
        crate_version,
        disambiguator.unwrap_or(""),
        ridx_format::CURRENT_FORMAT_VERSION,
        ridx_format::EXTRACTOR_VERSION,
        ridx_format::get_driver_hash(),
    );
    if let Some(m) = manifest_dir {
        key_str.push(':');
        key_str.push_str(&m.display().to_string());
    }
    // Effective target: from args or env
    let mut target = std::env::var("TARGET").ok();
    for i in 0..compiler_args.len() {
        if compiler_args[i] == "--target" && i + 1 < compiler_args.len() {
            target = Some(compiler_args[i + 1].clone());
            break;
        } else if let Some(rest) = compiler_args[i].strip_prefix("--target=") {
            target = Some(rest.to_string());
            break;
        }
    }
    if let Some(ref t) = target {
        key_str.push(':');
        key_str.push_str(t);
    }

    // Extract and sort all `--cfg` flags
    let mut cfgs = Vec::new();
    for i in 0..compiler_args.len() {
        if compiler_args[i] == "--cfg" && i + 1 < compiler_args.len() {
            cfgs.push(compiler_args[i + 1].clone());
        } else if let Some(rest) = compiler_args[i].strip_prefix("--cfg=") {
            cfgs.push(rest.to_string());
        }
    }
    cfgs.sort();
    for cfg in cfgs {
        key_str.push_str(":cfg=");
        key_str.push_str(&cfg);
    }

    // Extract and sort all `--extern` flags, hashing dependency artifact content instead of physical target paths
    let mut externs = Vec::new();
    for i in 0..compiler_args.len() {
        if compiler_args[i] == "--extern" && i + 1 < compiler_args.len() {
            externs.push(compiler_args[i + 1].clone());
        } else if let Some(rest) = compiler_args[i].strip_prefix("--extern=") {
            externs.push(rest.to_string());
        }
    }
    externs.sort();
    for ext in externs {
        if let Some((name_part, path_str)) = ext.split_once('=') {
            let p = Path::new(path_str);
            if p.is_file() {
                let digest = hash_file_contents(p).unwrap_or(0);
                key_str.push_str(&format!(":ext={name_part}:digest={:016x}", digest));
            } else {
                key_str.push_str(&format!(":ext={name_part}:missing"));
            }
        } else {
            key_str.push_str(&format!(":ext={ext}"));
        }
    }

    // Hash OUT_DIR contents if present
    if let Ok(out_dir_str) = std::env::var("OUT_DIR") {
        let out_dir_path = Path::new(&out_dir_str);
        if let Some(out_hash) = hash_directory_contents(out_dir_path) {
            key_str.push_str(&format!(":outdir={:016x}", out_hash));
        }
    }

    format!("{:016x}", ridx_format::fnv1a_64(key_str.as_bytes()))
}

fn extract_crate_items<'tcx>(
    tcx: TyCtxt<'tcx>,
    visitor: &mut HirVisitor<'tcx>,
    package_root: Option<&Path>,
) {
    // Extract all crate item definitions
    for def_id in tcx.hir_crate_items(()).definitions() {
        let item_span = tcx
            .def_ident_span(def_id)
            .unwrap_or_else(|| tcx.def_span(def_id));
        let Some((file, s_line, s_col, e_line, e_col, is_macro)) =
            resolve_span_coords(tcx, item_span, package_root)
        else {
            continue;
        };

        let kind = tcx.def_kind(def_id);
        let path = tcx.def_path_str(def_id);
        let name = path.rsplit("::").next().unwrap_or(&path).to_string();

        let signature = match kind {
            hir::def::DefKind::Fn | hir::def::DefKind::AssocFn => {
                Some(tcx.fn_sig(def_id).skip_binder().to_string())
            }
            _ => None,
        };

        visitor.writer.add_definition(RawDefinition {
            def_id: def_id.local_def_index.as_u32(),
            kind: map_def_kind(kind),
            visibility: 0,
            is_macro_expansion: is_macro,
            name,
            path,
            signature,
            file,
            start_line: s_line,
            start_col: s_col,
            end_line: e_line,
            end_col: e_col,
        });
    }

    // Traverse all item bodies for references and types
    tcx.hir_visit_all_item_likes_in_crate(visitor);
}

fn is_immutable_package(manifest_dir: &Path) -> bool {
    if let Ok(cargo_home) = std::env::var("CARGO_HOME") {
        let ch = Path::new(&cargo_home);
        if manifest_dir.starts_with(ch.join("registry")) || manifest_dir.starts_with(ch.join("git"))
        {
            return true;
        }
    }
    let s = manifest_dir.to_string_lossy();
    s.contains("/.cargo/registry/")
        || s.contains("\\.cargo\\registry\\")
        || s.contains("/.cargo/git/")
        || s.contains("\\.cargo\\git\\")
}

fn collect_file_hashes<'tcx>(
    tcx: TyCtxt<'tcx>,
    package_root: Option<&Path>,
    manifest_dir: Option<&Path>,
) -> std::collections::HashMap<String, u64> {
    let mut hashes = std::collections::HashMap::new();
    let root_scope = package_root.or(manifest_dir);
    let source_map = tcx.sess.source_map();

    for file in source_map.files().iter() {
        let name_str = source_map.filename_for_diagnostics(&file.name).to_string();
        if name_str.starts_with('<') {
            continue;
        }

        if let Some(root) = root_scope {
            let p = Path::new(&name_str);
            let abs_p = if p.is_absolute() {
                p.to_path_buf()
            } else if let Ok(cwd) = std::env::current_dir() {
                cwd.join(p)
            } else {
                p.to_path_buf()
            };
            let abs_canon = abs_p.canonicalize().unwrap_or(abs_p);
            let root_canon = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
            if !abs_canon.starts_with(&root_canon) {
                continue;
            }
        }

        let hash_opt = if let Some(ref src) = file.src {
            Some(ridx_format::normalized_source_hash(src.as_bytes()))
        } else {
            let p = Path::new(&name_str);
            hash_file_contents(p)
                .or_else(|| manifest_dir.and_then(|m| hash_file_contents(&m.join(p))))
        };

        if let Some(h) = hash_opt {
            let p = Path::new(&name_str);
            hashes.insert(name_str.clone(), h);

            if let Ok(canon) = p.canonicalize() {
                hashes.insert(canon.to_string_lossy().to_string(), h);
            }

            if let Some(root) = package_root.or(manifest_dir) {
                if let Ok(rel) = p.strip_prefix(root) {
                    hashes.insert(rel.to_string_lossy().to_string(), h);
                }
                let full = root.join(p);
                if let Ok(canon) = full.canonicalize() {
                    hashes.insert(canon.to_string_lossy().to_string(), h);
                }
            }
        }
    }

    hashes
}

fn build_crate_index(
    tcx: TyCtxt<'_>,
    out_dir: &Path,
    disambiguator: Option<&str>,
    compiler_args: &[String],
) {
    let start = Instant::now();
    let crate_name = tcx.crate_name(rustc_span::def_id::LOCAL_CRATE).to_string();
    let crate_version = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.1.0".to_string());

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").ok().map(PathBuf::from);
    let is_immutable = manifest_dir
        .as_ref()
        .map(|m| is_immutable_package(m))
        .unwrap_or(false);

    let suffix = match disambiguator {
        Some(d) if d.starts_with('-') || d.starts_with('_') => d.to_string(),
        Some(d) if !d.is_empty() => format!("_{d}"),
        _ => String::new(),
    };

    let unit_key = compute_dependency_unit_key(
        &crate_name,
        &crate_version,
        disambiguator,
        manifest_dir.as_deref(),
        compiler_args,
    );

    let deps_dir = out_dir.join(".deps");
    let _ = fs::create_dir_all(&deps_dir);

    let mut dep_keys_or_names = Vec::new();
    let mut extern_units = std::collections::HashMap::new();

    for i in 0..compiler_args.len() {
        let arg = &compiler_args[i];
        let ext_val = if arg == "--extern" && i + 1 < compiler_args.len() {
            Some(compiler_args[i + 1].as_str())
        } else {
            arg.strip_prefix("--extern=")
        };
        if let Some(val) = ext_val {
            let (name_part, path_opt) = if let Some((n, p)) = val.split_once('=') {
                (n.trim(), Some(p.trim()))
            } else if let Some((n, p)) = val.split_once(':') {
                (n.trim(), Some(p.trim()))
            } else {
                (val.trim(), None)
            };

            let clean_alias = name_part.rsplit(':').next().unwrap_or(name_part);
            if !clean_alias.is_empty() && !dep_keys_or_names.contains(&clean_alias.to_string()) {
                dep_keys_or_names.push(clean_alias.to_string());
            }

            // Look up disambiguated .deps/<dep_stem>.json for this extern
            let mut resolved_unit = None;
            if let Some(p_str) = path_opt {
                let p = Path::new(p_str);
                let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                let stem_without_lib = stem.strip_prefix("lib").unwrap_or(stem);
                let cand_files = [
                    deps_dir.join(format!("{stem_without_lib}.json")),
                    deps_dir.join(format!("{stem}.json")),
                    deps_dir.join(format!("{clean_alias}.json")),
                ];
                for c in &cand_files {
                    if c.is_file() {
                        if let Ok(content) = fs::read(c) {
                            if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&content) {
                                if let Some(uk) = val.get("unit_key").and_then(|v| v.as_str()) {
                                    resolved_unit = Some(uk.to_string());
                                    let dep_name = val
                                        .get("name")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or(clean_alias);
                                    if !dep_keys_or_names.contains(&dep_name.to_string()) {
                                        dep_keys_or_names.push(dep_name.to_string());
                                    }
                                    break;
                                }
                            }
                        }
                    }
                }
            } else {
                let c = deps_dir.join(format!("{clean_alias}.json"));
                if let Ok(content) = fs::read(&c) {
                    if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&content) {
                        resolved_unit = val
                            .get("unit_key")
                            .and_then(|v| v.as_str())
                            .map(ToString::to_string);
                    }
                }
            }

            if let Some(uk) = resolved_unit {
                extern_units.insert(clean_alias.to_string(), uk.clone());
                if !dep_keys_or_names.contains(&uk) {
                    dep_keys_or_names.push(uk);
                }
            }
        }
    }

    let mut krate_to_unit: std::collections::HashMap<rustc_span::def_id::CrateNum, String> =
        std::collections::HashMap::new();
    krate_to_unit.insert(rustc_span::def_id::LOCAL_CRATE, unit_key.clone());

    for &cnum in tcx.crates(()) {
        let csource = tcx.used_crate_source(cnum);
        let cand_path = csource
            .rmeta
            .as_deref()
            .or_else(|| csource.rlib.as_deref())
            .or_else(|| csource.dylib.as_deref());
        let mut found_unit = None;
        if let Some(p) = cand_path {
            let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            let stem_without_lib = stem.strip_prefix("lib").unwrap_or(stem);
            let cand_files = [
                deps_dir.join(format!("{stem_without_lib}.json")),
                deps_dir.join(format!("{stem}.json")),
            ];
            for c in &cand_files {
                if c.is_file() {
                    if let Ok(content) = fs::read(c) {
                        if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&content) {
                            if let Some(uk) = val.get("unit_key").and_then(|v| v.as_str()) {
                                found_unit = Some(uk.to_string());
                                break;
                            }
                        }
                    }
                }
            }
        }
        if found_unit.is_none() {
            let cname = tcx.crate_name(cnum).to_string();
            if let Some(uk) = extern_units.get(&cname) {
                found_unit = Some(uk.clone());
            }
        }
        if let Some(uk) = found_unit {
            krate_to_unit.insert(cnum, uk);
        }
    }

    if is_immutable {
        let cache_dir = ridx_format::resolve_cache_dir();
        let obj_path = ridx_format::cache_object_path(&cache_dir, &unit_key);

        let is_valid_cache = if obj_path.is_file() {
            if let Ok(reader) = ridx_format::IndexReader::open(&obj_path) {
                reader.header().version == ridx_format::CURRENT_FORMAT_VERSION
                    && reader.unit_key() == unit_key
            } else {
                false
            }
        } else {
            false
        };

        if is_valid_cache {
            let (generation, file_hashes) = {
                let reader = ridx_format::IndexReader::open(&obj_path).unwrap();
                let gen = reader.generation().to_string();
                let embedded = reader.embedded_file_hashes();
                let mut hashes =
                    collect_file_hashes(tcx, manifest_dir.as_deref(), manifest_dir.as_deref());
                ridx_format::merge_embedded_hashes_authoritative(
                    &mut hashes,
                    &embedded,
                    manifest_dir.as_deref(),
                );
                (gen, hashes)
            };
            let dep_info = serde_json::json!({
                "name": crate_name,
                "version": crate_version,
                "unit_key": unit_key,
                "driver_hash": format!("{:016x}", ridx_format::get_driver_hash()),
                "format_version": ridx_format::CURRENT_FORMAT_VERSION,
                "generation": generation,
                "object_path": obj_path.display().to_string(),
                "source_root": manifest_dir.as_ref().map(|p| p.display().to_string()),
                "is_dependency": true,
                "dependencies": dep_keys_or_names,
                "extern_units": extern_units,
                "file_hashes": file_hashes,
                "compiler_args": compiler_args,
            });
            let dep_file = deps_dir.join(format!("{crate_name}{suffix}.json"));
            let _ = fs::write(
                &dep_file,
                serde_json::to_string_pretty(&dep_info).unwrap_or_default(),
            );

            let file_size = fs::metadata(&obj_path).map(|m| m.len()).unwrap_or(0);
            eprintln!(
                "ridx: cache hit for {} v{} ({} KB, unit: {}) in {:.1} ms",
                crate_name,
                crate_version,
                file_size / 1024,
                &unit_key[..unit_key.len().min(8)],
                start.elapsed().as_secs_f64() * 1000.0
            );
            return;
        } else if obj_path.is_file() {
            let _ = fs::remove_file(&obj_path);
        }

        // Cache miss: extract index with package_root set for relative source paths
        let mut visitor = HirVisitor {
            tcx,
            typeck: None,
            package_root: manifest_dir.clone(),
            writer: IndexWriter::new(&crate_name, &crate_version),
            krate_to_unit: krate_to_unit.clone(),
            seen_refs: std::collections::HashSet::new(),
        };
        visitor.writer.set_unit_key(&unit_key);

        extract_crate_items(tcx, &mut visitor, manifest_dir.as_deref());

        let file_hashes =
            collect_file_hashes(tcx, manifest_dir.as_deref(), manifest_dir.as_deref());

        let snapshot_digest = ridx_format::compute_source_snapshot_digest(&file_hashes);
        let generation = format!("{:016x}", snapshot_digest);
        visitor.writer.set_generation(&generation);
        visitor.writer.set_file_hashes(file_hashes.clone());

        if let Some(parent) = obj_path.parent() {
            let _ = fs::create_dir_all(parent);
        }

        let tmp_file_path = obj_path.with_extension(format!("tmp.{}", std::process::id()));
        match File::create(&tmp_file_path) {
            Ok(f) => {
                let mut writer = BufWriter::new(f);
                let def_count = visitor.writer.definition_count();
                let ref_count = visitor.writer.reference_count();
                let type_count = visitor.writer.type_count();

                if let Err(e) = visitor.writer.write_to(&mut writer) {
                    eprintln!("ridx-driver: write failed for dependency {crate_name}: {e}");
                    let _ = fs::remove_file(&tmp_file_path);
                } else {
                    drop(writer);
                    if let Err(e) = fs::rename(&tmp_file_path, &obj_path) {
                        eprintln!(
                            "ridx-driver: failed to atomically rename {:?} to {:?}: {e}",
                            tmp_file_path, obj_path
                        );
                        let _ = fs::remove_file(&tmp_file_path);
                    } else {
                        // Publish sidecar metadata only after .ridx file is published
                        let dep_info = serde_json::json!({
                            "name": crate_name,
                            "version": crate_version,
                            "unit_key": unit_key,
                            "driver_hash": format!("{:016x}", ridx_format::get_driver_hash()),
                            "format_version": ridx_format::CURRENT_FORMAT_VERSION,
                            "generation": generation,
                            "object_path": obj_path.display().to_string(),
                            "source_root": manifest_dir.as_ref().map(|p| p.display().to_string()),
                            "is_dependency": true,
                            "dependencies": dep_keys_or_names,
                            "extern_units": extern_units,
                            "file_hashes": file_hashes,
                            "compiler_args": compiler_args,
                        });
                        let dep_file = deps_dir.join(format!("{crate_name}{suffix}.json"));
                        let _ = fs::write(
                            &dep_file,
                            serde_json::to_string_pretty(&dep_info).unwrap_or_default(),
                        );

                        let file_size = fs::metadata(&obj_path).map(|m| m.len()).unwrap_or(0);
                        eprintln!(
                            "ridx: indexed dependency {} v{} ({} defs, {} refs, {} types, {} KB in {:.1} ms, cached: {})",
                            crate_name,
                            crate_version,
                            def_count,
                            ref_count,
                            type_count,
                            file_size / 1024,
                            start.elapsed().as_secs_f64() * 1000.0,
                            &unit_key[..unit_key.len().min(8)]
                        );
                    }
                }
            }
            Err(e) => {
                eprintln!(
                    "ridx-driver: failed to create temporary index {:?}: {e}",
                    tmp_file_path
                );
            }
        }
        return;
    }

    // Workspace crate: write to out_dir
    if let Err(e) = fs::create_dir_all(out_dir) {
        eprintln!("ridx-driver: failed to create out_dir {:?}: {e}", out_dir);
        return;
    }

    let out_file_path = out_dir.join(format!("{crate_name}{suffix}.ridx"));

    let mut visitor = HirVisitor {
        tcx,
        typeck: None,
        package_root: None,
        writer: IndexWriter::new(&crate_name, &crate_version),
        krate_to_unit,
        seen_refs: std::collections::HashSet::new(),
    };
    visitor.writer.set_unit_key(&unit_key);

    extract_crate_items(tcx, &mut visitor, None);

    let file_hashes = collect_file_hashes(tcx, None, manifest_dir.as_deref());

    let snapshot_digest = ridx_format::compute_source_snapshot_digest(&file_hashes);
    let generation = format!("{:016x}", snapshot_digest);
    visitor.writer.set_generation(&generation);
    visitor.writer.set_file_hashes(file_hashes.clone());

    let tmp_file_path = out_dir.join(format!(
        "{crate_name}{suffix}.ridx.tmp.{}",
        std::process::id()
    ));

    match File::create(&tmp_file_path) {
        Ok(f) => {
            let mut writer = BufWriter::new(f);
            let def_count = visitor.writer.definition_count();
            let ref_count = visitor.writer.reference_count();
            let type_count = visitor.writer.type_count();

            if let Err(e) = visitor.writer.write_to(&mut writer) {
                eprintln!("ridx-driver: write failed for {crate_name}: {e}");
                let _ = fs::remove_file(&tmp_file_path);
            } else {
                drop(writer);
                if let Err(e) = fs::rename(&tmp_file_path, &out_file_path) {
                    eprintln!(
                        "ridx-driver: failed to atomically rename {:?} to {:?}: {e}",
                        tmp_file_path, out_file_path
                    );
                    let _ = fs::remove_file(&tmp_file_path);
                } else {
                    // Publish sidecar metadata only after .ridx file is published
                    let mut env_map = std::collections::HashMap::new();
                    for (k, v) in std::env::vars() {
                        if k.starts_with("CARGO_") || k == "OUT_DIR" || k == "TARGET" || k == "HOST"
                        {
                            env_map.insert(k, v);
                        }
                    }
                    if let Ok(out_dir_val) = std::env::var("OUT_DIR") {
                        let out_path = Path::new(&out_dir_val);
                        if let Some(build_dir) = out_path.parent() {
                            for cand in ["output", "root-output"] {
                                let cand_file = build_dir.join(cand);
                                if let Ok(contents) = fs::read_to_string(&cand_file) {
                                    for line in contents.lines() {
                                        let trimmed = line.trim();
                                        let env_payload = trimmed
                                            .strip_prefix("cargo:rustc-env=")
                                            .or_else(|| trimmed.strip_prefix("cargo::rustc-env="));
                                        if let Some(payload) = env_payload {
                                            if let Some((k, v)) = payload.split_once('=') {
                                                env_map.insert(
                                                    k.trim().to_string(),
                                                    v.trim().to_string(),
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    let dep_info = serde_json::json!({
                        "name": crate_name,
                        "version": crate_version,
                        "unit_key": unit_key,
                        "generation": generation,
                        "object_path": out_file_path.display().to_string(),
                        "source_root": serde_json::Value::Null,
                        "is_dependency": false,
                        "dependencies": dep_keys_or_names,
                        "extern_units": extern_units,
                        "file_hashes": file_hashes,
                        "compiler_args": compiler_args,
                        "env": env_map,
                    });
                    let dep_file = deps_dir.join(format!("{crate_name}{suffix}.json"));
                    let _ = fs::write(
                        &dep_file,
                        serde_json::to_string_pretty(&dep_info).unwrap_or_default(),
                    );

                    let file_size = fs::metadata(&out_file_path).map(|m| m.len()).unwrap_or(0);
                    eprintln!(
                        "ridx: indexed {} ({} defs, {} refs, {} types, {} KB in {:.1} ms)",
                        crate_name,
                        def_count,
                        ref_count,
                        type_count,
                        file_size / 1024,
                        start.elapsed().as_secs_f64() * 1000.0
                    );
                }
            }
        }
        Err(e) => {
            eprintln!(
                "ridx-driver: failed to create temporary index {:?}: {e}",
                tmp_file_path
            );
        }
    }
}

struct NoopCallbacks;
impl Callbacks for NoopCallbacks {}

#[allow(clippy::result_unit_err)]
pub fn run_driver(mut args: Vec<String>) -> Result<(), ()> {
    // Strip any leading compiler wrapper or rustc binary invocations passed by Cargo.
    // When Cargo chains wrappers or when multiple wrappers are configured, args may contain
    // multiple wrapper executable paths or rustc before the actual compiler flags.
    while args.len() > 1 {
        let arg1 = &args[1];
        let p = Path::new(arg1);
        let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        if stem == "rustc" || stem == "ridx-driver" || stem.ends_with("-driver") {
            args.remove(1);
        } else {
            break;
        }
    }

    let is_query = args.iter().any(|a| {
        a == "-vV"
            || a == "-V"
            || a == "--version"
            || a == "-h"
            || a == "--help"
            || a == "--print"
            || a.starts_with("--print=")
    });

    if is_query {
        let res = rustc_driver::catch_fatal_errors(|| {
            rustc_driver::run_compiler(&args, &mut NoopCallbacks)
        });
        return res.map_err(|_| ());
    }

    // Skip build scripts
    let crate_name_arg = args
        .windows(2)
        .find(|w| w[0] == "--crate-name")
        .map(|w| w[1].as_str());
    if crate_name_arg == Some("build_script_build") || crate_name_arg == Some("build_script_main") {
        let res = rustc_driver::catch_fatal_errors(|| {
            rustc_driver::run_compiler(&args, &mut NoopCallbacks)
        });
        return res.map_err(|_| ());
    }

    let out_dir = std::env::var("RIDX_OUT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("target/ridx"));
    let disambiguator = extract_target_disambiguator(&args);

    let res = rustc_driver::catch_fatal_errors(|| {
        rustc_driver::run_compiler(
            &args,
            &mut IndexCallbacks {
                out_dir,
                disambiguator,
                compiler_args: args.clone(),
            },
        )
    });
    res.map_err(|_| ())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if run_driver(args).is_err() {
        std::process::exit(1);
    }
}

#[cfg(test)]
#[cfg(test)]
pub mod tests;
