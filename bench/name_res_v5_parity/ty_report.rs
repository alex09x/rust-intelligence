/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! `.ridx.exprty` sidecar loader, strict-vs-lenient structural `InferTyId` oracle comparator
//! (`tys_eq_mode`), and Phase `3.4` + `3.7` 7-corpus summary table printer.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use ridx_format::DefKind;
use ridx_syntax::name_res::{FileId, GlobalDefId};
use ridx_syntax::ty_infer::{InferTyArena, InferTyId, InferTyKind, TySliceRange};

use crate::compare::CorpusReport;
use ridx::live::meta::CorpusSourceDb;
use crate::ty_funnel::print_funnel_strictness_and_latency;
use ridx::live::v5_db::RidxV5ItemDb;

pub const EXPRTY_MAGIC: &[u8; 8] = b"RIDXETY1";
pub const DESUGARED_TY_FLAG: u32 = 0x8000_0000;

pub const EQ_STRICT: u8 = 0;
pub const EQ_LENIENT_DYN_OPAQUE: u8 = 1 << 0;
pub const EQ_LENIENT_ARRAY_LEN: u8 = 1 << 1;

#[derive(Clone, Copy)]
pub struct ExprTyRec {
    pub start: u32,
    pub end: u32,
    pub owner_def_idx: u32,
    pub raw_ty: u32,
    pub adj_ty_flagged: u32,
}

#[derive(Default)]
pub struct ExprTySidecar {
    pub total_recs: usize,
    pub unmapped_file_recs: usize,
    pub by_file: BTreeMap<u32, Vec<ExprTyRec>>,
}

pub fn load_exprty_sidecar(ridx_path: &Path, source_root: Option<&Path>, src: &CorpusSourceDb) -> Option<ExprTySidecar> {
    let mut p = ridx_path.as_os_str().to_os_string();
    p.push(".exprty");
    let bytes = fs::read(Path::new(&p)).ok()?;
    if bytes.len() < 16 || &bytes[0..8] != EXPRTY_MAGIC {
        return None;
    }
    let files_len = u32::from_le_bytes(bytes[8..12].try_into().ok()?) as usize;
    let recs_len = u32::from_le_bytes(bytes[12..16].try_into().ok()?) as usize;
    let mut off = 16usize;
    let mut fmap: Vec<FileId> = Vec::with_capacity(files_len);
    for _ in 0..files_len {
        let slen = u32::from_le_bytes(bytes.get(off..off + 4)?.try_into().ok()?) as usize;
        off += 4;
        let s = std::str::from_utf8(bytes.get(off..off + slen)?).ok()?;
        off += slen;
        fmap.push(src.lookup_abs_or_rel(source_root, s).unwrap_or(FileId::NONE));
    }
    let mut sidecar = ExprTySidecar { total_recs: recs_len, unmapped_file_recs: 0, by_file: BTreeMap::new() };
    for _ in 0..recs_len {
        let chunk = bytes.get(off..off + 24)?;
        off += 24;
        let fidx = u32::from_le_bytes(chunk[0..4].try_into().unwrap()) as usize;
        let fid = fmap.get(fidx).copied().unwrap_or(FileId::NONE);
        if !fid.is_some() {
            sidecar.unmapped_file_recs += 1;
            continue;
        }
        sidecar.by_file.entry(fid.0).or_default().push(ExprTyRec {
            start: u32::from_le_bytes(chunk[4..8].try_into().unwrap()),
            end: u32::from_le_bytes(chunk[8..12].try_into().unwrap()),
            owner_def_idx: u32::from_le_bytes(chunk[12..16].try_into().unwrap()),
            raw_ty: u32::from_le_bytes(chunk[16..20].try_into().unwrap()),
            adj_ty_flagged: u32::from_le_bytes(chunk[20..24].try_into().unwrap()),
        });
    }
    for v in sidecar.by_file.values_mut() {
        v.sort_unstable_by_key(|r| (r.start, r.end, (r.adj_ty_flagged & DESUGARED_TY_FLAG != 0) as u8));
    }
    Some(sidecar)
}

