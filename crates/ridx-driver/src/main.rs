//! Compiler driver wrapper that intercepts compilation during `cargo check` and writes
//! a zero-copy `.ridx` binary index file for the compiled crate.
#![feature(rustc_private)]

extern crate rustc_driver;
extern crate rustc_hir;
extern crate rustc_interface;
extern crate rustc_middle;
extern crate rustc_span;

use std::fs::{self, File};
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::time::Instant;

use ridx_format::{
    DefKind as RDefKind, IndexWriter, RawDefinition, RawReference, RawType,
};
use rustc_driver::{Callbacks, Compilation};
use rustc_hir::{self as hir, def::Res, intravisit::{self, Visitor}, HirId};
use rustc_middle::ty::{TyCtxt, TypeckResults};
use rustc_span::Span;

struct IndexCallbacks {
    out_dir: PathBuf,
}

impl Callbacks for IndexCallbacks {
    fn after_analysis<'tcx>(
        &mut self,
        _compiler: &rustc_interface::interface::Compiler,
        tcx: TyCtxt<'tcx>,
    ) -> Compilation {
        build_crate_index(tcx, &self.out_dir);
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

fn resolve_span_coords(tcx: TyCtxt<'_>, span: Span) -> Option<(String, u32, u32, u32, u32)> {
    if span.is_dummy() {
        return None;
    }
    // Remap macro expansions to their outer call-site in source code
    let actual_span = if span.from_expansion() {
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
    let file = source_map.filename_for_diagnostics(&lo.file.name).to_string();
    Some((
        file,
        lo.line as u32,
        (lo.col.0 + 1) as u32,
        hi.line as u32,
        (hi.col.0 + 1) as u32,
    ))
}

struct HirVisitor<'tcx> {
    tcx: TyCtxt<'tcx>,
    typeck: Option<&'tcx TypeckResults<'tcx>>,
    writer: IndexWriter,
}

impl<'tcx> HirVisitor<'tcx> {
    fn add_ref_from_res(&mut self, span: Span, res: Res) {
        let Some((file, s_line, s_col, e_line, e_col)) = resolve_span_coords(self.tcx, span) else {
            return;
        };

        match res {
            Res::Def(kind, def_id) => {
                let target_path = self.tcx.def_path_str(def_id);
                let target_coords = resolve_span_coords(self.tcx, self.tcx.def_span(def_id));
                let (target_file, target_line, target_col) = match target_coords {
                    Some((f, l, c, _, _)) => (Some(f), l, c),
                    None => (None, 0, 0),
                };

                let target_def_id = if def_id.krate == rustc_span::def_id::LOCAL_CRATE {
                    Some(def_id.index.as_u32())
                } else {
                    None
                };

                self.writer.add_reference(RawReference {
                    file,
                    start_line: s_line,
                    start_col: s_col,
                    end_line: e_line,
                    end_col: e_col,
                    target_def_id,
                    target_path,
                    target_file,
                    target_line,
                    target_col,
                    kind: map_def_kind(kind),
                    is_write: false,
                });
            }
            Res::Local(hir_id) => {
                let name = self.tcx.hir_name(hir_id).to_string();
                let target_coords = resolve_span_coords(self.tcx, self.tcx.hir_span(hir_id));
                let (target_file, target_line, target_col) = match target_coords {
                    Some((f, l, c, _, _)) => (Some(f), l, c),
                    None => (None, 0, 0),
                };

                self.writer.add_reference(RawReference {
                    file,
                    start_line: s_line,
                    start_col: s_col,
                    end_line: e_line,
                    end_col: e_col,
                    target_def_id: None,
                    target_path: name,
                    target_file,
                    target_line,
                    target_col,
                    kind: RDefKind::Local,
                    is_write: false,
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
                        self.add_ref_from_res(segment.ident.span, typeck.qpath_res(qpath, expr.hir_id));
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
            if let Some((file, s_line, s_col, e_line, e_col)) =
                resolve_span_coords(self.tcx, expr.span)
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

fn build_crate_index(tcx: TyCtxt<'_>, out_dir: &Path) {
    let start = Instant::now();
    let crate_name = tcx.crate_name(rustc_span::def_id::LOCAL_CRATE).to_string();
    let crate_version =
        std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.1.0".to_string());
    let mut visitor = HirVisitor {
        tcx,
        typeck: None,
        writer: IndexWriter::new(&crate_name, &crate_version),
    };

    // Extract all crate item definitions
    for def_id in tcx.hir_crate_items(()).definitions() {
        let Some((file, s_line, s_col, e_line, e_col)) =
            resolve_span_coords(tcx, tcx.def_span(def_id))
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
    tcx.hir_visit_all_item_likes_in_crate(&mut visitor);

    if let Err(e) = fs::create_dir_all(out_dir) {
        eprintln!("ridx-driver: failed to create out_dir {:?}: {e}", out_dir);
        return;
    }

    let out_file_path = out_dir.join(format!("{crate_name}.ridx"));
    match File::create(&out_file_path) {
        Ok(f) => {
            let mut writer = BufWriter::new(f);
            let def_count = visitor.writer.definition_count();
            let ref_count = visitor.writer.reference_count();
            let type_count = visitor.writer.type_count();

            if let Err(e) = visitor.writer.write_to(&mut writer) {
                eprintln!("ridx-driver: write failed for {crate_name}: {e}");
            } else {
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
        Err(e) => {
            eprintln!("ridx-driver: failed to create {:?}: {e}", out_file_path);
        }
    }
}

struct NoopCallbacks;
impl Callbacks for NoopCallbacks {}

fn main() {
    let mut args: Vec<String> = std::env::args().collect();

    // When invoked as RUSTC_WORKSPACE_WRAPPER or RUSTC_WRAPPER, cargo passes the rustc binary as args[1]
    if args.len() > 1 && (args[1].ends_with("rustc") || args[1] == "rustc") {
        args.remove(1);
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
        rustc_driver::run_compiler(&args, &mut NoopCallbacks);
        return;
    }

    let out_dir = std::env::var("RIDX_OUT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("target/ridx"));

    rustc_driver::run_compiler(&args, &mut IndexCallbacks { out_dir });
}
