/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Full `.ridx.exprty` denominator funnel classifier (`expr coverage = сравнено / всего`),
//! `tys_eq` strictness breakdown, and per-function warm latency percentile (`p50`/`p95`/`p99`/`max`) reporter.

use ridx_format::{TyId, TyKindView};
use ridx_syntax::name_res::FileNameRes;
use ridx_syntax::ty_infer::autoderef::{normalize_ty_with_owner, NormOwner};
use ridx_syntax::ty_infer::{BodyInferCtx, FnInferResult, InferTyId, LocalCstDefs};
use ridx_syntax::{NodeId, SyntaxKind as K, SyntaxTree};

use crate::compare::CorpusReport;
use crate::meta::CorpusFile;
use crate::ty_parity::TyParityStats;
use crate::ty_report::{ty_contains_unknown, tys_eq_mode, ExprTyRec, DESUGARED_TY_FLAG, EQ_LENIENT_ARRAY_LEN, EQ_LENIENT_DYN_OPAQUE, EQ_STRICT};
use crate::v5_db::RidxV5ItemDb;
use crate::v5_ty_lower::{lower_v5_ty, ParamRemap};

pub const CLAIM_NONE: u8 = 0;
pub const CLAIM_PAREN: u8 = 1;
pub const CLAIM_CLAIMED: u8 = 2;

/// Accounts for all `.ridx.exprty` records in a crate file that was not reached/compared in CST walk.
pub fn account_unreached_file_recs(recs: &[ExprTyRec], stats: &mut TyParityStats) {
    let mut lo = 0usize;
    while lo < recs.len() {
        let (sb, eb) = (recs[lo].start, recs[lo].end);
        let hi = lo + recs[lo..].partition_point(|r| (r.start, r.end) == (sb, eb));
        stats.exprty_dup_span += (hi - lo) - 1;
        stats.exprty_no_cst_node += 1;
        stats.exprty_no_cst_unreached += 1;
        lo = hi;
    }
}

/// Finalizes a reached file's `.ridx.exprty` span groups after all active `body_nodes` in `st` ran.
pub fn finalize_reached_file_funnel(st: &SyntaxTree<'_>, body_nodes: &[NodeId], recs: &[ExprTyRec], group_state: &[u8], stats: &mut TyParityStats) {
    if recs.is_empty() {
        return;
    }
    let mut macro_ranges: Vec<(u32, u32)> = Vec::new();
    for nid in (0..st.nodes.len() as u32).map(NodeId) {
        if matches!(st.kind(nid), K::MacroCall | K::Attr | K::MacroRules) {
            let r = st.range(nid);
            macro_ranges.push((r.start as u32, r.end as u32));
        }
    }
    let mut body_ranges: Vec<(u32, u32)> = Vec::with_capacity(body_nodes.len());
    for &bn in body_nodes {
        let r = st.range(bn);
        body_ranges.push((r.start as u32, r.end as u32));
    }

    let mut lo = 0usize;
    while lo < recs.len() {
        let (sb, eb) = (recs[lo].start, recs[lo].end);
        let hi = lo + recs[lo..].partition_point(|r| (r.start, r.end) == (sb, eb));
        stats.exprty_dup_span += (hi - lo) - 1;
        match group_state[lo] {
            CLAIM_CLAIMED => {}
            CLAIM_PAREN => {
                stats.exprty_paren += 1;
            }
            _ => {
                stats.exprty_no_cst_node += 1;
                let in_macro = macro_ranges.iter().any(|&(ms, me)| sb >= ms && eb <= me);
                if in_macro {
                    stats.exprty_no_cst_macro += 1;
                } else if !body_ranges.iter().any(|&(bs, be)| sb >= bs && eb <= be) {
                    stats.exprty_no_cst_outside_body += 1;
                } else {
                    stats.exprty_no_cst_span_diff += 1;
                }
            }
        }
        lo = hi;
    }
}

fn pct_ns(sorted: &[u64], num: usize, den: usize) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((sorted.len() - 1) * num) / den;
    (sorted[idx] as f64) / 1_000.0
}

