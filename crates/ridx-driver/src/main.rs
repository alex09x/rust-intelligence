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
use rustc_middle::ty::{TyCtxt, TypeckResults};
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
        hir::def::DefKind::InlineConst => RDefKind::InlineConst,
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
                let target_coords = resolve_span_coords(
                    self.tcx,
                    self.tcx.def_span(def_id),
                    self.package_root.as_deref(),
                );
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
        self.add_ref_from_res(path.span, path.res);
        intravisit::walk_path(self, path);
    }

    fn visit_expr(&mut self, expr: &'tcx hir::Expr<'tcx>) {
        if let Some(typeck) = self.typeck {
            match expr.kind {
                hir::ExprKind::MethodCall(segment, ..) => {
                    if let Some(def_id) = typeck.type_dependent_def_id(expr.hir_id) {
                        self.add_ref_from_res(
                            segment.ident.span,
                            Res::Def(self.tcx.def_kind(def_id), def_id),
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
        "{}:{}:{}:v{}:ext{}",
        crate_name,
        crate_version,
        disambiguator.unwrap_or(""),
        ridx_format::CURRENT_FORMAT_VERSION,
        ridx_format::EXTRACTOR_VERSION,
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
        let Some((file, s_line, s_col, e_line, e_col, is_macro)) =
            resolve_span_coords(tcx, tcx.def_span(def_id), package_root)
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
    let source_map = tcx.sess.source_map();

    for file in source_map.files().iter() {
        let name_str = source_map.filename_for_diagnostics(&file.name).to_string();
        if name_str.starts_with('<') {
            continue;
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

        if obj_path.is_file() {
            let (generation, file_hashes) = if let Ok(reader) =
                ridx_format::IndexReader::open(&obj_path)
            {
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
            } else {
                let fh = collect_file_hashes(tcx, manifest_dir.as_deref(), manifest_dir.as_deref());

                let digest = ridx_format::compute_source_snapshot_digest(&fh);
                (format!("{:016x}", digest), fh)
            };
            let dep_info = serde_json::json!({
                "name": crate_name,
                "version": crate_version,
                "unit_key": unit_key,
                "generation": generation,
                "object_path": obj_path.display().to_string(),
                "source_root": manifest_dir.as_ref().map(|p| p.display().to_string()),
                "is_dependency": true,
                "dependencies": dep_keys_or_names,
                "extern_units": extern_units,
                "file_hashes": file_hashes,
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
        }

        // Cache miss: extract index with package_root set for relative source paths
        let mut visitor = HirVisitor {
            tcx,
            typeck: None,
            package_root: manifest_dir.clone(),
            writer: IndexWriter::new(&crate_name, &crate_version),
            krate_to_unit: krate_to_unit.clone(),
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
                            "generation": generation,
                            "object_path": obj_path.display().to_string(),
                            "source_root": manifest_dir.as_ref().map(|p| p.display().to_string()),
                            "is_dependency": true,
                            "dependencies": dep_keys_or_names,
                            "extern_units": extern_units,
                            "file_hashes": file_hashes,
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
        let _ = rustc_driver::catch_fatal_errors(|| {
            rustc_driver::run_compiler(&args, &mut NoopCallbacks);
        });
        return Ok(());
    }

    // Skip build scripts
    let crate_name_arg = args
        .windows(2)
        .find(|w| w[0] == "--crate-name")
        .map(|w| w[1].as_str());
    if crate_name_arg == Some("build_script_build") || crate_name_arg == Some("build_script_main") {
        let _ = rustc_driver::catch_fatal_errors(|| {
            rustc_driver::run_compiler(&args, &mut NoopCallbacks);
        });
        return Ok(());
    }

    let out_dir = std::env::var("RIDX_OUT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("target/ridx"));
    let disambiguator = extract_target_disambiguator(&args);

    let _ = rustc_driver::catch_fatal_errors(|| {
        rustc_driver::run_compiler(
            &args,
            &mut IndexCallbacks {
                out_dir,
                disambiguator,
                compiler_args: args.clone(),
            },
        );
    });
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let _ = run_driver(args);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_map_def_kind() {
        assert_eq!(map_def_kind(hir::def::DefKind::Mod), RDefKind::Mod);
        assert_eq!(map_def_kind(hir::def::DefKind::Struct), RDefKind::Struct);
        assert_eq!(map_def_kind(hir::def::DefKind::Union), RDefKind::Union);
        assert_eq!(map_def_kind(hir::def::DefKind::Enum), RDefKind::Enum);
        assert_eq!(map_def_kind(hir::def::DefKind::Variant), RDefKind::Variant);
        assert_eq!(map_def_kind(hir::def::DefKind::Trait), RDefKind::Trait);
        assert_eq!(map_def_kind(hir::def::DefKind::TyAlias), RDefKind::TyAlias);
        assert_eq!(
            map_def_kind(hir::def::DefKind::ForeignTy),
            RDefKind::ForeignTy
        );
        assert_eq!(
            map_def_kind(hir::def::DefKind::TraitAlias),
            RDefKind::TraitAlias
        );
        assert_eq!(map_def_kind(hir::def::DefKind::AssocTy), RDefKind::AssocTy);
        assert_eq!(map_def_kind(hir::def::DefKind::TyParam), RDefKind::TyParam);
        assert_eq!(map_def_kind(hir::def::DefKind::Fn), RDefKind::Fn);
        assert_eq!(
            map_def_kind(hir::def::DefKind::Const {
                is_type_const: false
            }),
            RDefKind::Const
        );
        assert_eq!(
            map_def_kind(hir::def::DefKind::ConstParam),
            RDefKind::ConstParam
        );
        assert_eq!(
            map_def_kind(hir::def::DefKind::Static {
                mutability: hir::Mutability::Not,
                safety: hir::Safety::Safe,
                nested: false
            }),
            RDefKind::Static
        );
        assert_eq!(
            map_def_kind(hir::def::DefKind::Ctor(
                hir::def::CtorOf::Struct,
                hir::def::CtorKind::Fn
            )),
            RDefKind::StructCtor
        );
        assert_eq!(
            map_def_kind(hir::def::DefKind::Ctor(
                hir::def::CtorOf::Variant,
                hir::def::CtorKind::Fn
            )),
            RDefKind::VariantCtor
        );
        assert_eq!(map_def_kind(hir::def::DefKind::AssocFn), RDefKind::AssocFn);
        assert_eq!(
            map_def_kind(hir::def::DefKind::AssocConst {
                is_type_const: false
            }),
            RDefKind::AssocConst
        );
        assert_eq!(
            map_def_kind(hir::def::DefKind::Macro(
                rustc_span::hygiene::MacroKind::Bang.into()
            )),
            RDefKind::Macro
        );
        assert_eq!(
            map_def_kind(hir::def::DefKind::ExternCrate),
            RDefKind::ExternCrate
        );
        assert_eq!(map_def_kind(hir::def::DefKind::Use), RDefKind::Use);
        assert_eq!(
            map_def_kind(hir::def::DefKind::ForeignMod),
            RDefKind::ForeignMod
        );
        assert_eq!(
            map_def_kind(hir::def::DefKind::AnonConst),
            RDefKind::AnonConst
        );
        assert_eq!(
            map_def_kind(hir::def::DefKind::InlineConst),
            RDefKind::InlineConst
        );
        assert_eq!(
            map_def_kind(hir::def::DefKind::OpaqueTy),
            RDefKind::OpaqueTy
        );
        assert_eq!(map_def_kind(hir::def::DefKind::Field), RDefKind::Field);
        assert_eq!(
            map_def_kind(hir::def::DefKind::LifetimeParam),
            RDefKind::LifetimeParam
        );
        assert_eq!(
            map_def_kind(hir::def::DefKind::GlobalAsm),
            RDefKind::GlobalAsm
        );
        assert_eq!(
            map_def_kind(hir::def::DefKind::Impl { of_trait: false }),
            RDefKind::Impl
        );
        assert_eq!(map_def_kind(hir::def::DefKind::Closure), RDefKind::Closure);
        assert_eq!(
            map_def_kind(hir::def::DefKind::SyntheticCoroutineBody),
            RDefKind::Other
        );
    }

    #[test]
    fn test_extract_target_disambiguator() {
        assert_eq!(
            extract_target_disambiguator(&["-Cextra-filename=123".to_string()]),
            Some("123".to_string())
        );
        assert_eq!(
            extract_target_disambiguator(&["-C".to_string(), "extra-filename=456".to_string()]),
            Some("456".to_string())
        );
        assert_eq!(
            extract_target_disambiguator(&["--crate-type=lib".to_string()]),
            Some("_lib".to_string())
        );
        assert_eq!(
            extract_target_disambiguator(&["--crate-type".to_string(), "bin".to_string()]),
            Some("_bin".to_string())
        );
        assert_eq!(
            extract_target_disambiguator(&["-Cextra-filename=".to_string()]),
            None
        );
        assert_eq!(
            extract_target_disambiguator(&["-C".to_string(), "extra-filename=".to_string()]),
            None
        );
        assert_eq!(
            extract_target_disambiguator(&["--crate-type=".to_string()]),
            None
        );
        assert_eq!(
            extract_target_disambiguator(&["--crate-type".to_string(), "".to_string()]),
            Some("_".to_string())
        );
        assert_eq!(extract_target_disambiguator(&["rustc".to_string()]), None);
    }

    #[test]
    fn test_hash_helpers() {
        assert_eq!(hash_directory_contents(Path::new("/nonexistent_dir")), None);
        assert_eq!(hash_file_contents(Path::new("/nonexistent_file")), None);

        let temp_dir = std::env::temp_dir().join(format!("ridx_hash_test_{}", std::process::id()));
        let _ = fs::create_dir_all(&temp_dir);
        let f1 = temp_dir.join("a.txt");
        let _ = fs::write(&f1, b"hello");
        assert!(hash_file_contents(&f1).is_some());
        assert!(hash_directory_contents(&temp_dir).is_some());
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_compute_dependency_unit_key() {
        let key1 = compute_dependency_unit_key(
            "my_crate",
            "0.1.0",
            Some("abc"),
            Some(Path::new("/path")),
            &[
                "--target".to_string(),
                "x86_64-unknown-linux-gnu".to_string(),
            ],
        );
        let key2 = compute_dependency_unit_key(
            "my_crate",
            "0.2.0",
            Some("abc"),
            Some(Path::new("/path")),
            &[
                "--target".to_string(),
                "x86_64-unknown-linux-gnu".to_string(),
            ],
        );
        assert_eq!(key1.len(), 16);
        assert_eq!(key2.len(), 16);
        assert_ne!(key1, key2);

        let temp_dir = std::env::temp_dir().join(format!("ridx_key_test_{}", std::process::id()));
        let _ = fs::create_dir_all(&temp_dir);
        let ext_file = temp_dir.join("dep.rmeta");
        fs::write(&ext_file, b"rmeta_bytes").unwrap();

        std::env::set_var("OUT_DIR", &temp_dir);
        let key3 = compute_dependency_unit_key(
            "my_crate",
            "0.1.0",
            None,
            None,
            &[
                "--target=aarch64-apple-darwin".to_string(),
                "--cfg".to_string(),
                "feat_a".to_string(),
                "--cfg=feat_b".to_string(),
                "--extern".to_string(),
                format!("dep={}", ext_file.display()),
                "--extern=plain_dep".to_string(),
                "--extern=missing_dep=/nonexistent/path.rmeta".to_string(),
            ],
        );
        assert_eq!(key3.len(), 16);
        std::env::remove_var("OUT_DIR");
        let _ = fs::remove_dir_all(&temp_dir);
    }

    static DRIVER_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn test_run_driver_queries() {
        let _ = run_driver(vec!["ridx-driver".into(), "--version".into()]);
        let _ = run_driver(vec!["ridx-driver".into(), "--help".into()]);
        let _ = run_driver(vec!["ridx-driver".into(), "-vV".into()]);
        let _ = run_driver(vec!["ridx-driver".into(), "-V".into()]);
        let _ = run_driver(vec!["ridx-driver".into(), "-h".into()]);
        let _ = run_driver(vec!["ridx-driver".into(), "--print".into(), "cfg".into()]);
        let _ = run_driver(vec!["ridx-driver".into(), "--print=cfg".into()]);
        let _ = run_driver(vec![
            "ridx-driver".into(),
            "foo-driver".into(),
            "rustc".into(),
            "--version".into(),
        ]);
        let _ = run_driver(vec![
            "ridx-driver".into(),
            "--crate-name".into(),
            "build_script_build".into(),
        ]);
        let _ = run_driver(vec![
            "ridx-driver".into(),
            "rustc".into(),
            "--crate-name".into(),
            "build_script_main".into(),
        ]);
    }

    #[test]
    fn test_driver_index_compilation() {
        let _lock = DRIVER_TEST_LOCK.lock().unwrap();
        std::env::remove_var("CARGO_MANIFEST_DIR");
        std::env::remove_var("CARGO_PKG_VERSION");
        std::env::remove_var("CARGO_HOME");

        let temp_dir =
            std::env::temp_dir().join(format!("ridx_driver_compile_test_{}", std::process::id()));
        let _ = fs::create_dir_all(&temp_dir);
        let src_file = temp_dir.join("lib.rs");
        let code = r#"
pub mod sub {
    pub fn sub_fn() -> i32 { 42 }
}
pub struct Point { pub x: i32, pub y: i32 }
pub struct TuplePt(pub i32, pub i32);
pub enum Status { Active, Inactive(u32), Detailed { code: u8 } }
pub union RawVal { pub f: f32, pub u: u32 }
pub trait Greeter {
    type Output;
    const GREETING: &'static str = "hi";
    fn greet(&self) -> String;
    fn default_greeting(&self) -> &'static str { Self::GREETING }
}
impl Greeter for Point {
    type Output = String;
    fn greet(&self) -> String { format!("{}:{}", self.x, self.y) }
}
pub const C: i32 = 123;
pub static S: &str = "abc";
pub type Pt = Point;
macro_rules! my_add {
    ($a:expr, $b:expr) => { $a + $b };
}
pub fn compute<T: Greeter>(p: &Point, g: &T) -> i32 {
    let mut val = my_add!(p.x, p.y) + C;
    let tp = TuplePt(1, 2);
    val += tp.0 + tp.1;
    let cl = |a: i32| a * 2;
    val += cl(sub::sub_fn());
    let _ = g.greet();
    let _ = g.default_greeting();
    let st = Status::Detailed { code: 7 };
    match st {
        Status::Active => 1,
        Status::Inactive(n) => n as i32,
        Status::Detailed { code } => code as i32,
    };
    val
}
"#;
        fs::write(&src_file, code).unwrap();
        let out_dir = temp_dir.join("out");
        fs::create_dir_all(&out_dir).unwrap();
        let rmeta_file = temp_dir.join("libtest_sample.rmeta");

        let sysroot = std::process::Command::new("rustc")
            .args(["--print", "sysroot"])
            .output()
            .unwrap();
        let sysroot_str = String::from_utf8_lossy(&sysroot.stdout).trim().to_string();

        let args = vec![
            "rustc".to_string(),
            src_file.to_str().unwrap().to_string(),
            "--crate-name".to_string(),
            "test_sample".to_string(),
            "--crate-type".to_string(),
            "lib".to_string(),
            "--emit=metadata".to_string(),
            "-o".to_string(),
            rmeta_file.to_str().unwrap().to_string(),
            "--sysroot".to_string(),
            sysroot_str,
        ];

        std::env::set_var("RIDX_OUT_DIR", &out_dir);
        let _ = run_driver(args);

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_is_immutable_package() {
        let _lock = DRIVER_TEST_LOCK.lock().unwrap();
        assert!(is_immutable_package(Path::new(
            "/home/user/.cargo/registry/src/github.com-1ecc6299db9ec823/foo-0.1.0"
        )));
        assert!(is_immutable_package(Path::new(
            "/home/user/.cargo/git/checkouts/bar-12345/abc"
        )));
        assert!(!is_immutable_package(Path::new(
            "/home/user/project/my_crate"
        )));

        std::env::set_var("CARGO_HOME", "/custom/cargo");
        assert!(is_immutable_package(Path::new(
            "/custom/cargo/registry/foo"
        )));
        assert!(is_immutable_package(Path::new("/custom/cargo/git/foo")));
        assert!(!is_immutable_package(Path::new("/custom/cargo/other/foo")));
        std::env::remove_var("CARGO_HOME");
    }

    #[test]
    fn test_driver_immutable_cache_hit_and_extern() {
        let _lock = DRIVER_TEST_LOCK.lock().unwrap();
        let temp_dir =
            std::env::temp_dir().join(format!("ridx_driver_cache_{}", std::process::id()));
        let _ = fs::create_dir_all(&temp_dir);
        let reg_dir = temp_dir.join(".cargo/registry/src/dep_pkg");
        fs::create_dir_all(&reg_dir).unwrap();
        let src_file = reg_dir.join("lib.rs");
        let code = "pub fn dep_calc(x: i32) -> i32 { x + 42 }\n";
        fs::write(&src_file, code).unwrap();
        let out_dir = temp_dir.join("out");
        fs::create_dir_all(&out_dir).unwrap();
        let deps_dir = out_dir.join(".deps");
        fs::create_dir_all(&deps_dir).unwrap();

        let mock_dep_json = serde_json::json!({
            "name": "mock_dep",
            "unit_key": "mock_unit_1234",
            "generation": "mock_gen_5678"
        });
        fs::write(
            deps_dir.join("mock_dep.json"),
            serde_json::to_string(&mock_dep_json).unwrap(),
        )
        .unwrap();

        let core_dep_json = serde_json::json!({
            "name": "core",
            "unit_key": "core_unit_1234",
            "generation": "core_gen_5678"
        });
        fs::write(
            deps_dir.join("core.json"),
            serde_json::to_string(&core_dep_json).unwrap(),
        )
        .unwrap();

        let alias_dep_json = serde_json::json!({
            "name": "alias_dep",
            "unit_key": "alias_unit_9999",
            "generation": "alias_gen_9999"
        });
        fs::write(
            deps_dir.join("alias_dep.json"),
            serde_json::to_string(&alias_dep_json).unwrap(),
        )
        .unwrap();

        let rmeta_file = temp_dir.join("libdep_pkg.rmeta");
        let sysroot = std::process::Command::new("rustc")
            .args(["--print", "sysroot"])
            .output()
            .unwrap();
        let sysroot_str = String::from_utf8_lossy(&sysroot.stdout).trim().to_string();

        let mock_src = temp_dir.join("mock_dep.rs");
        fs::write(&mock_src, "pub fn mock() {}\n").unwrap();
        let mock_rmeta = temp_dir.join("libmock_dep.rmeta");
        let mock_args = vec![
            "rustc".to_string(),
            mock_src.to_str().unwrap().to_string(),
            "--crate-name".to_string(),
            "mock_dep".to_string(),
            "--crate-type".to_string(),
            "lib".to_string(),
            "--emit=metadata".to_string(),
            "-o".to_string(),
            mock_rmeta.to_str().unwrap().to_string(),
            "--sysroot".to_string(),
            sysroot_str.clone(),
        ];
        let _ = run_driver(mock_args);

        let code =
            "extern crate mock_dep; pub fn dep_calc(x: i32) -> i32 { mock_dep::mock(); x + 42 }\n";
        fs::write(&src_file, code).unwrap();

        let args = vec![
            "rustc".to_string(),
            src_file.to_str().unwrap().to_string(),
            "--crate-name".to_string(),
            "dep_pkg".to_string(),
            "--crate-type".to_string(),
            "lib".to_string(),
            "--emit=metadata".to_string(),
            "-o".to_string(),
            rmeta_file.to_str().unwrap().to_string(),
            "--sysroot".to_string(),
            sysroot_str,
            "--extern".to_string(),
            format!("mock_dep={}", mock_rmeta.display()),
            "--extern".to_string(),
            "core".to_string(),
        ];

        std::env::set_var("RIDX_OUT_DIR", &out_dir);
        std::env::set_var("CARGO_MANIFEST_DIR", &reg_dir);
        std::env::set_var("CARGO_PKG_VERSION", "0.1.0");

        // Run 1: Cache miss
        let res_d1 = run_driver(args.clone());
        assert!(res_d1.is_ok());

        let cache_dir = ridx_format::resolve_cache_dir();
        let disambiguator = extract_target_disambiguator(&args);
        let unit_key = compute_dependency_unit_key(
            "dep_pkg",
            "0.1.0",
            disambiguator.as_deref(),
            Some(&reg_dir),
            &args,
        );
        let obj_path = ridx_format::cache_object_path(&cache_dir, &unit_key);
        assert!(obj_path.is_file());

        // Run 2: Cache hit (lines 773-785)
        let _ = run_driver(args.clone());

        // Run 3: Cache hit with invalid reader (lines 787-791)
        if obj_path.is_file() {
            fs::write(&obj_path, b"NOT_A_VALID_RIDX_INDEX").unwrap();
            let _ = run_driver(args.clone());
        }

        // Run 4: Dependency atomic rename failure (lines 864-869)
        let _ = fs::remove_file(&obj_path);
        let conflict_dep_dir = obj_path.join("nested");
        if fs::create_dir_all(&conflict_dep_dir).is_ok() {
            let _ = run_driver(args.clone());
            let _ = fs::remove_dir_all(&obj_path);
        }

        // Run 5: Temporary file create error for dependency (lines 904-909)
        let tmp_file_path = obj_path.with_extension(format!("tmp.{}", std::process::id()));
        if fs::create_dir_all(tmp_file_path.join("nested")).is_ok() {
            let _ = run_driver(args);
            let _ = fs::remove_dir_all(&tmp_file_path);
        }

        std::env::remove_var("CARGO_MANIFEST_DIR");
        std::env::remove_var("CARGO_PKG_VERSION");
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_driver_workspace_crate() {
        let _lock = DRIVER_TEST_LOCK.lock().unwrap();
        let temp_dir = std::env::temp_dir().join(format!("ridx_driver_ws_{}", std::process::id()));
        let _ = fs::create_dir_all(&temp_dir);
        let ws_dir = temp_dir.join("ws_pkg");
        fs::create_dir_all(&ws_dir).unwrap();
        let src_file = ws_dir.join("lib.rs");
        let code = r#"
pub struct Point { pub x: i32, pub y: i32 }
pub fn add_point(p: Point) -> i32 { p.x + p.y }
"#;
        fs::write(&src_file, code).unwrap();
        let out_dir = temp_dir.join("out");
        fs::create_dir_all(&out_dir).unwrap();
        let deps_dir = out_dir.join(".deps");
        fs::create_dir_all(&deps_dir).unwrap();

        let core_dep_json = serde_json::json!({
            "name": "core_renamed",
            "unit_key": "core_unit_ws_1234",
            "generation": "core_gen_ws_5678"
        });
        fs::write(
            deps_dir.join("core.json"),
            serde_json::to_string(&core_dep_json).unwrap(),
        )
        .unwrap();

        let rmeta_file = temp_dir.join("libws_pkg.rmeta");
        let sysroot = std::process::Command::new("rustc")
            .args(["--print", "sysroot"])
            .output()
            .unwrap();
        let sysroot_str = String::from_utf8_lossy(&sysroot.stdout).trim().to_string();

        let args = vec![
            "rustc".to_string(),
            src_file.to_str().unwrap().to_string(),
            "--crate-name".to_string(),
            "ws_pkg".to_string(),
            "--crate-type".to_string(),
            "lib".to_string(),
            "--emit=metadata".to_string(),
            "-Cextra-filename=custom123".to_string(),
            "-o".to_string(),
            rmeta_file.to_str().unwrap().to_string(),
            "--sysroot".to_string(),
            sysroot_str,
            "--extern".to_string(),
            "core".to_string(),
        ];

        std::env::set_var("RIDX_OUT_DIR", &out_dir);
        std::env::set_var("CARGO_MANIFEST_DIR", &ws_dir);
        std::env::set_var("CARGO_PKG_VERSION", "0.2.0");

        let res = run_driver(args.clone());
        assert!(res.is_ok());

        let out_ridx = out_dir.join("ws_pkg_custom123.ridx");
        assert!(out_ridx.is_file(), "expected {:?} to exist", out_ridx);
        let out_dep = deps_dir.join("ws_pkg_custom123.json");
        assert!(out_dep.is_file(), "expected {:?} to exist", out_dep);

        // Test out_dir creation failure (lines 916-917)
        let not_a_dir = temp_dir.join("file_not_dir");
        fs::write(&not_a_dir, b"file").unwrap();
        std::env::set_var("RIDX_OUT_DIR", &not_a_dir);
        let _ = run_driver(args.clone());
        std::env::set_var("RIDX_OUT_DIR", &out_dir);

        // Test workspace crate atomic rename failure (lines 958-963)
        let _ = fs::remove_file(&out_ridx);
        let conflict_dir = out_ridx.join("nested");
        if fs::create_dir_all(&conflict_dir).is_ok() {
            let _ = run_driver(args.clone());
            let _ = fs::remove_dir_all(&out_ridx);
        }

        // Test workspace crate temp file creation failure (lines 996-1001)
        let tmp_file_path =
            out_dir.join(format!("ws_pkg_custom123.ridx.tmp.{}", std::process::id()));
        if fs::create_dir_all(tmp_file_path.join("nested")).is_ok() {
            let _ = run_driver(args.clone());
            let _ = fs::remove_dir_all(&tmp_file_path);
        }

        // Also test driver wrapper stripping and query flags
        let query_args = vec![
            "ridx-driver".to_string(),
            "rustc".to_string(),
            "-vV".to_string(),
        ];
        assert!(run_driver(query_args).is_ok());

        // Also test help and print query flags
        assert!(run_driver(vec!["ridx-driver".to_string(), "--help".to_string()]).is_ok());
        assert!(run_driver(vec![
            "ridx-driver".to_string(),
            "--print=sysroot".to_string()
        ])
        .is_ok());

        // Also test build script skipping
        let bs_args = vec![
            "ridx-driver".to_string(),
            "--crate-name".to_string(),
            "build_script_build".to_string(),
        ];
        assert!(run_driver(bs_args).is_ok());

        // Disambiguator extraction edge cases (lines 325-348)
        assert_eq!(
            extract_target_disambiguator(&["-C".to_string(), "extra-filename=bar".to_string()]),
            Some("bar".to_string())
        );
        assert_eq!(
            extract_target_disambiguator(&["-C".to_string(), "extra-filename=".to_string()]),
            None
        );
        assert_eq!(
            extract_target_disambiguator(&["-Cextra-filename=".to_string()]),
            None
        );
        assert_eq!(
            extract_target_disambiguator(&["--crate-type=rlib".to_string()]),
            Some("_rlib".to_string())
        );
        assert_eq!(
            extract_target_disambiguator(&["--crate-type".to_string(), "dylib".to_string()]),
            Some("_dylib".to_string())
        );

        std::env::remove_var("CARGO_MANIFEST_DIR");
        std::env::remove_var("CARGO_PKG_VERSION");
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_hash_directory_source_with_subdirs() {
        let temp_dir = std::env::temp_dir().join(format!("ridx_hash_dir_{}", std::process::id()));
        let sub = temp_dir.join("sub");
        let _ = fs::create_dir_all(&sub);
        let nested_file = sub.join("nested.rs");
        fs::write(&nested_file, b"pub fn nested() {}").unwrap();
        let h = hash_directory_contents(&temp_dir);
        assert!(h.is_some());
        assert!(hash_directory_contents(&nested_file).is_none());
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_driver_main_and_disambiguator_extra() {
        // Disambiguator -C without extra-filename (line 335)
        assert_eq!(
            extract_target_disambiguator(&["-C".to_string(), "opt-level=3".to_string()]),
            None
        );

        // Driver main entrypoint (lines 1071-1074)
        main();
    }
}
