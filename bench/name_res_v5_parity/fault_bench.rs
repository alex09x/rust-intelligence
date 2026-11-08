/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Phase `3.5` corpus-wide error-tolerance & partial `x.` receiver benchmark across all `88,835` bodies:
//! 1. `N` deterministic mutations per body (`N >= 10`: random token deletions + random-offset truncations),
//!    asserting `0` panics, `0` leaked `Infer(_)` vars, and comparing `p99` inference latency against clean `p99`.
//! 2. Truncating at every CST dot site `x.` where `.ridx.exprty` has a known `rustc` type for `x`, measuring
//!    the fraction of partial `x.` sites where `receiver_ty_at_dot` matches `rustc` (`target >= 95%`).

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::time::Instant;

use ridx_format::{TyId, TyKindView};
use ridx_syntax::name_res::{CrateId, CrateSnapshot, FileId, FileNameRes};
use ridx_syntax::ty_infer::autoderef::{normalize_ty_with_owner, NormOwner};
use ridx_syntax::ty_infer::{BodyInferCtx, FnInferResult, InferTyId, InferTyKind, LocalCstDefs};
use ridx_syntax::{parse, NodeId, SyntaxKind as K, SyntaxTree, TokenKind as T};

use crate::meta::CorpusFile;
use crate::ty_parity::collect_active_body_nodes;
use crate::ty_report::{ty_contains_unknown, ExprTyRec, ExprTySidecar, DESUGARED_TY_FLAG};
use crate::v5_db::RidxV5ItemDb;
use crate::v5_ty_lower::{lower_v5_ty, ParamRemap};

#[derive(Default)]
pub struct FaultBenchStats {
    pub bodies_tested: usize,
    pub mutations_run: usize,
    pub panics: usize,
    pub leaked_infer_vars: usize,
    pub mut_fn_ns: Vec<u64>,
    pub dot_sites_known: usize,
    pub dot_recv_matched: usize,
    pub dot_recv_posthoc_full: usize,
    pub dot_recv_strict: usize,
    pub dot_recv_drv_erased: usize,
    pub dot_recv_prefix_head: usize,
    pub dot_recv_our_unknown: usize,
    pub dot_recv_wrong_ty: usize,
    pub dot_recv_panics: usize,
    pub dot_samples: Vec<String>,
}

struct SplitMix64(u64);

impl SplitMix64 {
    fn new(seed: u64) -> Self {
        Self(seed ^ 0x9e37_79b9_7f4a_7c15)
    }

    #[inline]
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
}