pub fn ty_contains_unknown(arena: &InferTyArena, ty: InferTyId, depth: u8) -> bool {
    if depth > 24 || !ty.is_some() {
        return true;
    }
    match arena.kind(ty) {
        InferTyKind::Unknown | InferTyKind::Infer(_) => true,
        InferTyKind::Ref { inner, .. } | InferTyKind::Ptr { inner, .. } | InferTyKind::Slice(inner) | InferTyKind::Array { elem: inner, .. } => {
            ty_contains_unknown(arena, inner, depth + 1)
        }
        InferTyKind::Tuple(r) | InferTyKind::Adt { args: r, .. } | InferTyKind::FnPtr { sig: r } | InferTyKind::FnDef { args: r, .. } | InferTyKind::Closure { sig: r, .. } => {
            arena.slice(r).iter().any(|&e| ty_contains_unknown(arena, e, depth + 1))
        }
        InferTyKind::Alias { self_ty, args, .. } => ty_contains_unknown(arena, self_ty, depth + 1) || arena.slice(args).iter().any(|&e| ty_contains_unknown(arena, e, depth + 1)),
        _ => false,
    }
}

/// Compares `a` and `b`, returning `Some(EQ_STRICT)` (`0`) when strictly structurally equal, or
/// bitflags (`EQ_LENIENT_DYN_OPAQUE` / `EQ_LENIENT_ARRAY_LEN`) when equal ONLY via lenient rules.
pub fn tys_eq_mode(db: &RidxV5ItemDb<'_>, arena: &InferTyArena, a: InferTyId, b: InferTyId, depth: u8) -> Option<u8> {
    if a == b && !ty_contains_unknown(arena, a, 0) {
        return Some(EQ_STRICT);
    }
    if depth > 24 || !a.is_some() || !b.is_some() {
        return None;
    }
    match (arena.kind(a), arena.kind(b)) {
        (InferTyKind::Unknown | InferTyKind::Infer(_), _) | (_, InferTyKind::Infer(_)) => None,
        (_, InferTyKind::Unknown) => (depth > 0 && !ty_contains_unknown(arena, a, 0)).then_some(EQ_LENIENT_DYN_OPAQUE),
        (InferTyKind::Never, InferTyKind::Never) => Some(EQ_STRICT),
        (InferTyKind::Prim(pa), InferTyKind::Prim(pb)) => (pa == pb).then_some(EQ_STRICT),
        (InferTyKind::Param { idx: ia, .. }, InferTyKind::Param { idx: ib, .. }) => (ia == ib).then_some(EQ_STRICT),
        (InferTyKind::Ref { is_mut: ma, inner: ia }, InferTyKind::Ref { is_mut: mb, inner: ib })
        | (InferTyKind::Ptr { is_mut: ma, inner: ia }, InferTyKind::Ptr { is_mut: mb, inner: ib }) => {
            if ma == mb {
                tys_eq_mode(db, arena, ia, ib, depth + 1)
            } else {
                None
            }
        }
        (InferTyKind::Slice(ea), InferTyKind::Slice(eb)) => tys_eq_mode(db, arena, ea, eb, depth + 1),
        (InferTyKind::Array { elem: ea, len: la }, InferTyKind::Array { elem: eb, len: lb }) => {
            let m = tys_eq_mode(db, arena, ea, eb, depth + 1)?;
            if la == lb {
                Some(m)
            } else if la == u64::MAX || lb == u64::MAX {
                Some(m | EQ_LENIENT_ARRAY_LEN)
            } else {
                None
            }
        }
        (InferTyKind::Tuple(ra), InferTyKind::Tuple(rb)) | (InferTyKind::FnPtr { sig: ra }, InferTyKind::FnPtr { sig: rb }) => slice_tys_eq_mode(db, arena, ra, rb, depth + 1),
        (InferTyKind::Closure { def_start: sa, def_end: ea, sig: ra }, InferTyKind::Closure { def_start: sb, def_end: eb, sig: rb }) => {
            (sa == sb && ea == eb).then(|| slice_tys_eq_mode(db, arena, ra, rb, depth + 1)).flatten()
        }
        (InferTyKind::Adt { def: da, args: ra }, InferTyKind::Adt { def: d2, args: rb }) | (InferTyKind::FnDef { def: da, args: ra }, InferTyKind::FnDef { def: d2, args: rb }) => {
            if defs_same_item(db, da, d2) {
                adt_args_eq_mode(db, arena, ra, rb, depth + 1)
            } else {
                None
            }
        }
        (InferTyKind::Dyn { principal: da, args: ra }, InferTyKind::Dyn { principal: d2, args: rb })
        | (InferTyKind::Opaque { def: da, args: ra }, InferTyKind::Opaque { def: d2, args: rb }) => {
            if !defs_same_item(db, da, d2) {
                return None;
            }
            if let Some(m) = slice_tys_eq_mode(db, arena, ra, rb, depth + 1) {
                Some(m)
            } else {
                Some(EQ_LENIENT_DYN_OPAQUE)
            }
        }
        (InferTyKind::Alias { trait_def: ta, assoc_def: aa, self_ty: sa, .. }, InferTyKind::Alias { trait_def: tb, assoc_def: ab, self_ty: sb, .. }) => {
            if defs_same_item(db, ta, tb) && defs_same_item(db, aa, ab) {
                tys_eq_mode(db, arena, sa, sb, depth + 1)
            } else {
                None
            }
        }
        _ => None,
    }
}

