/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Function-body structural fingerprinting (`FnBodySummary`), compact per-function frozen type
//! tables (`FrozenTyArena`), and cached dirty-function outputs (`CachedDirtyFn`) for Checkbox 4.1.

use super::{
    arena::InferTyArena,
    body_ctx::{FnInferResult, MacroExprTy, ResolvedDotRef},
    ty::{InferTyId, InferTyKind, TyFlags, TySliceRange},
};
use crate::{
    syntax_kind::SyntaxKind as K,
    token::TokenKind as T,
    tree::{NodeId, SyntaxTree},
};

const FNV_OFFSET: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x00000100000001b3;

#[inline]
fn fnv_u8(mut h: u64, b: u8) -> u64 {
    h ^= b as u64;
    h.wrapping_mul(FNV_PRIME)
}

#[inline]
fn fnv_u16(h: u64, v: u16) -> u64 {
    let b = v.to_le_bytes();
    fnv_u8(fnv_u8(h, b[0]), b[1])
}

#[inline]
fn fnv_u32(mut h: u64, v: u32) -> u64 {
    for b in v.to_le_bytes() {
        h = fnv_u8(h, b);
    }
    h
}

#[inline]
fn fnv_bytes(mut h: u64, bytes: &[u8]) -> u64 {
    for &b in bytes {
        h = fnv_u8(h, b);
    }
    h
}

#[inline]
fn hash_tok_range(st: &SyntaxTree<'_>, mut h: u64, lo: u32, hi: u32, base_byte: Option<u32>) -> u64 {
    let end = (hi as usize).min(st.tokens.len());
    let start = (lo as usize).min(end);
    for tok in &st.tokens[start..end] {
        if !tok.is_trivia() {
            h = fnv_u16(h, tok.kind as u16);
            if let Some(base) = base_byte {
                h = fnv_u32(h, tok.start.saturating_sub(base));
            }
            h = fnv_bytes(h, &st.src.as_bytes()[tok.start as usize..tok.end as usize]);
        }
    }
    h
}

#[inline]
pub(crate) fn node_span_u32(st: &SyntaxTree<'_>, id: NodeId) -> (u32, u32) {
    let r = st.range(id);
    (r.start as u32, r.end as u32)
}

/// Compact read-only snapshot of an [`InferTyArena`] for a single dirty function (`0` hash-bucket overhead).
#[derive(Clone, Debug, Default)]
pub struct FrozenTyArena {
    kinds: Vec<InferTyKind>,
    flags: Vec<TyFlags>,
    ty_lists: Vec<InferTyId>,
}

impl FrozenTyArena {
    pub fn with_capacity(cap: usize) -> Self {
        Self { kinds: Vec::with_capacity(cap), flags: Vec::with_capacity(cap), ty_lists: Vec::with_capacity(cap) }
    }

    #[inline]
    pub fn clear(&mut self) {
        self.kinds.clear();
        self.flags.clear();
        self.ty_lists.clear();
    }

    /// Copies all interned type nodes and slices from `arena` without reallocating after warmup.
    #[inline]
    pub fn copy_from(&mut self, arena: &InferTyArena) {
        self.kinds.clear();
        self.kinds.extend_from_slice(&arena.kinds);
        self.flags.clear();
        self.flags.extend_from_slice(&arena.flags);
        self.ty_lists.clear();
        self.ty_lists.extend_from_slice(&arena.ty_lists);
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.kinds.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.kinds.is_empty()
    }

    #[inline]
    pub fn kind(&self, id: InferTyId) -> InferTyKind {
        self.kinds.get(id.0 as usize).copied().unwrap_or(InferTyKind::Unknown)
    }

    #[inline]
    pub fn flags(&self, id: InferTyId) -> TyFlags {
        self.flags.get(id.0 as usize).copied().unwrap_or(TyFlags::HAS_UNKNOWN)
    }