#[allow(clippy::too_many_arguments)]
pub fn run_file_fault_and_partial_dot(
    r_idx: usize,
    fid: FileId,
    cf: &CorpusFile,
    st: &SyntaxTree<'_>,
    snap: &CrateSnapshot,
    db: &RidxV5ItemDb<'_>,
    sidecar: Option<&ExprTySidecar>,
    fault_n: usize,
    max_samples: usize,
    res_buf: &mut FileNameRes,
    local_defs: &mut LocalCstDefs,
    infer_ctx: &mut BodyInferCtx,
    fn_res: &mut FnInferResult,
    body_nodes: &mut Vec<NodeId>,
    stats: &mut FaultBenchStats,
) {
    let krate = CrateId(r_idx as u32);
    collect_active_body_nodes(st, db, krate, body_nodes);
    if body_nodes.is_empty() {
        return;
    }
    let file_recs = sidecar.and_then(|sc| sc.by_file.get(&fid.0).map(Vec::as_slice)).unwrap_or(&[]);
    let va_opt = (!file_recs.is_empty()).then(|| db.ws.readers()[r_idx].ty_arena());

    let mut masked_base = String::new();
    let mut mut_buf = String::new();
    let mut sig_toks: Vec<(u32, u32)> = Vec::new();
    let mut dot_sites: Vec<(u32, u32, u32, K)> = Vec::new();

    for idx in 0..body_nodes.len() {
        let body_node = body_nodes[idx];
        local_defs.index_target_fn_with_item_db(st, Some(&snap.clean_def_map), Some(db), fid, body_node);
        let fn_gid = local_defs.fn_def_for_node(body_node);
        build_masked_base_for_body(st, &cf.text, body_node, &mut masked_base);
        let fn_range = st.range(body_node);
        let body_sub = find_body_expr_child(st, body_node).unwrap_or(body_node);
        let body_range = st.range(body_sub);

        sig_toks.clear();
        collect_non_trivia_tok_spans(st, body_sub, &mut sig_toks);

        stats.bodies_tested += 1;
        let seed = ((r_idx as u64) << 40) ^ ((fid.0 as u64) << 20) ^ (body_node.0 as u64);
        let mut rng = SplitMix64::new(seed);

        for m in 0..fault_n {
            mut_buf.clear();
            let b_start = body_range.start.min(masked_base.len());
            let b_end = body_range.end.min(masked_base.len());
            if m % 2 == 0 && !sig_toks.is_empty() {
                let &(ts, te) = &sig_toks[(rng.next_u64() as usize) % sig_toks.len()];
                let (s, e) = ((ts as usize).min(masked_base.len()), (te as usize).min(masked_base.len()));
                mut_buf.push_str(&masked_base[..s]);
                mut_buf.push_str(&masked_base[e..]);
            } else {
                let span_len = b_end.saturating_sub(b_start);
                let mut cut = b_start + if span_len == 0 { 0 } else { (rng.next_u64() as usize) % (span_len + 1) };
                while cut > b_start && !masked_base.is_char_boundary(cut) {
                    cut -= 1;
                }
                mut_buf.push_str(&masked_base[..cut]);
                if m % 4 == 1 && b_end <= masked_base.len() {
                    mut_buf.push('}');
                    mut_buf.push_str(&masked_base[b_end..]);
                }
            }

            stats.mutations_run += 1;
            let run_res = catch_unwind(AssertUnwindSafe(|| {
                let mst = parse(&mut_buf);
                let Some(m_fn) = find_mutated_body_node(&mst, fn_range.start) else {
                    return (0u64, false);
                };
                snap.resolve_fn_paths_into(db, fid, &mst, m_fn, res_buf);
                local_defs.index_target_fn_with_item_db(&mst, Some(&snap.clean_def_map), Some(db), fid, m_fn);
                let t0 = Instant::now();
                infer_ctx.infer_fn_into(&mst, res_buf, db, local_defs, m_fn, fn_res);
                (t0.elapsed().as_nanos() as u64, has_leaked_infer_var(infer_ctx, fn_res))
            }));
            match run_res {
                Ok((ns, leaked)) => {
                    stats.mut_fn_ns.push(ns);
                    stats.leaked_infer_vars += usize::from(leaked);
                }
                Err(_) => stats.panics += 1,
            }
        }

        let Some(va) = va_opt.filter(|_| !file_recs.is_empty()) else {
            continue;
        };
        dot_sites.clear();
        collect_body_dot_sites(st, body_sub, &mut dot_sites);
        for &(rs, re, dot_end, rek) in &dot_sites {
            if (dot_end as usize) > masked_base.len() {
                continue;
            }
            let slice = lookup_compiler_slice(file_recs, rs, re);
            if slice.is_empty() {
                continue;
            }
            let is_desugared = matches!(rek, K::TryExpr | K::AwaitExpr | K::ForExpr | K::WhileExpr);
            let chosen = slice.iter().find(|r| ((r.adj_ty_flagged & DESUGARED_TY_FLAG) != 0) == is_desugared).unwrap_or(&slice[0]);
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
            let probe_rmap = ParamRemap::for_def(db, r_idx, owner_didx, true, &mut infer_ctx.table.arena);
            let pr = lower_v5_ty(db, r_idx, va, raw_tid, &probe_rmap, &mut infer_ctx.table.arena);
            let pa = lower_v5_ty(db, r_idx, va, TyId(chosen.adj_ty_flagged & !DESUGARED_TY_FLAG), &probe_rmap, &mut infer_ctx.table.arena);
            if pr == InferTyId::UNKNOWN && pa == InferTyId::UNKNOWN {
                continue;
            }

            stats.dot_sites_known += 1;
            let partial_src = &masked_base[..dot_end as usize];
            let dot_res = catch_unwind(AssertUnwindSafe(|| {
                let mst = parse(partial_src);
                let m_fn = find_mutated_body_node(&mst, fn_range.start)?;
                snap.resolve_fn_paths_into(db, fid, &mst, m_fn, res_buf);
                local_defs.index_target_fn_with_item_db(&mst, Some(&snap.clean_def_map), Some(db), fid, m_fn);
                infer_ctx.infer_fn_into(&mst, res_buf, db, local_defs, m_fn, fn_res);
                let raw = fn_res.receiver_ty_at_dot(&mst, dot_end)?;
                let n_own = NormOwner { fn_def: fn_gid, impl_node: None, impl_gc: 0, fn_node: Some(m_fn), self_ty: None };
                Some(normalize_ty_with_owner(&mut infer_ctx.table, db, local_defs, &mst, res_buf, n_own, raw))
            }));
            let our_ty = match dot_res {
                Ok(Some(t)) => t,
                Ok(None) => InferTyId::UNKNOWN,
                Err(_) => {
                    stats.dot_recv_panics += 1;
                    continue;
                }
            };
            let rmap = ParamRemap::for_def(db, r_idx, owner_didx, true, &mut infer_ctx.table.arena);
            let cr0 = lower_v5_ty(db, r_idx, va, raw_tid, &rmap, &mut infer_ctx.table.arena);
            let ca0 = lower_v5_ty(db, r_idx, va, TyId(chosen.adj_ty_flagged & !DESUGARED_TY_FLAG), &rmap, &mut infer_ctx.table.arena);
            let mut best = crate::fault_report::classify_recv_match(db, &infer_ctx.table.arena, our_ty, cr0).max(crate::fault_report::classify_recv_match(
                db,
                &infer_ctx.table.arena,
                our_ty,
                ca0,
            ));
            if best < Some(crate::fault_report::RecvMatch::StrictFull) && slice.len() > 1 {
                for alt in slice {
                    let cr = lower_v5_ty(db, r_idx, va, TyId(alt.raw_ty), &rmap, &mut infer_ctx.table.arena);
                    let ca = lower_v5_ty(db, r_idx, va, TyId(alt.adj_ty_flagged & !DESUGARED_TY_FLAG), &rmap, &mut infer_ctx.table.arena);
                    best = best.max(crate::fault_report::classify_recv_match(db, &infer_ctx.table.arena, our_ty, cr)).max(crate::fault_report::classify_recv_match(
                        db,
                        &infer_ctx.table.arena,
                        our_ty,
                        ca,
                    ));
                    if best == Some(crate::fault_report::RecvMatch::StrictFull) {
                        break;
                    }
                }
            }
            if let Some(rm) = best {
                stats.dot_recv_matched += 1;
                match rm {
                    crate::fault_report::RecvMatch::StrictFull => {
                        stats.dot_recv_posthoc_full += 1;
                        stats.dot_recv_strict += 1;
                    }
                    crate::fault_report::RecvMatch::LenientFull => stats.dot_recv_posthoc_full += 1,
                    crate::fault_report::RecvMatch::DriverErasedArg => stats.dot_recv_drv_erased += 1,
                    crate::fault_report::RecvMatch::PrefixHead => stats.dot_recv_prefix_head += 1,
                }
            } else {
                let has_unk = ty_contains_unknown(&infer_ctx.table.arena, our_ty, 0);
                stats.dot_recv_our_unknown += usize::from(has_unk);
                stats.dot_recv_wrong_ty += usize::from(!has_unk);
                if stats.dot_samples.len() < max_samples {
                    let (ln, col) = cf.lines.to_line_col(&cf.text, rs);
                    let snip: String = cf.text.get(rs as usize..dot_end as usize).unwrap_or("?").chars().take(48).collect();
                    stats.dot_samples.push(format!(
                        "[{}] {}:{ln}:{col} `{}` -> our `{}` vs rustc `{}`",
                        if has_unk { "DOT_UNK" } else { "DOT_DIFF" },
                        cf.rel_path,
                        snip.replace('\n', " "),
                        infer_ctx.table.arena.format_ty(our_ty),
                        infer_ctx.table.arena.format_ty(cr0)
                    ));
                }
            }
        }
    }
}

