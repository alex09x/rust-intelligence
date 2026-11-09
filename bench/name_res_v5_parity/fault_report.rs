/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Markdown table formatters and transparent 4-tier partial-`x.` receiver comparator for Phase `3.5`.

use ridx_syntax::ty_infer::{InferTyArena, InferTyId, InferTyKind};
use ridx_syntax::{NodeId, SyntaxKind as K, SyntaxTree};

use crate::compare::CorpusReport;
use crate::ty_report::{defs_same_item, ty_contains_unknown, tys_eq_mode, EQ_STRICT};
use crate::v5_db::RidxV5ItemDb;

/// Ordered from weakest (`PrefixHead`) to strongest (`StrictFull`) so `Option::max` picks the strictest match.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum RecvMatch {
    /// Receiver head ADT/Ref/Tuple matches `rustc`, known args match, and an unconstrained generic arg is `{unknown}`
    /// because the code after `x.` (which constrained `?T` in intact code) was truncated.
    PrefixHead,
    /// Our inferred type is 100% concrete (no `{unknown}`), while `.ridx.exprty` serialized a nested arg (`Global` / `{closure}`) as `Unknown`.
    DriverErasedArg,
    /// Full post-hoc equality under `tys_eq_mode` lenient `dyn`/`opaque` or array-len rules.
    LenientFull,
    /// Full post-hoc strict structural equality (`EQ_STRICT`).
    StrictFull,
}

pub fn classify_recv_match(db: &RidxV5ItemDb<'_>, arena: &InferTyArena, our: InferTyId, comp: InferTyId) -> Option<RecvMatch> {
    if let Some(m) = tys_eq_mode(db, arena, our, comp, 0) {
        return Some(if m == EQ_STRICT { RecvMatch::StrictFull } else { RecvMatch::LenientFull });
    }
    recv_compat_rec(db, arena, our, comp, 0, false)
}

fn recv_compat_rec(db: &RidxV5ItemDb<'_>, arena: &InferTyArena, our: InferTyId, comp: InferTyId, depth: u8, in_generic_arg: bool) -> Option<RecvMatch> {
    if depth > 24 || !our.is_some() || !comp.is_some() {
        return None;
    }
    if let Some(m) = tys_eq_mode(db, arena, our, comp, depth) {
        return Some(if m == EQ_STRICT { RecvMatch::StrictFull } else { RecvMatch::LenientFull });
    }
    if in_generic_arg {
        if comp == InferTyId::UNKNOWN && !ty_contains_unknown(arena, our, depth) {
            return Some(RecvMatch::DriverErasedArg);
        }
        if our == InferTyId::UNKNOWN || (matches!(arena.kind(our), InferTyKind::Alias { .. }) && ty_contains_unknown(arena, our, depth)) {
            return Some(RecvMatch::PrefixHead);
        }
    }
    match (arena.kind(our), arena.kind(comp)) {
        (InferTyKind::Unknown | InferTyKind::Infer(_), _) | (_, InferTyKind::Unknown | InferTyKind::Infer(_)) => None,
        (InferTyKind::Ref { is_mut: ma, inner: ia }, InferTyKind::Ref { is_mut: mb, inner: ib })
        | (InferTyKind::Ptr { is_mut: ma, inner: ia }, InferTyKind::Ptr { is_mut: mb, inner: ib }) => {
            (ma == mb).then(|| recv_compat_rec(db, arena, ia, ib, depth + 1, in_generic_arg)).flatten()
        }
        (InferTyKind::Slice(ea), InferTyKind::Slice(eb)) | (InferTyKind::Array { elem: ea, .. }, InferTyKind::Array { elem: eb, .. }) => {
            recv_compat_rec(db, arena, ea, eb, depth + 1, in_generic_arg)
        }
        (InferTyKind::Tuple(ra), InferTyKind::Tuple(rb)) => {
            let (sa, sb) = (arena.slice(ra), arena.slice(rb));
            if sa.len() != sb.len() || sa.is_empty() {
                return None;
            }
            let mut acc = RecvMatch::StrictFull;
            let mut any_concrete = false;
            for (&x, &y) in sa.iter().zip(sb.iter()) {
                any_concrete |= !matches!(arena.kind(x), InferTyKind::Unknown | InferTyKind::Infer(_));
                acc = acc.min(recv_compat_rec(db, arena, x, y, depth + 1, true)?);
            }
            (any_concrete || in_generic_arg).then_some(acc)
        }
        (InferTyKind::Adt { def: da, args: ra }, InferTyKind::Adt { def: d2, args: rb }) | (InferTyKind::FnDef { def: da, args: ra }, InferTyKind::FnDef { def: d2, args: rb }) => {
            if !defs_same_item(db, da, d2) {
                return None;
            }
            let (sa, sb) = (arena.slice(ra), arena.slice(rb));
            let mut acc = RecvMatch::StrictFull;
            for i in 0..sa.len().min(sb.len()) {
                if sa[i] == InferTyId::UNKNOWN && sb[i] == InferTyId::UNKNOWN {
                    continue;
                }
                acc = acc.min(recv_compat_rec(db, arena, sa[i], sb[i], depth + 1, true)?);
            }
            Some(acc)
        }
        _ => None,
    }
}