    #[inline]
    pub fn slice(&self, range: TySliceRange) -> &[InferTyId] {
        let s = range.start as usize;
        let e = s + range.len as usize;
        self.ty_lists.get(s..e).unwrap_or(&[])
    }

    #[inline]
    pub fn heap_bytes(&self) -> usize {
        self.kinds.capacity() * core::mem::size_of::<InferTyKind>()
            + self.flags.capacity() * core::mem::size_of::<TyFlags>()
            + self.ty_lists.capacity() * core::mem::size_of::<InferTyId>()
    }
}

/// Shift-invariant structural fingerprint and span metadata for a body-bearing item (`fn`, `const`, `static`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FnBodySummary {
    pub fn_key: u64,
    pub sig_hash: u64,
    pub body_hash: u64,
    pub fn_node: NodeId,
    pub body_node: NodeId,
    pub fn_start: u32,
    pub fn_end: u32,
    pub body_start: u32,
    pub body_end: u32,
}

/// Extracts all body-bearing [`FnBodySummary`] entries in `st` and returns the shift-invariant
/// `file_env_hash` covering all non-trivia tokens outside function bodies.
pub fn extract_file_fn_summaries(st: &SyntaxTree<'_>, out: &mut Vec<FnBodySummary>) -> u64 {
    out.clear();
    let total_nodes = st.nodes.len() as u32;
    let tok_count = st.tokens.len() as u32;
    let mut max_top_body_hi = 0u32;
    let mut env_h = FNV_OFFSET;

    for raw in 0..total_nodes {
        let nid = NodeId(raw);
        let kind = st.kind(nid);
        if kind != K::Fn {
            continue;
        }
        let Some(body_node) = st.child(nid, K::BlockExpr) else { continue };
        let fn_data = *st.node(nid);
        let body_data = *st.node(body_node);
        let (fn_start, fn_end) = node_span_u32(st, nid);
        let (body_start, body_end) = node_span_u32(st, body_node);

        let fn_key = compute_fn_key(st, nid, kind);
        let sig_h =
            hash_tok_range(st, hash_tok_range(st, fnv_u16(FNV_OFFSET, kind as u16), fn_data.first, body_data.first, Some(fn_start)), body_data.end, fn_data.end, Some(fn_start));
        let body_h = hash_fn_body_tokens(st, body_node, body_start);

        if body_data.first >= max_top_body_hi && body_data.end > body_data.first {
            if max_top_body_hi < body_data.first {
                env_h = hash_tok_range(st, env_h, max_top_body_hi, body_data.first, None);
            }
            max_top_body_hi = body_data.end;
        }

        out.push(FnBodySummary { fn_key, sig_hash: sig_h, body_hash: body_h, fn_node: nid, body_node, fn_start, fn_end, body_start, body_end });
    }

    if max_top_body_hi < tok_count {
        env_h = hash_tok_range(st, env_h, max_top_body_hi, tok_count, None);
    }
    env_h
}

fn hash_fn_body_tokens(st: &SyntaxTree<'_>, body_node: NodeId, body_start: u32) -> u64 {
    let body_data = *st.node(body_node);
    let mut h = FNV_OFFSET;
    let mut cur_tok = body_data.first;
    let mut raw = body_node.0 + 1;
    let end_raw = body_data.subtree_end;
    while raw < end_raw {
        let ch = NodeId(raw);
        if st.kind(ch) == K::Fn {
            if let Some(nested_body) = st.child(ch, K::BlockExpr) {
                let nb = *st.node(nested_body);
                if cur_tok < nb.first {
                    h = hash_tok_range(st, h, cur_tok, nb.first, Some(body_start));
                }
                cur_tok = cur_tok.max(nb.end);
                raw = nb.subtree_end;
                continue;
            }
        }
        raw += 1;
    }
    if cur_tok < body_data.end {
        h = hash_tok_range(st, h, cur_tok, body_data.end, Some(body_start));
    }
    h
}