pub(crate) fn defs_same_item(db: &RidxV5ItemDb<'_>, a: GlobalDefId, b: GlobalDefId) -> bool {
    if !a.is_some() || !b.is_some() {
        return false;
    }
    if a == b {
        return true;
    }
    if a.is_overlay() || b.is_overlay() {
        return false;
    }
    let (ra, rb) = (a.krate.0 as usize, b.krate.0 as usize);
    let (Some(ma), Some(mb)) = (db.metas.get(ra), db.metas.get(rb)) else {
        return false;
    };
    if ma.crate_name != mb.crate_name {
        return false;
    }
    let (rdr_a, rdr_b) = (&db.ws.readers()[ra], &db.ws.readers()[rb]);
    let (Some(da), Some(d2)) = (rdr_a.definitions().get(a.index as usize), rdr_b.definitions().get(b.index as usize)) else {
        return false;
    };
    let (ka, kb) = (DefKind::from(da.kind), DefKind::from(d2.kind));
    let kind_compat = ka == kb
        || matches!(
            (ka, kb),
            (DefKind::Struct, DefKind::StructCtor)
                | (DefKind::StructCtor, DefKind::Struct)
                | (DefKind::Variant, DefKind::VariantCtor)
                | (DefKind::VariantCtor, DefKind::Variant)
                | (DefKind::Struct, DefKind::Fn)
                | (DefKind::Fn, DefKind::Struct)
        );
    if !kind_compat {
        return false;
    }
    let (pa, pb) = (rdr_a.get_str(da.path_offset()), rdr_b.get_str(d2.path_offset()));
    if pa == pb {
        return true;
    }
    let (sa, sb) = (da.span(), d2.span());
    ra == rb && sa.file_id() == sb.file_id() && sa.start_line() > 0 && sa.start_line() == sb.start_line() && sa.start_col() == sb.start_col()
}