fn build_masked_base_for_body(st: &SyntaxTree<'_>, src: &str, body_node: NodeId, out: &mut String) {
    out.clear();
    if src.len() <= 8_192 {
        out.push_str(src);
        return;
    }
    let fn_r = st.range(body_node);
    let mut keep_end = (fn_r.end + 512).min(src.len());
    while keep_end < src.len() && !src.is_char_boundary(keep_end) {
        keep_end += 1;
    }
    out.push_str(&src[..keep_end]);
    let bytes = unsafe { out.as_bytes_mut() };
    mask_disjoint_siblings(st, st.root(), body_node, bytes);
    for anc in st.ancestors(body_node).filter(|&a| st.kind(a) == K::ItemList && st.range(a).end > keep_end) {
        let _ = anc;
        out.push_str("\n}");
    }
}

fn mask_disjoint_siblings(st: &SyntaxTree<'_>, container: NodeId, target: NodeId, bytes: &mut [u8]) {
    let (ts, te, t_byte) = (target.0, st.node(target).subtree_end, st.range(target).start);
    for ch in st.children(container) {
        if ch.0 < te && ts < st.node(ch).subtree_end {
            if let Some(il) = st.child(ch, K::ItemList) {
                mask_disjoint_siblings(st, il, target, bytes);
            }
            continue;
        }
        match st.kind(ch) {
            K::Fn => {
                if let Some(blk) = st.child(ch, K::BlockExpr) {
                    let br = st.range(blk);
                    if br.end <= t_byte && br.end <= bytes.len() && br.start + 1 < br.end.saturating_sub(1) {
                        bytes[br.start + 1..br.end - 1].fill(b' ');
                    }
                }
            }
            K::Impl | K::Trait | K::Module => {
                if let Some(il) = st.child(ch, K::ItemList) {
                    mask_disjoint_siblings(st, il, target, bytes);
                }
            }
            _ => {}
        }
    }
}

