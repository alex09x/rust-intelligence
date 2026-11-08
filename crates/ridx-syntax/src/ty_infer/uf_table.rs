/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Tarjan union-find table over [`TyVarId`] with union-by-rank, snapshot-safe path compression,
//! occurs-check, shallow chasing, and deep structural resolution with Rust literal fallbacks.

use super::arena::InferTyArena;
use super::ty::{InferTyId, InferTyKind, TySliceRange, TyVarId, TyVarKind};

#[derive(Clone, Copy, Debug)]
pub(super) enum VarSlot {
    /// Root when `parent == own_id`; redirected child when `parent != own_id`.
    Unbound { parent: TyVarId, rank: u8, kind: TyVarKind, self_ty: InferTyId },
    /// Root bound to a concrete or composite `InferTyId`.
    Bound { ty: InferTyId },
}

#[derive(Clone, Copy, Debug)]
struct UndoEntry {
    var: TyVarId,
    prev: VarSlot,
}

/// Token representing a speculative unification checkpoint (`start_snapshot` -> `commit` / `rollback_to`).
#[derive(Debug)]
pub struct InferSnapshot {
    var_len: usize,
    undo_len: usize,
}

/// Flat union-find inference variable table reusable across functions via [`Self::clear`].
#[derive(Debug, Default)]
pub struct UnionFindTable {
    slots: Vec<VarSlot>,
    undo_log: Vec<UndoEntry>,
    active_snapshots: u32,
}

impl UnionFindTable {
    pub fn with_capacity(cap: usize) -> Self {
        Self { slots: Vec::with_capacity(cap), undo_log: Vec::with_capacity(cap / 2), active_snapshots: 0 }
    }

    #[inline]
    pub fn clear(&mut self) {
        self.slots.clear();
        self.undo_log.clear();
        self.active_snapshots = 0;
    }

    #[inline]
    pub fn var_count(&self) -> usize {
        self.slots.len()
    }

    /// Allocates a fresh inference variable and its [`InferTyId`] wrapper inside `arena`.
    #[inline]
    pub fn new_var(&mut self, arena: &mut InferTyArena, kind: TyVarKind) -> (TyVarId, InferTyId) {
        let vid = TyVarId(self.slots.len() as u32);
        let ty = arena.mk_infer(vid);
        self.slots.push(VarSlot::Unbound { parent: vid, rank: 0, kind, self_ty: ty });
        (vid, ty)
    }

    #[inline]
    pub fn start_snapshot(&mut self) -> InferSnapshot {
        self.active_snapshots += 1;
        InferSnapshot { var_len: self.slots.len(), undo_len: self.undo_log.len() }
    }

    #[inline]
    pub fn commit(&mut self, snap: InferSnapshot) {
        debug_assert!(self.active_snapshots > 0);
        self.active_snapshots -= 1;
        if self.active_snapshots == 0 {
            self.undo_log.clear();
        } else {
            let _ = snap;
        }
    }

    pub fn rollback_to(&mut self, snap: InferSnapshot) {
        debug_assert!(self.active_snapshots > 0);
        while self.undo_log.len() > snap.undo_len {
            let entry = self.undo_log.pop().unwrap();
            if (entry.var.0 as usize) < snap.var_len {
                self.slots[entry.var.0 as usize] = entry.prev;
            }
        }
        self.slots.truncate(snap.var_len);
        self.active_snapshots -= 1;
        if self.active_snapshots == 0 {
            self.undo_log.clear();
        }
    }

    #[inline]
    fn set_slot(&mut self, var: TyVarId, next: VarSlot) {
        let idx = var.0 as usize;
        if self.active_snapshots > 0 {
            self.undo_log.push(UndoEntry { var, prev: self.slots[idx] });
        }
        self.slots[idx] = next;
    }

    /// Finds the current representative of `var` with snapshot-aware path halving/compression.
    pub fn probe_var(&mut self, mut var: TyVarId) -> Result<InferTyId, (TyVarId, TyVarKind, u8, InferTyId)> {
        let start = var;
        let root_res = loop {
            match self.slots[var.0 as usize] {
                VarSlot::Bound { ty } => break Ok(ty),
                VarSlot::Unbound { parent, rank, kind, self_ty } => {
                    if parent == var {
                        break Err((var, kind, rank, self_ty));
                    }
                    var = parent;
                }
            }
        };
        let target_parent = match root_res {
            Ok(_) => var,
            Err((root, ..)) => root,
        };
        let mut cur = start;
        while let VarSlot::Unbound { parent, rank, kind, self_ty } = self.slots[cur.0 as usize] {
            if parent == cur {
                break;
            }
            if parent != target_parent {
                self.set_slot(cur, VarSlot::Unbound { parent: target_parent, rank, kind, self_ty });
            }
            cur = parent;
        }
        root_res
    }