pub fn print_funnel_strictness_and_latency(all_reports: &[CorpusReport]) {
    if all_reports.is_empty() {
        return;
    }

    println!("\n=== `.ridx.exprty` Full Denominator Funnel & `expr coverage` (`сравнено / всего`) ===");
    println!(
        "| Corpus | всего в `.exprty` | дубликаты span | без нашего узла (`unreach`/`macro`/`non-fn`/`span`) | `ParenExpr` | driver->`Unknown` | сравнено | **expr coverage (`сравнено / всего`)** |"
    );
    println!("|---|---:|---:|---:|---:|---:|---:|---:|");

    let (mut g_tot, mut g_dup, mut g_nocst, mut g_par, mut g_dunk, mut g_cmp) = (0usize, 0usize, 0usize, 0usize, 0usize, 0usize);
    let (mut g_unr, mut g_mac, mut g_out, mut g_spd) = (0usize, 0usize, 0usize, 0usize);

    for r in all_reports {
        let t = &r.ty_stats;
        assert_eq!(
            t.exprty_total,
            t.exprty_dup_span + t.exprty_no_cst_node + t.exprty_paren + t.exprty_driver_unknown + t.expr_compared,
            "`.exprty` funnel leak in corpus `{}`",
            r.corpus
        );
        assert_eq!(
            t.exprty_no_cst_node,
            t.exprty_no_cst_unreached + t.exprty_no_cst_macro + t.exprty_no_cst_outside_body + t.exprty_no_cst_span_diff,
            "`.exprty` no-cst breakdown mismatch in corpus `{}`",
            r.corpus
        );
        let cov = if t.exprty_total == 0 { 0.0 } else { (t.expr_compared as f64) * 100.0 / (t.exprty_total as f64) };
        println!(
            "| `{}` | {} | {} | {} ({}/{}/{}/{}) | {} | {} | {} | **{} / {} ({:.2}%)** |",
            r.corpus,
            t.exprty_total,
            t.exprty_dup_span,
            t.exprty_no_cst_node,
            t.exprty_no_cst_unreached,
            t.exprty_no_cst_macro,
            t.exprty_no_cst_outside_body,
            t.exprty_no_cst_span_diff,
            t.exprty_paren,
            t.exprty_driver_unknown,
            t.expr_compared,
            t.expr_compared,
            t.exprty_total,
            cov,
        );
        g_tot += t.exprty_total;
        g_dup += t.exprty_dup_span;
        g_nocst += t.exprty_no_cst_node;
        g_unr += t.exprty_no_cst_unreached;
        g_mac += t.exprty_no_cst_macro;
        g_out += t.exprty_no_cst_outside_body;
        g_spd += t.exprty_no_cst_span_diff;
        g_par += t.exprty_paren;
        g_dunk += t.exprty_driver_unknown;
        g_cmp += t.expr_compared;
    }
    let g_cov = if g_tot == 0 { 0.0 } else { (g_cmp as f64) * 100.0 / (g_tot as f64) };
    println!(
        "| **TOTAL** | **{}** | **{}** | **{} ({}/{}/{}/{})** | **{}** | **{}** | **{}** | **{} / {} ({:.2}%)** |",
        g_tot, g_dup, g_nocst, g_unr, g_mac, g_out, g_spd, g_par, g_dunk, g_cmp, g_cmp, g_tot, g_cov
    );
    println!("expr coverage = {} / {} ({:.2}%)", g_cmp, g_tot, g_cov);

    println!("\n=== `tys_eq` Strictness Breakdown on `expr_matched` ===");
    println!("| Corpus | `expr_matched` | `strict_matched` | Strict % of Compared | `lenient_dyn_opaque` (only) | `lenient_array_len` (only) |");
    println!("|---|---:|---:|---:|---:|---:|");
    let (mut s_mat, mut s_str, mut s_dyn, mut s_arr, mut s_cmp) = (0usize, 0usize, 0usize, 0usize, 0usize);
    for r in all_reports {
        let t = &r.ty_stats;
        let spct = if t.expr_compared == 0 { 0.0 } else { (t.expr_strict_matched as f64) * 100.0 / (t.expr_compared as f64) };
        println!("| `{}` | {} | {} | **{:.2}%** | {} | {} |", r.corpus, t.expr_matched, t.expr_strict_matched, spct, t.expr_lenient_dyn_opaque, t.expr_lenient_array_len);
        s_mat += t.expr_matched;
        s_str += t.expr_strict_matched;
        s_dyn += t.expr_lenient_dyn_opaque;
        s_arr += t.expr_lenient_array_len;
        s_cmp += t.expr_compared;
    }
    let g_spct = if s_cmp == 0 { 0.0 } else { (s_str as f64) * 100.0 / (s_cmp as f64) };
    println!("| **TOTAL** | **{}** | **{}** | **{:.2}%** | **{}** | **{}** |", s_mat, s_str, g_spct, s_dyn, s_arr);

    println!("\n=== Warm Per-Function Latency Percentiles (`µs/fn`) & Tail Profile ===");
    println!("| Corpus | Bodies | Mean (µs) | p50 (µs) | p95 (µs) | p99 (µs) | Max (µs) | Mean Exprs/fn | >=p99 Exprs/fn | >=p99 Dots/fn |");
    println!("|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|");
    let mut all_prof: Vec<(u64, u32, u32)> = Vec::new();
    let mut tot_ns: u128 = 0;
    for r in all_reports {
        let t = &r.ty_stats;
        let mut prof = t.fn_profiles.clone();
        prof.sort_unstable_by_key(|p| p.0);
        let lat: Vec<u64> = prof.iter().map(|p| p.0).collect();
        let n = prof.len().max(1) as f64;
        let mean_us = (t.warm_fn_ns as f64) / n / 1_000.0;
        let mean_ex = prof.iter().map(|p| u64::from(p.1)).sum::<u64>() as f64 / n;
        let p99_i = ((prof.len().saturating_sub(1)) * 99) / 100;
        let tail = &prof[p99_i..];
        let tn = tail.len().max(1) as f64;
        let tail_ex = tail.iter().map(|p| u64::from(p.1)).sum::<u64>() as f64 / tn;
        let tail_dt = tail.iter().map(|p| u64::from(p.2)).sum::<u64>() as f64 / tn;
        println!(
            "| `{}` | {} | {:.1} | {:.1} | {:.1} | {:.1} | {:.1} | {:.1} | {:.1} | {:.1} |",
            r.corpus,
            prof.len(),
            mean_us,
            pct_ns(&lat, 50, 100),
            pct_ns(&lat, 95, 100),
            pct_ns(&lat, 99, 100),
            pct_ns(&lat, 100, 100),
            mean_ex,
            tail_ex,
            tail_dt,
        );
        tot_ns += t.warm_fn_ns;
        all_prof.extend_from_slice(&prof);
    }
    all_prof.sort_unstable_by_key(|p| p.0);
    let all_lat: Vec<u64> = all_prof.iter().map(|p| p.0).collect();
    let gn = all_prof.len().max(1) as f64;
    let g_mean_us = (tot_ns as f64) / gn / 1_000.0;
    let g_mean_ex = all_prof.iter().map(|p| u64::from(p.1)).sum::<u64>() as f64 / gn;
    let gp99_i = ((all_prof.len().saturating_sub(1)) * 99) / 100;
    let gtail = &all_prof[gp99_i..];
    let gtn = gtail.len().max(1) as f64;
    let gtail_ex = gtail.iter().map(|p| u64::from(p.1)).sum::<u64>() as f64 / gtn;
    let gtail_dt = gtail.iter().map(|p| u64::from(p.2)).sum::<u64>() as f64 / gtn;
    println!(
        "| **TOTAL** | **{}** | **{:.1}** | **{:.1}** | **{:.1}** | **{:.1}** | **{:.1}** | **{:.1}** | **{:.1}** | **{:.1}** |",
        all_prof.len(),
        g_mean_us,
        pct_ns(&all_lat, 50, 100),
        pct_ns(&all_lat, 95, 100),
        pct_ns(&all_lat, 99, 100),
        pct_ns(&all_lat, 100, 100),
        g_mean_ex,
        gtail_ex,
        gtail_dt,
    );
}

