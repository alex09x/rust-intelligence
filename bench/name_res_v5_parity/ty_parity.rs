/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Phase `3.3` + `3.7` live body type-inference runner, `.method()`/`.field` (`Excl Dot`)
//! reference matcher, and `.ridx.exprty` compiler `TypeckResults` per-function comparator.

use std::collections::HashMap;
use std::time::Instant;

use memmap2::Mmap;
use ridx_format::{DefKind, IndexReader, RefEntry, TyId, TyKindView};
use ridx_syntax::name_res::{eval_outer_attrs, CrateDefMap, CrateId, FileId, FileNameRes, GlobalDefId, ItemKind, LocalItemId, ResTarget};
use ridx_syntax::ty_infer::autoderef::normalize_ty;
use ridx_syntax::ty_infer::{BodyInferCtx, FnInferResult, InferTyId, LocalCstDefs, ResolvedDotRef};
use ridx_syntax::{NodeId, SyntaxKind as K, SyntaxTree};

use crate::meta::CorpusFile;
use crate::target_match::ref_targets_match;
use crate::ty_report::{ty_contains_unknown, tys_eq, ExprTyRec, ExprTySidecar, DESUGARED_TY_FLAG};
use crate::v5_db::RidxV5ItemDb;
use crate::v5_ty_lower::{lower_v5_ty, ParamRemap};
use crate::{reset_alloc_counters, snapshot_alloc_counters};

#[derive(Default)]
pub struct TyParityStats {
    pub fns_inferred: usize,
    pub warm_fn_ns: u128,
    pub warm_fn_allocs: u64,
    pub warm_fn_bytes: u64,
    pub dot_cst_total: usize,
    pub dot_cst_matched: usize,
    pub dot_mtt_total: usize,
    pub dot_mtt_matched: usize,
    pub dot_miss_buckets: HashMap<String, usize>,
    pub dot_samples: Vec<String>,
    pub expr_compared: usize,
    pub expr_matched: usize,
    pub expr_our_unknown: usize,
    pub expr_wrong_ty: usize,
    pub expr_miss_buckets: HashMap<String, usize>,
    pub expr_samples: Vec<String>,
}

pub fn collect_active_body_nodes(st: &SyntaxTree<'_>, db: &RidxV5ItemDb<'_>, krate: CrateId, out: &mut Vec<NodeId>) {
    out.clear();
    collect_bodies_rec(st, st.root(), db, krate, out);
}