    /// Immutable read without path compression (safe for diagnostics and assertions).
    pub fn probe_var_imm(&self, mut var: TyVarId) -> Result<InferTyId, (TyVarId, TyVarKind)> {
        loop {
            match self.slots[var.0 as usize] {
                VarSlot::Bound { ty } => return Ok(ty),
                VarSlot::Unbound { parent, kind, .. } => {
                    if parent == var {
                        return Err((var, kind));
                    }
                    var = parent;
                }
            }
        }
    }

    /// Peels chained `Infer(v)` indirections until reaching a concrete type or an unbound root `Infer(root)`.
    #[inline]
    pub fn shallow_resolve(&mut self, arena: &InferTyArena, mut ty: InferTyId) -> InferTyId {
        while let InferTyKind::Infer(vid) = arena.kind(ty) {
            match self.probe_var(vid) {
                Ok(bound) => ty = bound,
                Err((_root, _kind, _rank, root_ty)) => return root_ty,
            }
        }
        ty
    }

    #[inline]
    pub(super) fn bind_root(&mut self, root: TyVarId, ty: InferTyId) {
        self.set_slot(root, VarSlot::Bound { ty });
    }

    pub(super) fn union_roots(&mut self, a: (TyVarId, u8, InferTyId), b: (TyVarId, u8, InferTyId), merged_kind: TyVarKind) {
        let (ra, rank_a, ty_a) = a;
        let (rb, rank_b, ty_b) = b;
        if ra == rb {
            self.set_slot(ra, VarSlot::Unbound { parent: ra, rank: rank_a, kind: merged_kind, self_ty: ty_a });
            return;
        }
        if rank_a < rank_b {
            self.set_slot(ra, VarSlot::Unbound { parent: rb, rank: rank_a, kind: merged_kind, self_ty: ty_a });
            self.set_slot(rb, VarSlot::Unbound { parent: rb, rank: rank_b, kind: merged_kind, self_ty: ty_b });
        } else if rank_a > rank_b {
            self.set_slot(rb, VarSlot::Unbound { parent: ra, rank: rank_b, kind: merged_kind, self_ty: ty_b });
            self.set_slot(ra, VarSlot::Unbound { parent: ra, rank: rank_a, kind: merged_kind, self_ty: ty_a });
        } else {
            self.set_slot(rb, VarSlot::Unbound { parent: ra, rank: rank_b, kind: merged_kind, self_ty: ty_b });
            self.set_slot(ra, VarSlot::Unbound { parent: ra, rank: rank_a.saturating_add(1), kind: merged_kind, self_ty: ty_a });
        }
    }

    /// Checks whether `needle_root` appears anywhere inside `ty` (`O(1)` fast path when `!has_infer()`).
    pub fn occurs_in(&mut self, arena: &InferTyArena, needle_root: TyVarId, ty: InferTyId) -> bool {
        self.occurs_rec(arena, needle_root, ty, 0)
    }

    fn occurs_rec(&mut self, arena: &InferTyArena, needle_root: TyVarId, ty: InferTyId, depth: u8) -> bool {
        if depth > 64 || !arena.flags(ty).has_infer() {
            return false;
        }
        let resolved = self.shallow_resolve(arena, ty);
        if !arena.flags(resolved).has_infer() {
            return false;
        }
        let check_slice = |this: &mut Self, r: TySliceRange| -> bool {
            for idx in 0..r.len as usize {
                let elem = arena.slice(r)[idx];
                if this.occurs_rec(arena, needle_root, elem, depth + 1) {
                    return true;
                }
            }
            false
        };
        match arena.kind(resolved) {
            InferTyKind::Infer(v) => v == needle_root,
            InferTyKind::Ref { inner, .. } | InferTyKind::Ptr { inner, .. } | InferTyKind::Slice(inner) | InferTyKind::Array { elem: inner, .. } => {
                self.occurs_rec(arena, needle_root, inner, depth + 1)
            }
            InferTyKind::Tuple(r)
            | InferTyKind::Adt { args: r, .. }
            | InferTyKind::FnPtr { sig: r }
            | InferTyKind::FnDef { args: r, .. }
            | InferTyKind::Closure { sig: r, .. }
            | InferTyKind::Dyn { args: r, .. }
            | InferTyKind::Opaque { args: r, .. } => check_slice(self, r),
            InferTyKind::Alias { self_ty, args, .. } => self.occurs_rec(arena, needle_root, self_ty, depth + 1) || check_slice(self, args),
            InferTyKind::Unknown | InferTyKind::Never | InferTyKind::Prim(_) | InferTyKind::Param { .. } => false,
        }
    }

    /// Recursively substitutes solved inference variables (`O(1)` when `!has_infer()`).
    /// When `fallback_unbound` is true, maps `{integer} -> i32`, `{float} -> f64`, `_ -> {unknown}`.
    pub fn deep_resolve(&mut self, arena: &mut InferTyArena, ty: InferTyId, fallback_unbound: bool) -> InferTyId {
        self.deep_resolve_rec(arena, ty, fallback_unbound, 0)
    }