fn compute_fn_key(st: &SyntaxTree<'_>, fn_node: NodeId, kind: K) -> u64 {
    let mut h = fnv_u16(FNV_OFFSET, kind as u16);
    for anc in st.ancestors(fn_node) {
        let ak = st.kind(anc);
        if matches!(ak, K::Impl | K::Trait | K::Module | K::Fn) {
            h = fnv_u16(h, ak as u16);
            if ak == K::Impl {
                let stop_lo = st.child(anc, K::ItemList).map(|c| st.node(c).first).unwrap_or(st.node(anc).end);
                h = hash_tok_range(st, h, st.node(anc).first, stop_lo, None);
            } else if let Some(nm) = st.child(anc, K::Name) {
                h = fnv_bytes(h, st.text(nm).as_bytes());
            } else if let Some((_, nt)) = st.own_tokens(anc).find(|(_, t)| t.kind == T::Ident) {
                h = fnv_bytes(h, &st.src.as_bytes()[nt.start as usize..nt.end as usize]);
            }
        }
    }
    if let Some(nm) = st.child(fn_node, K::Name) {
        h = fnv_bytes(h, st.text(nm).as_bytes());
    } else if let Some((_, nt)) = st.own_tokens(fn_node).find(|(_, t)| t.kind == T::Ident) {
        h = fnv_bytes(h, &st.src.as_bytes()[nt.start as usize..nt.end as usize]);
    }
    h
}

/// Cached local type inference output and frozen type table for a single dirty function body.
#[derive(Clone, Debug)]
pub struct CachedDirtyFn {
    pub fn_key: u64,
    pub sig_hash: u64,
    pub body_hash: u64,
    pub env_hash: u64,
    pub fn_node: NodeId,
    pub body_node: NodeId,
    pub fn_start: u32,
    pub fn_end: u32,
    pub body_start: u32,
    pub body_end: u32,
    pub has_full_db: bool,
    pub infer: FnInferResult,
    pub types: FrozenTyArena,
}

impl CachedDirtyFn {
    pub fn new() -> Self {
        Self {
            fn_key: 0,
            sig_hash: 0,
            body_hash: 0,
            env_hash: 0,
            fn_node: NodeId(0),
            body_node: NodeId(0),
            fn_start: 0,
            fn_end: 0,
            body_start: 0,
            body_end: 0,
            has_full_db: false,
            infer: FnInferResult::new(),
            types: FrozenTyArena::with_capacity(128),
        }
    }

    pub fn init_from_summary(&mut self, s: &FnBodySummary, env_hash: u64, has_full_db: bool) {
        self.fn_key = s.fn_key;
        self.sig_hash = s.sig_hash;
        self.body_hash = s.body_hash;
        self.env_hash = env_hash;
        self.fn_node = s.fn_node;
        self.body_node = s.body_node;
        self.fn_start = s.fn_start;
        self.fn_end = s.fn_end;
        self.body_start = s.body_start;
        self.body_end = s.body_end;
        self.has_full_db = has_full_db;
    }

    /// Shifts all stored `NodeId`s and byte spans in `O(N_fn)` when an edit occurs outside this function.
    pub fn shift_to(&mut self, summary: &FnBodySummary) {
        let delta_node = summary.fn_node.0 as i64 - self.fn_node.0 as i64;
        let delta_byte = summary.fn_start as i64 - self.fn_start as i64;
        if delta_node != 0 {
            self.infer.fn_node = NodeId((self.infer.fn_node.0 as i64 + delta_node) as u32);
            for (nid, _) in &mut self.infer.expr_tys {
                nid.0 = (nid.0 as i64 + delta_node) as u32;
            }
            for (nid, _) in &mut self.infer.pat_tys {
                nid.0 = (nid.0 as i64 + delta_node) as u32;
            }
        }
        if delta_byte != 0 {
            for m in &mut self.infer.macro_expr_tys {
                m.start = (m.start as i64 + delta_byte) as u32;
                m.end = (m.end as i64 + delta_byte) as u32;
            }
            for d in &mut self.infer.dot_refs {
                d.start = (d.start as i64 + delta_byte) as u32;
                d.end = (d.end as i64 + delta_byte) as u32;
            }
            for (off, _) in &mut self.infer.incomplete_dots {
                *off = (*off as i64 + delta_byte) as u32;
            }
        }
        self.fn_node = summary.fn_node;
        self.body_node = summary.body_node;
        self.fn_start = summary.fn_start;
        self.fn_end = summary.fn_end;
        self.body_start = summary.body_start;
        self.body_end = summary.body_end;
    }