#[inline]
fn best_eq_mode(a: Option<u8>, b: Option<u8>) -> Option<u8> {
    a.zip(b).map(|(x, y)| x.min(y)).or(a).or(b)
}

#[allow(clippy::too_many_arguments)]
pub fn compare_fn_expr_tys(
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
    group_state: &mut [u8],
    max_samples: usize,
    stats: &mut TyParityStats,
) {
    let reader = &db.ws.readers()[r_idx];
    let va = reader.ty_arena();
    let fn_gid = local_defs.fn_def_for_node(fn_node);
    let norm_owner = NormOwner { fn_def: fn_gid, impl_node: None, impl_gc: 0, fn_node: Some(fn_node), self_ty: None };
    let mut cached_owner = u32::MAX - 1;
    let mut remap = ParamRemap::empty();

    let cst_iter = fn_res.expr_tys.iter().map(|&(en, ty)| {
        let er = st.range(en);
        (er.start as u32, er.end as u32, st.kind(en), ty)
    });
    let mac_iter = fn_res.macro_expr_tys.iter().map(|m| (m.start, m.end, m.kind, m.ty));

    for (sb, eb, ek, our_raw_ty) in cst_iter.chain(mac_iter) {
        let lo = file_recs.partition_point(|r| (r.start, r.end) < (sb, eb));
        if lo >= file_recs.len() || (file_recs[lo].start, file_recs[lo].end) != (sb, eb) {
            continue;
        }
        if ek == K::ParenExpr {
            if group_state[lo] == CLAIM_NONE {
                group_state[lo] = CLAIM_PAREN;
            }
            continue;
        }
        if group_state[lo] == CLAIM_CLAIMED {
            continue;
        }
        group_state[lo] = CLAIM_CLAIMED;

        let hi = lo + file_recs[lo..].partition_point(|r| (r.start, r.end) == (sb, eb));
        let slice = &file_recs[lo..hi];
        let is_cst_desugared = matches!(ek, K::TryExpr | K::AwaitExpr | K::ForExpr | K::WhileExpr);
        let chosen = slice.iter().find(|r| ((r.adj_ty_flagged & DESUGARED_TY_FLAG) != 0) == is_cst_desugared).unwrap_or(&slice[0]);

        let raw_tid = TyId(chosen.raw_ty);
        if matches!(va.kind(raw_tid), TyKindView::Unknown) {
            stats.exprty_driver_unknown += 1;
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
        let comp_adj = lower_v5_ty(db, r_idx, va, TyId(chosen.adj_ty_flagged & !DESUGARED_TY_FLAG), &remap, &mut infer_ctx.table.arena);
        if comp_raw == InferTyId::UNKNOWN && comp_adj == InferTyId::UNKNOWN {
            stats.exprty_driver_unknown += 1;
            continue;
        }

        stats.expr_compared += 1;
        let norm_our = normalize_ty_with_owner(&mut infer_ctx.table, db, local_defs, st, res_buf, norm_owner, our_raw_ty);
        let mut best = best_eq_mode(tys_eq_mode(db, &infer_ctx.table.arena, norm_our, comp_raw, 0), tys_eq_mode(db, &infer_ctx.table.arena, norm_our, comp_adj, 0));
        if best != Some(EQ_STRICT) && slice.len() > 1 {
            for alt in slice {
                let cr = lower_v5_ty(db, r_idx, va, TyId(alt.raw_ty), &remap, &mut infer_ctx.table.arena);
                let ca = lower_v5_ty(db, r_idx, va, TyId(alt.adj_ty_flagged & !DESUGARED_TY_FLAG), &remap, &mut infer_ctx.table.arena);
                best = best_eq_mode(best, best_eq_mode(tys_eq_mode(db, &infer_ctx.table.arena, norm_our, cr, 0), tys_eq_mode(db, &infer_ctx.table.arena, norm_our, ca, 0)));
                if best == Some(EQ_STRICT) {
                    break;
                }
            }
        }
        if best != Some(EQ_STRICT) && ek == K::MacroCall {
            for m in fn_res.macro_expr_tys.iter().filter(|m| m.start == sb && m.end == eb) {
                let nm = normalize_ty_with_owner(&mut infer_ctx.table, db, local_defs, st, res_buf, norm_owner, m.ty);
                best = best_eq_mode(best, best_eq_mode(tys_eq_mode(db, &infer_ctx.table.arena, nm, comp_raw, 0), tys_eq_mode(db, &infer_ctx.table.arena, nm, comp_adj, 0)));
                if best == Some(EQ_STRICT) {
                    break;
                }
            }
        }
        if let Some(mode) = best {
            stats.expr_matched += 1;
            stats.expr_strict_matched += usize::from(mode == EQ_STRICT);
            stats.expr_lenient_dyn_opaque += usize::from((mode & EQ_LENIENT_DYN_OPAQUE) != 0);
            stats.expr_lenient_array_len += usize::from((mode & EQ_LENIENT_ARRAY_LEN) != 0);
            continue;
        }
        let has_unk = ty_contains_unknown(&infer_ctx.table.arena, norm_our, 0);
        if has_unk {
            stats.expr_our_unknown += 1;
        } else {
            stats.expr_wrong_ty += 1;
        }
        let bkey = format!("{}:{ek:?}", if has_unk { "EXPR_UNK" } else { "EXPR_DIFF" });
        let b_cnt = stats.expr_miss_buckets.entry(bkey.clone()).or_insert(0);
        *b_cnt += 1;
        let b_limit = if max_samples <= 8 { 3 } else { max_samples };
        if *b_cnt <= b_limit && stats.expr_samples.len() < max_samples.max(40) {
            let (ln, col) = cf.lines.to_line_col(&cf.text, sb);
            let snip: String = cf.text.get(sb as usize..eb as usize).unwrap_or("?").chars().take(64).collect();
            stats.expr_samples.push(format!(
                "[{bkey}] {}:{ln}:{col} `{}` -> our `{}` vs rustc `{}` (`{}`)",
                cf.rel_path,
                snip.replace('\n', " "),
                infer_ctx.table.arena.format_ty(norm_our),
                infer_ctx.table.arena.format_ty(comp_raw),
                va.display_ty(raw_tid),
            ));
        }
    }
}