    fn deep_resolve_rec(&mut self, arena: &mut InferTyArena, ty: InferTyId, fallback: bool, depth: u8) -> InferTyId {
        if depth > 64 {
            return InferTyId::UNKNOWN;
        }
        if !arena.flags(ty).has_infer() {
            return ty;
        }
        let resolved = self.shallow_resolve(arena, ty);
        if !arena.flags(resolved).has_infer() {
            return resolved;
        }
        match arena.kind(resolved) {
            InferTyKind::Infer(vid) => {
                if !fallback {
                    return resolved;
                }
                match self.probe_var(vid) {
                    Ok(b) => self.deep_resolve_rec(arena, b, true, depth + 1),
                    Err((_, TyVarKind::IntVar, ..)) => InferTyId::I32,
                    Err((_, TyVarKind::FloatVar, ..)) => InferTyId::F64,
                    Err((_, TyVarKind::General, ..)) => InferTyId::UNKNOWN,
                }
            }
            InferTyKind::Ref { is_mut, inner } => {
                let sub = self.deep_resolve_rec(arena, inner, fallback, depth + 1);
                arena.mk_ref(is_mut, sub)
            }
            InferTyKind::Ptr { is_mut, inner } => {
                let sub = self.deep_resolve_rec(arena, inner, fallback, depth + 1);
                arena.mk_ptr(is_mut, sub)
            }
            InferTyKind::Slice(elem) => {
                let sub = self.deep_resolve_rec(arena, elem, fallback, depth + 1);
                arena.mk_slice(sub)
            }
            InferTyKind::Array { elem, len } => {
                let sub = self.deep_resolve_rec(arena, elem, fallback, depth + 1);
                arena.mk_array(sub, len)
            }
            InferTyKind::Tuple(r) => {
                let sub = self.resolve_slice_buf(arena, r, fallback, depth + 1);
                arena.mk_tuple(sub.as_slice())
            }
            InferTyKind::Adt { def, args } => {
                let sub = self.resolve_slice_buf(arena, args, fallback, depth + 1);
                arena.mk_adt(def, sub.as_slice())
            }
            InferTyKind::FnPtr { sig } => {
                let sub = self.resolve_slice_buf(arena, sig, fallback, depth + 1);
                let s = sub.as_slice();
                arena.mk_fn_ptr(&s[..s.len() - 1], s[s.len() - 1])
            }
            InferTyKind::FnDef { def, args } => {
                let sub = self.resolve_slice_buf(arena, args, fallback, depth + 1);
                arena.mk_fn_def(def, sub.as_slice())
            }
            InferTyKind::Closure { id, sig } => {
                let sub = self.resolve_slice_buf(arena, sig, fallback, depth + 1);
                let s = sub.as_slice();
                arena.mk_closure(id, &s[..s.len() - 1], s[s.len() - 1])
            }
            InferTyKind::Alias { trait_def, assoc_def, self_ty, args } => {
                let ns = self.deep_resolve_rec(arena, self_ty, fallback, depth + 1);
                let sub = self.resolve_slice_buf(arena, args, fallback, depth + 1);
                arena.mk_alias(trait_def, assoc_def, ns, sub.as_slice())
            }
            InferTyKind::Dyn { principal, args } => {
                let sub = self.resolve_slice_buf(arena, args, fallback, depth + 1);
                arena.mk_dyn(principal, sub.as_slice())
            }
            InferTyKind::Opaque { def, args } => {
                let sub = self.resolve_slice_buf(arena, args, fallback, depth + 1);
                arena.mk_opaque(def, sub.as_slice())
            }
            InferTyKind::Unknown | InferTyKind::Never | InferTyKind::Prim(_) | InferTyKind::Param { .. } => resolved,
        }
    }

    fn resolve_slice_buf(&mut self, arena: &mut InferTyArena, r: TySliceRange, fallback: bool, depth: u8) -> InlineTyBuf {
        let mut buf = InlineTyBuf::new();
        for idx in 0..r.len as usize {
            let orig = arena.slice(r)[idx];
            let next = self.deep_resolve_rec(arena, orig, fallback, depth);
            buf.push(next);
        }
        buf
    }
}

pub(super) struct InlineTyBuf {
    inline: [InferTyId; 8],
    len: usize,
    spill: Vec<InferTyId>,
}

impl InlineTyBuf {
    #[inline]
    pub(super) fn new() -> Self {
        Self { inline: [InferTyId::UNKNOWN; 8], len: 0, spill: Vec::new() }
    }

    #[inline]
    pub(super) fn push(&mut self, ty: InferTyId) {
        if self.len < 8 {
            self.inline[self.len] = ty;
            self.len += 1;
        } else {
            if self.spill.is_empty() {
                self.spill.extend_from_slice(&self.inline);
            }
            self.spill.push(ty);
        }
    }

    #[inline]
    pub(super) fn as_slice(&self) -> &[InferTyId] {
        if self.spill.is_empty() {
            &self.inline[..self.len]
        } else {
            &self.spill
        }
    }
}