    #[inline]
    pub fn expr_ty(&self, node: NodeId) -> Option<InferTyId> {
        let idx = self.infer.expr_tys.binary_search_by_key(&node, |&(n, _)| n).ok()?;
        Some(self.infer.expr_tys[idx].1)
    }

    #[inline]
    pub fn pat_ty(&self, node: NodeId) -> Option<InferTyId> {
        let idx = self.infer.pat_tys.binary_search_by_key(&node, |&(n, _)| n).ok()?;
        Some(self.infer.pat_tys[idx].1)
    }

    /// Finds the tightest inferred expression covering `byte_offset` in `st`.
    pub fn expr_ty_at_offset(&self, st: &SyntaxTree<'_>, byte_offset: u32) -> Option<(NodeId, InferTyId)> {
        tightest_ty_at_offset(st, &self.infer.expr_tys, byte_offset)
    }

    /// Finds the tightest inferred pattern covering `byte_offset` in `st`.
    pub fn pat_ty_at_offset(&self, st: &SyntaxTree<'_>, byte_offset: u32) -> Option<(NodeId, InferTyId)> {
        tightest_ty_at_offset(st, &self.infer.pat_tys, byte_offset)
    }

    #[inline]
    pub fn dot_ref_at_offset(&self, byte_offset: u32) -> Option<ResolvedDotRef> {
        self.infer.dot_refs.iter().copied().find(|d| d.start <= byte_offset && byte_offset <= d.end)
    }

    #[inline]
    pub fn incomplete_dot_at_offset(&self, byte_offset: u32) -> Option<(u32, InferTyId)> {
        self.infer.incomplete_dots.iter().copied().find(|&(off, _)| off == byte_offset || off.abs_diff(byte_offset) <= 1)
    }

    #[inline]
    pub fn heap_bytes(&self) -> usize {
        self.types.heap_bytes()
            + self.infer.expr_tys.capacity() * core::mem::size_of::<(NodeId, InferTyId)>()
            + self.infer.macro_expr_tys.capacity() * core::mem::size_of::<MacroExprTy>()
            + self.infer.pat_tys.capacity() * core::mem::size_of::<(NodeId, InferTyId)>()
            + self.infer.local_tys.capacity() * core::mem::size_of::<(crate::name_res::LocalBindId, InferTyId)>()
            + self.infer.dot_refs.capacity() * core::mem::size_of::<ResolvedDotRef>()
            + self.infer.incomplete_dots.capacity() * core::mem::size_of::<(u32, InferTyId)>()
    }
}

impl Default for CachedDirtyFn {
    fn default() -> Self {
        Self::new()
    }
}

fn tightest_ty_at_offset(st: &SyntaxTree<'_>, items: &[(NodeId, InferTyId)], byte_offset: u32) -> Option<(NodeId, InferTyId)> {
    let mut best: Option<(NodeId, InferTyId, u32)> = None;
    for &(nid, ty) in items {
        let (s, e) = node_span_u32(st, nid);
        if s <= byte_offset && byte_offset <= e && best.is_none_or(|(_, _, bl)| (e - s) <= bl) {
            best = Some((nid, ty, e - s));
        }
    }
    best.map(|(n, t, _)| (n, t))
}
