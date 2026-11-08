/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Opt-in structured expression-type sidecar dumper (`RIDX_DUMP_EXPR_TYPES=1`, Checkbox `3.7`).
//! Interns `rustc` `TypeckResults` (`expr_ty` + `expr_ty_adjusted`) directly into each workspace
//! crate shard's `TyArena` and writes `<crate><suffix>.ridx.exprty` for compiler parity checks.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ridx_format::{FnAbiKind, IndexWriter, RawTy, TyId};
use rustc_hir::intravisit::{self, Visitor};
use rustc_hir::{self as hir, def::DefKind as HirDefKind};
use rustc_middle::ty::{Ty, TyCtxt, TyKind, TypeckResults};
use rustc_span::def_id::DefId;

use crate::ty_lower::TcxTypeLowerer;

pub(crate) const EXPRTY_MAGIC: &[u8; 8] = b"RIDXETY1";
pub(crate) const DESUGARED_TY_FLAG: u32 = 0x8000_0000;

#[derive(Clone, Copy)]
pub(crate) struct ExprTyRecord {
    pub file_idx: u32,
    pub start: u32,
    pub end: u32,
    pub owner_def_idx: u32,
    pub raw_ty: u32,
    pub adj_ty_flagged: u32,
}

pub(crate) struct ExprTyDump {
    pub files: Vec<String>,
    pub records: Vec<ExprTyRecord>,
}

#[inline]
pub(crate) fn dump_enabled() -> bool {
    std::env::var_os("RIDX_DUMP_EXPR_TYPES").is_some_and(|v| v == "1")
}

#[inline]
pub(crate) fn exprty_sidecar_path(ridx_path: &Path) -> PathBuf {
    let mut os = ridx_path.as_os_str().to_os_string();
    os.push(".exprty");
    PathBuf::from(os)
}

pub(crate) fn collect_crate_expr_types<'tcx>(
    tcx: TyCtxt<'tcx>,
    lowerer: &mut TcxTypeLowerer<'tcx, '_>,
    writer: &mut IndexWriter,
    local_def_indices: &HashMap<DefId, u32>,
    package_root: Option<&Path>,
) -> ExprTyDump {
    let mut dump = ExprTyDump { files: Vec::new(), records: Vec::new() };
    let mut file_map: HashMap<String, u32> = HashMap::new();

    for local_did in tcx.hir_body_owners() {
        let kind = tcx.def_kind(local_did);
        if !matches!(kind, HirDefKind::Fn | HirDefKind::AssocFn | HirDefKind::Const { .. } | HirDefKind::Static { .. }) {
            continue;
        }
        if tcx.def_span(local_did).from_expansion() || !tcx.has_typeck_results(local_did) {
            continue;
        }
        let owner_def_idx = local_def_indices.get(&local_did.to_def_id()).copied().unwrap_or(u32::MAX);
        let typeck = tcx.typeck(local_did);
        let body = tcx.hir_body_owned_by(local_did);
        let mut collector =
            BodyExprCollector { tcx, typeck, lowerer, writer, package_root, owner_def_idx, files: &mut dump.files, file_map: &mut file_map, records: &mut dump.records };
        intravisit::walk_body(&mut collector, body);
    }

    dump.records.sort_unstable_by_key(|r| (r.file_idx, r.start, r.end, (r.adj_ty_flagged & DESUGARED_TY_FLAG != 0) as u8));
    dump
}

pub(crate) fn write_exprty_sidecar(ridx_path: &Path, dump: &ExprTyDump) {
    let target = exprty_sidecar_path(ridx_path);
    let tmp = target.with_extension(format!("exprty.tmp.{}", std::process::id()));
    let Ok(f) = File::create(&tmp) else {
        return;
    };
    let mut w = BufWriter::new(f);
    let write_res = (|| -> std::io::Result<()> {
        w.write_all(EXPRTY_MAGIC)?;
        w.write_all(&(dump.files.len() as u32).to_le_bytes())?;
        w.write_all(&(dump.records.len() as u32).to_le_bytes())?;
        for path in &dump.files {
            let bytes = path.as_bytes();
            w.write_all(&(bytes.len() as u32).to_le_bytes())?;
            w.write_all(bytes)?;
        }
        for rec in &dump.records {
            w.write_all(&rec.file_idx.to_le_bytes())?;
            w.write_all(&rec.start.to_le_bytes())?;
            w.write_all(&rec.end.to_le_bytes())?;
            w.write_all(&rec.owner_def_idx.to_le_bytes())?;
            w.write_all(&rec.raw_ty.to_le_bytes())?;
            w.write_all(&rec.adj_ty_flagged.to_le_bytes())?;
        }
        w.flush()
    })();
    drop(w);
    if write_res.is_ok() {
        let _ = fs::rename(&tmp, &target);
    } else {
        let _ = fs::remove_file(&tmp);
    }
}