fn find_body_expr_child(st: &SyntaxTree<'_>, item: NodeId) -> Option<NodeId> {
    if st.kind(item) == K::Fn {
        st.child(item, K::BlockExpr)
    } else {
        st.children(item).find(|&c| st.kind(c).is_expr())
    }
}

fn find_mutated_body_node(st: &SyntaxTree<'_>, fn_start: usize) -> Option<NodeId> {
    let mut cur = st.root();
    loop {
        let next = st.children(cur).find(|&ch| {
            let r = st.range(ch);
            r.start <= fn_start && fn_start < r.end.max(r.start + 1)
        })?;
        if matches!(st.kind(next), K::Fn | K::Const | K::Static) && st.range(next).start == fn_start {
            return Some(next);
        }
        cur = next;
    }
}

fn collect_non_trivia_tok_spans(st: &SyntaxTree<'_>, root: NodeId, out: &mut Vec<(u32, u32)>) {
    let sub_end = st.node(root).subtree_end;
    let mut idx = root.0;
    while idx < sub_end {
        let n = NodeId(idx);
        if n != root && matches!(st.kind(n), K::Fn | K::Const | K::Static) {
            idx = st.node(n).subtree_end;
            continue;
        }
        for (_, tok) in st.own_tokens(n) {
            if !tok.is_trivia() {
                out.push((tok.start, tok.end));
            }
        }
        idx += 1;
    }
    out.sort_unstable_by_key(|&(s, _)| s);
}

fn collect_body_dot_sites(st: &SyntaxTree<'_>, root: NodeId, out: &mut Vec<(u32, u32, u32, K)>) {
    let sub_end = st.node(root).subtree_end;
    let mut idx = root.0;
    while idx < sub_end {
        let n = NodeId(idx);
        if n != root && matches!(st.kind(n), K::Fn | K::Const | K::Static) {
            idx = st.node(n).subtree_end;
            continue;
        }
        if matches!(st.kind(n), K::FieldExpr | K::MethodCallExpr | K::AwaitExpr) {
            if let Some(recv) = st.children(n).find(|&c| st.kind(c).is_expr() || st.kind(c) == K::MacroCall) {
                if st.kind(recv) != K::MacroCall {
                    if let Some((_, dtok)) = st.own_tokens(n).find(|(_, t)| t.kind == T::Dot) {
                        let rr = st.range(recv);
                        out.push((rr.start as u32, rr.end as u32, dtok.end, st.kind(recv)));
                    }
                }
            }
        }
        idx += 1;
    }
    out.sort_unstable_by_key(|&(_, _, de, _)| de);
    out.dedup_by_key(|x| x.2);
}

fn lookup_compiler_slice(file_recs: &[ExprTyRec], sb: u32, eb: u32) -> &[ExprTyRec] {
    let lo = file_recs.partition_point(|r| (r.start, r.end) < (sb, eb));
    let hi = lo + file_recs[lo..].partition_point(|r| (r.start, r.end) == (sb, eb));
    &file_recs[lo..hi]
}

fn has_leaked_infer_var(ctx: &BodyInferCtx, res: &FnInferResult) -> bool {
    let a = &ctx.table.arena;
    let _ = ParamRemap::empty;
    matches!(a.kind(res.ret_ty), InferTyKind::Infer(_))
        || matches!(a.kind(res.body_ty), InferTyKind::Infer(_))
        || res.expr_tys.iter().any(|&(_, t)| matches!(a.kind(t), InferTyKind::Infer(_)))
        || res.pat_tys.iter().any(|&(_, t)| matches!(a.kind(t), InferTyKind::Infer(_)))
        || res.local_tys.iter().any(|&(_, t)| matches!(a.kind(t), InferTyKind::Infer(_)))
        || res.incomplete_dots.iter().any(|&(_, t)| matches!(a.kind(t), InferTyKind::Infer(_)))
}