fn collect_bodies_rec(st: &SyntaxTree<'_>, parent: NodeId, db: &RidxV5ItemDb<'_>, krate: CrateId, out: &mut Vec<NodeId>) {
    for ch in st.children(parent) {
        let k = st.kind(ch);
        if matches!(k, K::Fn | K::Const | K::Static | K::Module | K::Impl | K::Trait) && !eval_outer_attrs(st, ch, db, krate).enabled {
            continue;
        }
        match k {
            K::Fn => {
                if st.child(ch, K::BlockExpr).is_some() {
                    out.push(ch);
                }
                if let Some(bl) = st.child(ch, K::BlockExpr) {
                    collect_bodies_rec(st, bl, db, krate, out);
                }
            }
            K::Const | K::Static => {
                if st.children(ch).any(|c| st.kind(c).is_expr()) {
                    out.push(ch);
                }
            }
            K::Module | K::Impl | K::Trait | K::ItemList | K::BlockExpr => collect_bodies_rec(st, ch, db, krate, out),
            _ => {}
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn run_file_ty_infer_and_expr_parity(
    r_idx: usize,
    fid: FileId,
    cf: &CorpusFile,
    st: &SyntaxTree<'_>,
    res_buf: &FileNameRes,
    def_map: &CrateDefMap,
    db: &RidxV5ItemDb<'_>,
    sidecar: Option<&ExprTySidecar>,
    local_defs: &mut LocalCstDefs,
    infer_ctx: &mut BodyInferCtx,
    fn_res: &mut FnInferResult,
    body_nodes: &mut Vec<NodeId>,
    dot_out: &mut HashMap<(u32, u32), Vec<ResolvedDotRef>>,
    max_samples: usize,
    stats: &mut TyParityStats,
) {
    dot_out.clear();
    let krate = CrateId(r_idx as u32);
    local_defs.index_file_with_item_db(st, Some(def_map), Some(db), fid);
    collect_active_body_nodes(st, db, krate, body_nodes);

    for &fn_node in body_nodes.iter() {
        infer_ctx.infer_fn_into(st, res_buf, db, local_defs, fn_node, fn_res);
    }

    for &fn_node in body_nodes.iter() {
        reset_alloc_counters();
        let t0 = Instant::now();
        infer_ctx.infer_fn_into(st, res_buf, db, local_defs, fn_node, fn_res);
        let ns = t0.elapsed().as_nanos();
        let (a, b) = snapshot_alloc_counters();
        stats.fns_inferred += 1;
        stats.warm_fn_ns += ns;
        stats.warm_fn_allocs += a;
        stats.warm_fn_bytes += b;

        for &dr in &fn_res.dot_refs {
            dot_out.entry((dr.start, dr.end)).or_default().push(dr);
        }

        if let Some(file_recs) = sidecar.and_then(|sc| sc.by_file.get(&fid)) {
            compare_fn_expr_tys(r_idx, cf, st, res_buf, db, local_defs, infer_ctx, fn_res, fn_node, file_recs, max_samples, stats);
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn match_dot_ref(
    r_idx: usize,
    reader: &IndexReader<Mmap>,
    db: &RidxV5ItemDb<'_>,
    cf: &CorpusFile,
    out: &FileNameRes,
    dot_map: &HashMap<(u32, u32), Vec<ResolvedDotRef>>,
    sb: u32,
    eb: u32,
    tok_span: (u32, u32),
    is_macro_arg: bool,
    r: &RefEntry,
    max_samples: usize,
    stats: &mut TyParityStats,
) -> bool {
    if is_macro_arg {
        stats.dot_mtt_total += 1;
    } else {
        stats.dot_cst_total += 1;
    }
    let cands = dot_map.get(&(sb, eb)).or_else(|| dot_map.get(&tok_span));
    let dkind = DefKind::from(r.kind);
    let mut matched = false;
    if let Some(list) = cands {
        for dr in list {
            let (def, local) = if dr.def.krate == CrateId::LOCAL_OVERLAY && (dr.def.index & 0xf000_0000) == 0x8000_0000 {
                (GlobalDefId::NONE, LocalItemId(dr.def.index & 0x0fff_ffff))
            } else {
                (dr.def, LocalItemId::NONE)
            };
            let target = ResTarget::Def { def, local, kind: dr.kind };
            if ref_targets_match(r_idx, reader, db, cf, out, target, r).is_matched()
                || sysroot_field_owner_matches(reader, db, dr, r)
                || sysroot_trait_impl_method_matches(r_idx, reader, db, dr, r)
            {
                matched = true;
                break;
            }
        }
    }
    if matched {
        if is_macro_arg {
            stats.dot_mtt_matched += 1;
        } else {
            stats.dot_cst_matched += 1;
        }
    } else if !is_macro_arg {
        let tag = if cands.is_none() { "DOT_MISSING" } else { "DOT_WRONG" };
        let bkey = format!("{tag}:{dkind:?}");
        *stats.dot_miss_buckets.entry(bkey.clone()).or_insert(0) += 1;
        if stats.dot_samples.len() < max_samples {
            let sp = r.span();
            let snip = cf.text.get(sb as usize..eb as usize).unwrap_or("?");
            let tp = reader.get_str(r.target_path_offset());
            let our_desc = cands
                .and_then(|l| l.first())
                .map(|dr| {
                    let p =
                        db.ws.readers().get(dr.def.krate.0 as usize).and_then(|rd| rd.definitions().get(dr.def.index as usize).map(|d| rd.get_str(d.path_offset()))).unwrap_or("?");
                    format!("{:?}({}:{}:{p})", dr.kind, dr.def.krate.0, dr.def.index)
                })
                .unwrap_or_else(|| "none".to_string());
            stats.dot_samples.push(format!("[{bkey}] {}:{}:{} `.{snip}` our={our_desc} -> `{tp}`", cf.rel_path, sp.start_line(), sp.start_col()));
        }
    }
    matched
}

fn sysroot_field_owner_matches(reader: &IndexReader<Mmap>, db: &RidxV5ItemDb<'_>, dr: &ResolvedDotRef, r: &RefEntry) -> bool {
    if dr.kind != ItemKind::Field || DefKind::from(r.kind) != DefKind::Field || !dr.def.is_some() {
        return false;
    }
    let or_idx = dr.def.krate.0 as usize;
    let Some(or_rdr) = db.ws.readers().get(or_idx) else {
        return false;
    };
    let Some(od) = or_rdr.definitions().get(dr.def.index as usize) else {
        return false;
    };
    if !matches!(DefKind::from(od.kind), DefKind::Struct | DefKind::Union | DefKind::Variant) {
        return false;
    }
    let drv_path = reader.get_str(r.target_path_offset());
    let Some((drv_owner, _)) = drv_path.rsplit_once("::") else {
        return false;
    };
    let our_path = or_rdr.get_str(od.path_offset());
    drv_owner == our_path || drv_owner.ends_with(&format!("::{our_path}"))
}

fn sysroot_trait_impl_method_matches(r_idx: usize, reader: &IndexReader<Mmap>, db: &RidxV5ItemDb<'_>, dr: &ResolvedDotRef, r: &RefEntry) -> bool {
    if dr.kind != ItemKind::AssocFn || DefKind::from(r.kind) != DefKind::AssocFn || !dr.def.is_some() || dr.def.is_overlay() {
        return false;
    }
    let or_idx = dr.def.krate.0 as usize;
    let Some(or_rdr) = db.ws.readers().get(or_idx) else {
        return false;
    };
    let Some(od) = or_rdr.definitions().get(dr.def.index as usize) else {
        return false;
    };
    let our_name = or_rdr.get_str(od.name_offset()).trim_start_matches("r#");
    let drv_path = reader.get_str(r.target_path_offset());
    let drv_leaf = drv_path.rsplit("::").next().unwrap_or(drv_path).trim_start_matches("r#");
    if our_name != drv_leaf {
        return false;
    }
    let Some(&our_tr_didx) = db.shards[or_idx].trait_parent_of_assoc.get(&dr.def.index) else {
        return false;
    };
    let our_tr = db.canonical_def(GlobalDefId::new(dr.def.krate, our_tr_didx));
    let Some(impl_pos) = drv_path.find("<impl ") else {
        return false;
    };
    let rest = &drv_path[impl_pos + 6..];
    let after_gen = if rest.starts_with('<') {
        let mut d = 0i32;
        let mut end = 0usize;
        for (i, b) in rest.bytes().enumerate() {
            if b == b'<' {
                d += 1;
            } else if b == b'>' {
                d -= 1;
                if d == 0 {
                    end = i + 1;
                    break;
                }
            }
        }
        rest[end..].trim_start()
    } else {
        rest
    };
    let Some(for_pos) = after_gen.find(" for ") else {
        return false;
    };
    let tr_base = after_gen[..for_pos].split('<').next().unwrap_or("").trim().trim_end_matches("::");
    if tr_base.is_empty() {
        return false;
    }
    let first = tr_base.split("::").next().unwrap_or(tr_base);
    let tr_r = db.resolve_extern_name_to_reader(r_idx, first).or_else(|| db.sysroot_by_name.get(first).copied()).unwrap_or(r_idx);
    let drv_tr = db.canonical_def(db.find_def_in_reader_by_path(tr_r, tr_base));
    drv_tr.is_some() && (drv_tr == our_tr || db.trait_supertraits.get(&drv_tr).is_some_and(|s| s.iter().any(|&x| db.canonical_def(x) == our_tr)))
}

#[allow(clippy::too_many_arguments)]
fn compare_fn_expr_tys(
    r_idx: usize,
    cf: &CorpusFile,
    st: &SyntaxTree<'_>,
    res_buf: &FileNameRes,
    db: &RidxV5ItemDb<'_>,
    local_defs: &LocalCstDefs,
    infer_ctx: &mut BodyInferCtx,
    fn_res: &FnInferResult,
    fn_node: NodeId,
    file_recs: &[ExprTyRec],
    max_samples: usize,
    stats: &mut TyParityStats,
) {
    let reader = &db.ws.readers()[r_idx];
    let va = reader.ty_arena();
    let fn_gid = local_defs.fn_def_for_node(fn_node);
    let mut cached_owner = u32::MAX - 1;
    let mut remap = ParamRemap::empty();

    for &(enode, our_raw_ty) in &fn_res.expr_tys {
        let ek = st.kind(enode);
        if ek == K::ParenExpr {
            continue;
        }
        let er = st.range(enode);
        let (sb, eb) = (er.start as u32, er.end as u32);
        let lo = file_recs.partition_point(|r| (r.start, r.end) < (sb, eb));
        if lo >= file_recs.len() || (file_recs[lo].start, file_recs[lo].end) != (sb, eb) {
            continue;
        }
        let hi = lo + file_recs[lo..].partition_point(|r| (r.start, r.end) == (sb, eb));
        let slice = &file_recs[lo..hi];
        let is_cst_desugared = matches!(ek, K::TryExpr | K::AwaitExpr | K::ForExpr | K::WhileExpr);
        let chosen = slice.iter().find(|r| ((r.adj_ty_flagged & DESUGARED_TY_FLAG) != 0) == is_cst_desugared).unwrap_or(&slice[0]);

        let raw_tid = TyId(chosen.raw_ty);
        if matches!(va.kind(raw_tid), TyKindView::Unknown) {
            continue;
        }
        let owner_didx = if chosen.owner_def_idx != u32::MAX {
            chosen.owner_def_idx
        } else if fn_gid.krate.0 as usize == r_idx {
            fn_gid.index
        } else {
            u32::MAX
        };
        if owner_didx != cached_owner {
            remap = ParamRemap::for_def(db, r_idx, owner_didx, true, &mut infer_ctx.table.arena);
            cached_owner = owner_didx;
        }
        let comp_raw = lower_v5_ty(db, r_idx, va, raw_tid, &remap, &mut infer_ctx.table.arena);
        let adj_tid = TyId(chosen.adj_ty_flagged & !DESUGARED_TY_FLAG);
        let comp_adj = lower_v5_ty(db, r_idx, va, adj_tid, &remap, &mut infer_ctx.table.arena);
        if comp_raw == InferTyId::UNKNOWN && comp_adj == InferTyId::UNKNOWN {
            continue;
        }

        stats.expr_compared += 1;
        let norm_our = normalize_ty(&mut infer_ctx.table, db, local_defs, st, res_buf, our_raw_ty);
        let mut ok = tys_eq(db, &infer_ctx.table.arena, norm_our, comp_raw, 0) || tys_eq(db, &infer_ctx.table.arena, norm_our, comp_adj, 0);
        if !ok && slice.len() > 1 {
            for alt in slice {
                let cr = lower_v5_ty(db, r_idx, va, TyId(alt.raw_ty), &remap, &mut infer_ctx.table.arena);
                let ca = lower_v5_ty(db, r_idx, va, TyId(alt.adj_ty_flagged & !DESUGARED_TY_FLAG), &remap, &mut infer_ctx.table.arena);
                if tys_eq(db, &infer_ctx.table.arena, norm_our, cr, 0) || tys_eq(db, &infer_ctx.table.arena, norm_our, ca, 0) {
                    ok = true;
                    break;
                }
            }
        }
        if ok {
            stats.expr_matched += 1;
            continue;
        }
        let has_unk = ty_contains_unknown(&infer_ctx.table.arena, norm_our, 0);
        if has_unk {
            stats.expr_our_unknown += 1;
        } else {
            stats.expr_wrong_ty += 1;
        }
        let tag = if has_unk { "EXPR_UNK" } else { "EXPR_DIFF" };
        let bkey = format!("{tag}:{ek:?}");
        *stats.expr_miss_buckets.entry(bkey.clone()).or_insert(0) += 1;
        if stats.expr_samples.len() < max_samples {
            let (ln, col) = cf.lines.to_line_col(&cf.text, sb);
            let snip: String = cf.text.get(sb as usize..eb as usize).unwrap_or("?").chars().take(48).collect();
            stats.expr_samples.push(format!(
                "[{bkey}] {}:{ln}:{col} `{}` -> our {:?} vs rustc {:?}",
                cf.rel_path,
                snip.replace('\n', " "),
                infer_ctx.table.arena.kind(norm_our),
                va.kind(raw_tid),
            ));
        }
    }
}