fn slice_tys_eq_mode(db: &RidxV5ItemDb<'_>, arena: &InferTyArena, ra: TySliceRange, rb: TySliceRange, depth: u8) -> Option<u8> {
    let (sa, sb) = (arena.slice(ra), arena.slice(rb));
    if sa.len() != sb.len() {
        return None;
    }
    let mut acc = EQ_STRICT;
    for (&x, &y) in sa.iter().zip(sb.iter()) {
        acc |= tys_eq_mode(db, arena, x, y, depth)?;
    }
    Some(acc)
}

fn adt_args_eq_mode(db: &RidxV5ItemDb<'_>, arena: &InferTyArena, ra: TySliceRange, rb: TySliceRange, depth: u8) -> Option<u8> {
    let (sa, sb) = (arena.slice(ra), arena.slice(rb));
    let common = sa.len().min(sb.len());
    let mut acc = EQ_STRICT;
    for i in 0..common {
        if sa[i] == InferTyId::UNKNOWN && sb[i] == InferTyId::UNKNOWN {
            continue;
        }
        acc |= tys_eq_mode(db, arena, sa[i], sb[i], depth)?;
    }
    Some(acc)
}

pub fn print_ty_parity_tables(all_reports: &[CorpusReport], verbose_samples: usize) {
    if all_reports.is_empty() {
        return;
    }
    println!("\n=== Phase `3.4` + `3.7` Live Type Inference, `.method()`/`.field` (`Excl Dot`), & Compiler `TypeckResults` Parity ===");
    println!(
        "| Corpus | Bodies | Exprs vs `rustc` | Expr Match % | Expr Unk | Expr Diff | Dot CST Match | Dot CST % | Dot MacroTT | Dot Total / Excl Dot | Unified % (`+Dot`) | warm ns/fn | warm allocs/fn |"
    );
    println!("|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|");

    let (mut g_fns, mut g_ecmp, mut g_emat, mut g_eunk, mut g_ediff) = (0usize, 0usize, 0usize, 0usize, 0usize);
    let (mut g_dc_t, mut g_dc_m, mut g_dm_t, mut g_dm_m, mut g_dot_all) = (0usize, 0usize, 0usize, 0usize, 0usize);
    let (mut g_u_num, mut g_u_den) = (0usize, 0usize);
    let (mut g_ns, mut g_allocs) = (0u128, 0u64);

    for r in all_reports {
        let t = &r.ty_stats;
        let epct = if t.expr_compared == 0 { 0.0 } else { (t.expr_matched as f64) * 100.0 / (t.expr_compared as f64) };
        let dcpct = if t.dot_cst_total == 0 { 100.0 } else { (t.dot_cst_matched as f64) * 100.0 / (t.dot_cst_total as f64) };
        let dot_mat = t.dot_cst_matched + t.dot_mtt_matched;
        let dot_pct = if r.excluded_dot_refs == 0 { 100.0 } else { (dot_mat as f64) * 100.0 / (r.excluded_dot_refs as f64) };
        let u_num = r.matched_refs + r.matched_macro_arg_refs + dot_mat;
        let u_den = r.phase2_driver_refs + r.macro_arg_refs + r.excluded_dot_refs;
        let upct = if u_den == 0 { 100.0 } else { (u_num as f64) * 100.0 / (u_den as f64) };
        let fdiv = t.fns_inferred.max(1) as f64;
        let ns_fn = (t.warm_fn_ns as f64) / fdiv;
        let al_fn = (t.warm_fn_allocs as f64) / fdiv;

        println!(
            "| `{}` | {} | {}/{} | **{:.2}%** | {} | {} | {}/{} | **{:.2}%** | {}/{} | {}/{} ({:.2}%) | **{:.2}%** | {:.1} | {:.2} |",
            r.corpus,
            t.fns_inferred,
            t.expr_matched,
            t.expr_compared,
            epct,
            t.expr_our_unknown,
            t.expr_wrong_ty,
            t.dot_cst_matched,
            t.dot_cst_total,
            dcpct,
            t.dot_mtt_matched,
            t.dot_mtt_total,
            dot_mat,
            r.excluded_dot_refs,
            dot_pct,
            upct,
            ns_fn,
            al_fn,
        );

        g_fns += t.fns_inferred;
        g_ecmp += t.expr_compared;
        g_emat += t.expr_matched;
        g_eunk += t.expr_our_unknown;
        g_ediff += t.expr_wrong_ty;
        g_dc_t += t.dot_cst_total;
        g_dc_m += t.dot_cst_matched;
        g_dm_t += t.dot_mtt_total;
        g_dm_m += t.dot_mtt_matched;
        g_dot_all += r.excluded_dot_refs;
        g_u_num += u_num;
        g_u_den += u_den;
        g_ns += t.warm_fn_ns;
        g_allocs += t.warm_fn_allocs;
    }

    let g_epct = if g_ecmp == 0 { 0.0 } else { (g_emat as f64) * 100.0 / (g_ecmp as f64) };
    let g_dcpct = if g_dc_t == 0 { 100.0 } else { (g_dc_m as f64) * 100.0 / (g_dc_t as f64) };
    let g_dmat = g_dc_m + g_dm_m;
    let g_dpct = if g_dot_all == 0 { 100.0 } else { (g_dmat as f64) * 100.0 / (g_dot_all as f64) };
    let g_upct = if g_u_den == 0 { 100.0 } else { (g_u_num as f64) * 100.0 / (g_u_den as f64) };
    let g_fdiv = g_fns.max(1) as f64;
    println!(
        "| **TOTAL** | **{}** | **{}/{}** | **{:.2}%** | **{}** | **{}** | **{}/{}** | **{:.2}%** | **{}/{}** | **{}/{} ({:.2}%)** | **{:.2}%** | **{:.1}** | **{:.2}** |",
        g_fns,
        g_emat,
        g_ecmp,
        g_epct,
        g_eunk,
        g_ediff,
        g_dc_m,
        g_dc_t,
        g_dcpct,
        g_dm_m,
        g_dm_t,
        g_dmat,
        g_dot_all,
        g_dpct,
        g_upct,
        (g_ns as f64) / g_fdiv,
        (g_allocs as f64) / g_fdiv,
    );

    print_funnel_strictness_and_latency(all_reports);

    let mut global_ebuck: BTreeMap<String, usize> = BTreeMap::new();
    for r in all_reports {
        for (k, &v) in &r.ty_stats.expr_miss_buckets {
            *global_ebuck.entry(k.clone()).or_insert(0) += v;
        }
    }
    let mut g_sorted: Vec<(&String, &usize)> = global_ebuck.iter().collect();
    g_sorted.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
    println!("\n=== Top-12 Global `EXPR_UNK` / `EXPR_DIFF` Buckets (7 Corpora) ===");
    for (idx, (k, v)) in g_sorted.iter().take(12).enumerate() {
        println!("  {:>2}. {:>7}  {}", idx + 1, v, k);
    }

    if verbose_samples > 0 {
        for r in all_reports {
            let t = &r.ty_stats;
            if !t.dot_miss_buckets.is_empty() || !t.expr_miss_buckets.is_empty() {
                println!("\n=== Type/Dot Diagnostics for `{}` ===", r.corpus);
                let mut dbuck: Vec<(&String, &usize)> = t.dot_miss_buckets.iter().collect();
                dbuck.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
                for (k, v) in dbuck.iter().take(6) {
                    println!("  dot  {:>6}  {}", v, k);
                }
                for s in t.dot_samples.iter().take(verbose_samples) {
                    println!("    dot sample: {s}");
                }
                let mut ebuck: Vec<(&String, &usize)> = t.expr_miss_buckets.iter().collect();
                ebuck.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
                for (k, v) in ebuck.iter().take(8) {
                    println!("  expr {:>6}  {}", v, k);
                }
                for s in t.expr_samples.iter().take(verbose_samples) {
                    println!("    expr sample: {s}");
                }
            }
        }
    }
}