struct BodyExprCollector<'tcx, 'a, 'b> {
    tcx: TyCtxt<'tcx>,
    typeck: &'tcx TypeckResults<'tcx>,
    lowerer: &'b mut TcxTypeLowerer<'tcx, 'a>,
    writer: &'b mut IndexWriter,
    package_root: Option<&'b Path>,
    owner_def_idx: u32,
    files: &'b mut Vec<String>,
    file_map: &'b mut HashMap<String, u32>,
    records: &'b mut Vec<ExprTyRecord>,
}

impl<'tcx> Visitor<'tcx> for BodyExprCollector<'tcx, '_, '_> {
    fn visit_expr(&mut self, expr: &'tcx hir::Expr<'tcx>) {
        if let hir::ExprKind::DropTemps(inner) = expr.kind {
            self.visit_expr(inner);
            return;
        }
        if !expr.span.is_dummy() && !expr.span.from_expansion() {
            if let Some(raw) = self.typeck.expr_ty_opt(expr) {
                if let Some((file_idx, start, end)) = self.resolve_byte_span(expr.span) {
                    let adj = self.typeck.expr_ty_adjusted_opt(expr).unwrap_or(raw);
                    let raw_id = lower_expr_ty(self.lowerer, raw, self.writer).0;
                    let adj_id = lower_expr_ty(self.lowerer, adj, self.writer).0 & !DESUGARED_TY_FLAG;
                    let flag = if expr.span.desugaring_kind().is_some() { DESUGARED_TY_FLAG } else { 0 };
                    self.records.push(ExprTyRecord { file_idx, start, end, owner_def_idx: self.owner_def_idx, raw_ty: raw_id, adj_ty_flagged: adj_id | flag });
                }
            }
        }
        intravisit::walk_expr(self, expr);
        if let hir::ExprKind::Closure(cl) = &expr.kind {
            intravisit::walk_body(self, self.tcx.hir_body(cl.body));
        }
    }
}

impl<'tcx> BodyExprCollector<'tcx, '_, '_> {
    fn resolve_byte_span(&mut self, span: rustc_span::Span) -> Option<(u32, u32, u32)> {
        let sm = self.tcx.sess.source_map();
        let lo = sm.lookup_byte_offset(span.lo());
        let hi = sm.lookup_byte_offset(span.hi());
        if !Arc::ptr_eq(&lo.sf, &hi.sf) || lo.pos.0 >= hi.pos.0 {
            return None;
        }
        let full = sm.filename_for_diagnostics(&lo.sf.name).to_string();
        let rel = match self.package_root.and_then(|r| Path::new(&full).strip_prefix(r).ok()) {
            Some(p) => p.to_string_lossy().into_owned(),
            None => full,
        };
        let fidx = match self.file_map.get(&rel) {
            Some(&id) => id,
            None => {
                let id = self.files.len() as u32;
                self.files.push(rel.clone());
                self.file_map.insert(rel, id);
                id
            }
        };
        Some((fidx, lo.pos.0, hi.pos.0))
    }
}

fn lower_expr_ty<'tcx>(lowerer: &mut TcxTypeLowerer<'tcx, '_>, ty: Ty<'tcx>, writer: &mut IndexWriter) -> TyId {
    if let TyKind::Closure(_did, args) = *ty.kind() {
        let sig = args.as_closure().sig().skip_binder();
        let inputs: Vec<TyId> = match sig.inputs().first().map(|t| t.kind()) {
            Some(TyKind::Tuple(list)) => list.iter().map(|t| lower_expr_ty(lowerer, t, writer)).collect(),
            _ => Vec::new(),
        };
        let output = lower_expr_ty(lowerer, sig.output(), writer);
        return writer.intern_ty(RawTy::FnPtr { inputs: &inputs, output, abi: FnAbiKind::RustCall, is_unsafe: false, is_variadic: false });
    }
    lowerer.lower_ty(ty, writer)
}