fn pct_us(sorted: &[u64], num: usize, den: usize) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    sorted[((sorted.len() - 1) * num) / den] as f64 / 1_000.0
}

pub fn build_masked_base_for_body(st: &SyntaxTree<'_>, src: &str, body_node: NodeId, out: &mut String) {
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

pub fn print_fault_bench_tables(reports: &[CorpusReport], fault_n: usize, verbose_samples: usize) {
    println!("\n=== Phase 3.5/3.6 Fault-Injection & Latency Gate (N={fault_n} mutations/body; TokDel vs Clean p99) ===");
    println!(
        "| Corpus | Bodies | Mutations | Panics | Leaked Infer | Clean p99 (µs) | TokDel Mean (µs) | TokDel p50 (µs) | TokDel p95 (µs) | **TokDel p99 (µs)** | TokDel/Clean | Trunc p99 (µs) | All Fault p99 (µs) | Max (µs) |"
    );
    println!("|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|");
    let (mut g_b, mut g_m, mut g_p, mut g_l) = (0usize, 0usize, 0usize, 0usize);
    let mut g_clean_ns: Vec<u64> = Vec::new();
    let mut g_tok_ns: Vec<u64> = Vec::new();
    let mut g_trunc_ns: Vec<u64> = Vec::new();
    let mut g_mut_ns: Vec<u64> = Vec::new();
    for r in reports {
        let fb = &r.fault_stats;
        g_b += fb.bodies_tested;
        g_m += fb.mutations_run;
        g_p += fb.panics;
        g_l += fb.leaked_infer_vars;
        let mut c_ns: Vec<u64> = r.ty_stats.fn_profiles.iter().map(|p| p.0).collect();
        c_ns.sort_unstable();
        g_clean_ns.extend_from_slice(&c_ns);
        let mut t_ns = fb.tok_del_fn_ns.clone();
        t_ns.sort_unstable();
        g_tok_ns.extend_from_slice(&t_ns);
        let mut tr_ns = fb.trunc_fn_ns.clone();
        tr_ns.sort_unstable();
        g_trunc_ns.extend_from_slice(&tr_ns);
        let mut m_ns = fb.mut_fn_ns.clone();
        m_ns.sort_unstable();
        g_mut_ns.extend_from_slice(&m_ns);
        let tok_mean = if t_ns.is_empty() { 0.0 } else { (t_ns.iter().sum::<u64>() as f64 / t_ns.len() as f64) / 1_000.0 };
        let cp99 = pct_us(&c_ns, 99, 100);
        let tp99 = pct_us(&t_ns, 99, 100);
        let ratio = if cp99 > 0.0 { tp99 / cp99 } else { 0.0 };
        println!(
            "| `{}` | {} | {} | {} | {} | {:.1} | {:.1} | {:.1} | {:.1} | **{:.1}** | {:.2}x | {:.1} | {:.1} | {:.1} |",
            r.corpus,
            fb.bodies_tested,
            fb.mutations_run,
            fb.panics,
            fb.leaked_infer_vars,
            cp99,
            tok_mean,
            pct_us(&t_ns, 50, 100),
            pct_us(&t_ns, 95, 100),
            tp99,
            ratio,
            pct_us(&tr_ns, 99, 100),
            pct_us(&m_ns, 99, 100),
            pct_us(&m_ns, 100, 100),
        );
    }
    g_clean_ns.sort_unstable();
    g_tok_ns.sort_unstable();
    g_trunc_ns.sort_unstable();
    g_mut_ns.sort_unstable();
    let g_tok_mean = if g_tok_ns.is_empty() { 0.0 } else { (g_tok_ns.iter().sum::<u64>() as f64 / g_tok_ns.len() as f64) / 1_000.0 };
    let g_cp99 = pct_us(&g_clean_ns, 99, 100);
    let g_tp99 = pct_us(&g_tok_ns, 99, 100);
    let g_ratio = if g_cp99 > 0.0 { g_tp99 / g_cp99 } else { 0.0 };
    println!(
        "| **TOTAL** | **{}** | **{}** | **{}** | **{}** | **{:.1}** | **{:.1}** | **{:.1}** | **{:.1}** | **{:.1}** | **{:.2}x** | **{:.1}** | **{:.1}** | **{:.1}** |",
        g_b,
        g_m,
        g_p,
        g_l,
        g_cp99,
        g_tok_mean,
        pct_us(&g_tok_ns, 50, 100),
        pct_us(&g_tok_ns, 95, 100),
        g_tp99,
        g_ratio,
        pct_us(&g_trunc_ns, 99, 100),
        pct_us(&g_mut_ns, 99, 100),
        pct_us(&g_mut_ns, 100, 100),
    );

    println!("\n=== Phase 3.5/3.6 Partial `x.` Receiver-Type Recovery (`Recv Match = PostHoc Full + Prefix EqMeth` ONLY) ===");
    println!("| Corpus | Known `x.` | Recv Match | **Recv %** | PostHoc Full (`Strict`) | Prefix EqMeth | Prefix DiffMeth | DrvErased (excl) | Unresolved | Wrong Ty | Panics |");
    println!("|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|");
    let (mut gk, mut gm, mut gf, mut gs, mut gpe, mut gpd, mut gd, mut gu, mut gw, mut gp) = (0usize, 0usize, 0usize, 0usize, 0usize, 0usize, 0usize, 0usize, 0usize, 0usize);
    for r in reports {
        let fb = &r.fault_stats;
        gk += fb.dot_sites_known;
        gm += fb.dot_recv_matched;
        gf += fb.dot_recv_posthoc_full;
        gs += fb.dot_recv_strict;
        gpe += fb.dot_recv_prefix_eq_methods;
        gpd += fb.dot_recv_prefix_diff_methods;
        gd += fb.dot_recv_drv_erased;
        gu += fb.dot_recv_our_unknown;
        gw += fb.dot_recv_wrong_ty;
        gp += fb.dot_recv_panics;
        let mp = if fb.dot_sites_known == 0 { 100.0 } else { 100.0 * fb.dot_recv_matched as f64 / fb.dot_sites_known as f64 };
        println!(
            "| `{}` | {} | {} | **{:.2}%** | {} ({}) | {} | {} | {} | {} | {} | {} |",
            r.corpus,
            fb.dot_sites_known,
            fb.dot_recv_matched,
            mp,
            fb.dot_recv_posthoc_full,
            fb.dot_recv_strict,
            fb.dot_recv_prefix_eq_methods,
            fb.dot_recv_prefix_diff_methods,
            fb.dot_recv_drv_erased,
            fb.dot_recv_our_unknown,
            fb.dot_recv_wrong_ty,
            fb.dot_recv_panics
        );
    }
    let g_mp = if gk == 0 { 100.0 } else { 100.0 * gm as f64 / gk as f64 };
    println!("| **TOTAL** | **{}** | **{}** | **{:.2}%** | **{} ({})** | **{}** | **{}** | **{}** | **{}** | **{}** | **{}** |", gk, gm, g_mp, gf, gs, gpe, gpd, gd, gu, gw, gp);
    if verbose_samples > 0 {
        for r in reports {
            for s in r.fault_stats.dot_samples.iter().take(verbose_samples) {
                println!("  dot-recv sample [{}]: {s}", r.corpus);
            }
        }
    }
}
