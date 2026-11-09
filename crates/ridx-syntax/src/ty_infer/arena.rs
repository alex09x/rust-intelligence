/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Intrusive hash-consed [`InferTyArena`] with pre-interned builtins and zero-allocation reuse.

use super::ty::{InferTyId, InferTyKind, TyFlags, TySliceRange, TyVarId};
use crate::name_res::{GlobalDefId, PrimTy};

/// High-32-bit tag on [`InferTyKind::Array::len`] encoding a generic `const N: usize` parameter index (`low 16 bits`).
pub const ARRAY_LEN_PARAM_TAG: u64 = 0xFFFF_FFFE_0000_0000;
/// High-32-bit tag on [`InferTyKind::Array::len`] encoding an inference variable `TyVarId` (`low 32 bits`).
pub const ARRAY_LEN_VAR_TAG: u64 = 0xFFFF_FFFD_0000_0000;

/// Intrusive hash-consed arena of [`InferTyKind`]s with zero per-type heap allocations.
#[derive(Debug)]
pub struct InferTyArena {
    kinds: Vec<InferTyKind>,
    flags: Vec<TyFlags>,
    ty_lists: Vec<InferTyId>,
    hashes: Vec<u64>,
    next: Vec<u32>,
    buckets: Vec<u32>,
    dynamic_count: usize,
}

impl Default for InferTyArena {
    fn default() -> Self {
        Self::with_capacity(128)
    }
}

impl InferTyArena {
    pub fn with_capacity(cap: usize) -> Self {
        let cap = cap.max(64).next_power_of_two();
        let mut arena = Self {
            kinds: Vec::with_capacity(cap),
            flags: Vec::with_capacity(cap),
            ty_lists: Vec::with_capacity(cap * 2),
            hashes: Vec::with_capacity(cap),
            next: Vec::with_capacity(cap),
            buckets: vec![u32::MAX; cap * 2],
            dynamic_count: 0,
        };
        arena.seed_builtins();
        arena
    }

    fn seed_builtins(&mut self) {
        self.push_raw(InferTyKind::Unknown, TyFlags::HAS_UNKNOWN, 0);
        self.push_raw(InferTyKind::Never, TyFlags::EMPTY, 0);
        self.push_raw(InferTyKind::Tuple(TySliceRange::EMPTY), TyFlags::EMPTY, 0);
        for p in PrimTy::ALL {
            self.push_raw(InferTyKind::Prim(p), TyFlags::EMPTY, 0);
        }
        self.push_raw(InferTyKind::Ref { is_mut: false, inner: InferTyId::STR }, TyFlags::EMPTY, 0);
        debug_assert_eq!(self.kinds.len(), InferTyId::BUILTIN_COUNT);
    }

    #[inline]
    fn push_raw(&mut self, kind: InferTyKind, fl: TyFlags, hash: u64) -> InferTyId {
        let id = InferTyId(self.kinds.len() as u32);
        self.kinds.push(kind);
        self.flags.push(fl);
        self.hashes.push(hash);
        self.next.push(u32::MAX);
        id
    }

    /// Resets dynamic entries while keeping all underlying `Vec` capacities (`0` allocations on next fn).
    pub fn clear(&mut self) {
        if self.dynamic_count > 0 {
            let dyn_len = self.hashes.len().saturating_sub(InferTyId::BUILTIN_COUNT);
            if dyn_len * 2 < self.buckets.len() {
                let mask = self.buckets.len() - 1;
                for &h in &self.hashes[InferTyId::BUILTIN_COUNT..] {
                    self.buckets[(h as usize) & mask] = u32::MAX;
                }
            } else {
                self.buckets.fill(u32::MAX);
            }
            self.dynamic_count = 0;
        }
        self.kinds.truncate(InferTyId::BUILTIN_COUNT);
        self.flags.truncate(InferTyId::BUILTIN_COUNT);
        self.hashes.truncate(InferTyId::BUILTIN_COUNT);
        self.next.truncate(InferTyId::BUILTIN_COUNT);
        self.ty_lists.clear();
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
        self.kinds[id.0 as usize]
    }

    #[inline]
    pub fn flags(&self, id: InferTyId) -> TyFlags {
        self.flags[id.0 as usize]
    }

    #[inline]
    pub fn slice(&self, range: TySliceRange) -> &[InferTyId] {
        let s = range.start as usize;
        &self.ty_lists[s..s + range.len as usize]
    }

    #[inline]
    pub fn intern_slice(&mut self, items: &[InferTyId]) -> TySliceRange {
        if items.is_empty() {
            return TySliceRange::EMPTY;
        }
        let start = self.ty_lists.len() as u32;
        self.ty_lists.extend_from_slice(items);
        TySliceRange { start, len: items.len() as u16 }
    }

    #[inline]
    pub fn mk_infer(&mut self, var: TyVarId) -> InferTyId {
        self.push_raw(InferTyKind::Infer(var), TyFlags::HAS_INFER, 0)
    }

    #[inline]
    pub fn mk_ref(&mut self, is_mut: bool, inner: InferTyId) -> InferTyId {
        if !is_mut && inner == InferTyId::STR {
            return InferTyId::STR_REF;
        }
        let fl = self.flags(inner);
        self.intern_scalar(1, ((is_mut as u64) << 32) | (inner.0 as u64), 0, InferTyKind::Ref { is_mut, inner }, fl)
    }

    #[inline]
    pub fn mk_ptr(&mut self, is_mut: bool, inner: InferTyId) -> InferTyId {
        let fl = self.flags(inner);
        self.intern_scalar(2, ((is_mut as u64) << 32) | (inner.0 as u64), 0, InferTyKind::Ptr { is_mut, inner }, fl)
    }

    #[inline]
    pub fn mk_slice(&mut self, elem: InferTyId) -> InferTyId {
        let fl = self.flags(elem);
        self.intern_scalar(3, elem.0 as u64, 0, InferTyKind::Slice(elem), fl)
    }

    #[inline]
    pub fn mk_array(&mut self, elem: InferTyId, len: u64) -> InferTyId {
        let mut fl = self.flags(elem);
        let hi = len & 0xFFFF_FFFF_0000_0000;
        if hi == ARRAY_LEN_PARAM_TAG {
            fl = fl.union(TyFlags::HAS_PARAM);
        } else if hi == ARRAY_LEN_VAR_TAG {
            fl = fl.union(TyFlags::HAS_INFER);
        }
        self.intern_scalar(4, elem.0 as u64, len, InferTyKind::Array { elem, len }, fl)
    }

    #[inline]
    pub fn resolve_array_len(&self, len: u64, subst: &[InferTyId]) -> u64 {
        if (len & 0xFFFF_FFFF_0000_0000) == ARRAY_LEN_PARAM_TAG {
            if let Some(&st) = subst.get((len & 0xFFFF) as usize) {
                match self.kind(st) {
                    InferTyKind::Array { elem, len: c_len } if elem == InferTyId::NEVER => return c_len,
                    InferTyKind::Infer(v) => return ARRAY_LEN_VAR_TAG | u64::from(v.0),
                    _ => {}
                }
            }
            return u64::MAX;
        }
        len
    }

    pub fn extract_array_const_param(&self, pat_ty: InferTyId, act_ty: InferTyId) -> Option<(usize, u64)> {
        match (self.kind(pat_ty), self.kind(act_ty)) {
            (InferTyKind::Ref { inner: p, .. }, InferTyKind::Ref { inner: a, .. }) => self.extract_array_const_param(p, a),
            (InferTyKind::Array { len: plen, .. }, InferTyKind::Array { len: alen, .. }) if (plen & 0xFFFF_FFFF_0000_0000) == ARRAY_LEN_PARAM_TAG && alen < ARRAY_LEN_PARAM_TAG => {
                Some(((plen & 0xFFFF) as usize, alen))
            }
            _ => None,
        }
    }

    #[inline]
    pub fn mk_param(&mut self, idx: u16, def: GlobalDefId) -> InferTyId {
        let w = ((def.krate.0 as u64) << 32) | (def.index as u64);
        self.intern_scalar(5, idx as u64, w, InferTyKind::Param { idx, def }, TyFlags::HAS_PARAM)
    }

    pub fn mk_tuple(&mut self, elems: &[InferTyId]) -> InferTyId {
        if elems.is_empty() {
            return InferTyId::UNIT;
        }
        self.intern_with_slice(6, 0, 0, elems, TyFlags::EMPTY, InferTyKind::Tuple)
    }

    pub fn mk_adt(&mut self, def: GlobalDefId, args: &[InferTyId]) -> InferTyId {
        let w = ((def.krate.0 as u64) << 32) | (def.index as u64);
        self.intern_with_slice(7, w, 0, args, TyFlags::EMPTY, |r| InferTyKind::Adt { def, args: r })
    }

    pub fn mk_fn_ptr(&mut self, inputs: &[InferTyId], output: InferTyId) -> InferTyId {
        let mark = self.ty_lists.len();
        self.ty_lists.extend_from_slice(inputs);
        self.ty_lists.push(output);
        self.finish_slice_intern(8, 0, 0, mark, TyFlags::EMPTY, |r| InferTyKind::FnPtr { sig: r })
    }

    pub fn mk_fn_def(&mut self, def: GlobalDefId, args: &[InferTyId]) -> InferTyId {
        let w = ((def.krate.0 as u64) << 32) | (def.index as u64);
        self.intern_with_slice(9, w, 0, args, TyFlags::EMPTY, |r| InferTyKind::FnDef { def, args: r })
    }

    pub fn mk_closure(&mut self, def_start: u32, def_end: u32, inputs: &[InferTyId], output: InferTyId) -> InferTyId {
        let mark = self.ty_lists.len();
        self.ty_lists.extend_from_slice(inputs);
        self.ty_lists.push(output);
        let w = ((def_start as u64) << 32) | (def_end as u64);
        self.finish_slice_intern(10, w, 0, mark, TyFlags::EMPTY, |r| InferTyKind::Closure { def_start, def_end, sig: r })
    }

    pub fn mk_alias(&mut self, trait_def: GlobalDefId, assoc_def: GlobalDefId, self_ty: InferTyId, args: &[InferTyId]) -> InferTyId {
        let w1 = ((trait_def.krate.0 as u64) << 32) | (trait_def.index as u64);
        let w2 = ((assoc_def.krate.0 as u64) << 32) | (assoc_def.index as u64) ^ ((self_ty.0 as u64).rotate_left(17));
        let base_fl = self.flags(self_ty).union(TyFlags::HAS_ALIAS);
        self.intern_with_slice(11, w1, w2, args, base_fl, |r| InferTyKind::Alias { trait_def, assoc_def, self_ty, args: r })
    }

    pub fn mk_dyn(&mut self, principal: GlobalDefId, args: &[InferTyId]) -> InferTyId {
        let w = ((principal.krate.0 as u64) << 32) | (principal.index as u64);
        self.intern_with_slice(12, w, 0, args, TyFlags::EMPTY, |r| InferTyKind::Dyn { principal, args: r })
    }

    pub fn mk_opaque(&mut self, def: GlobalDefId, args: &[InferTyId]) -> InferTyId {
        let w = ((def.krate.0 as u64) << 32) | (def.index as u64);
        self.intern_with_slice(13, w, 0, args, TyFlags::EMPTY, |r| InferTyKind::Opaque { def, args: r })
    }

    fn intern_scalar(&mut self, tag: u8, a: u64, b: u64, kind: InferTyKind, fl: TyFlags) -> InferTyId {
        let h = mix_hash(tag, a, b, &[]);
        let mask = self.buckets.len() - 1;
        let mut cur = self.buckets[(h as usize) & mask];
        while cur != u32::MAX {
            let idx = cur as usize;
            if self.hashes[idx] == h && self.kinds[idx] == kind {
                return InferTyId(cur);
            }
            cur = self.next[idx];
        }
        self.insert_hashed(kind, fl, h)
    }

    fn intern_with_slice(&mut self, tag: u8, a: u64, b: u64, items: &[InferTyId], base_fl: TyFlags, build: impl FnOnce(TySliceRange) -> InferTyKind) -> InferTyId {
        let mark = self.ty_lists.len();
        self.ty_lists.extend_from_slice(items);
        self.finish_slice_intern(tag, a, b, mark, base_fl, build)
    }

    fn finish_slice_intern(&mut self, tag: u8, a: u64, b: u64, mark: usize, mut fl: TyFlags, build: impl FnOnce(TySliceRange) -> InferTyKind) -> InferTyId {
        let len = self.ty_lists.len() - mark;
        for i in mark..self.ty_lists.len() {
            fl = fl.union(self.flags[self.ty_lists[i].0 as usize]);
        }
        let h = mix_hash(tag, a, b, &self.ty_lists[mark..]);
        let cand_range = TySliceRange { start: mark as u32, len: len as u16 };
        let cand_kind = build(cand_range);
        let mask = self.buckets.len() - 1;
        let mut cur = self.buckets[(h as usize) & mask];
        while cur != u32::MAX {
            let idx = cur as usize;
            if self.hashes[idx] == h && self.kinds_eq_slice(self.kinds[idx], cand_kind, mark, len) {
                self.ty_lists.truncate(mark);
                return InferTyId(cur);
            }
            cur = self.next[idx];
        }
        self.insert_hashed(cand_kind, fl, h)
    }

    fn kinds_eq_slice(&self, existing: InferTyKind, cand: InferTyKind, mark: usize, len: usize) -> bool {
        let slice_eq = |r: TySliceRange| r.len as usize == len && self.slice(r) == &self.ty_lists[mark..mark + len];
        match (existing, cand) {
            (InferTyKind::Tuple(r), InferTyKind::Tuple(_)) => slice_eq(r),
            (InferTyKind::Adt { def: d1, args: r1 }, InferTyKind::Adt { def: d2, .. }) => d1 == d2 && slice_eq(r1),
            (InferTyKind::FnPtr { sig: r1 }, InferTyKind::FnPtr { .. }) => slice_eq(r1),
            (InferTyKind::FnDef { def: d1, args: r1 }, InferTyKind::FnDef { def: d2, .. }) => d1 == d2 && slice_eq(r1),
            (InferTyKind::Closure { def_start: s1, def_end: e1, sig: r1 }, InferTyKind::Closure { def_start: s2, def_end: e2, .. }) => s1 == s2 && e1 == e2 && slice_eq(r1),
            (InferTyKind::Alias { trait_def: t1, assoc_def: a1, self_ty: s1, args: r1 }, InferTyKind::Alias { trait_def: t2, assoc_def: a2, self_ty: s2, .. }) => {
                t1 == t2 && a1 == a2 && s1 == s2 && slice_eq(r1)
            }
            (InferTyKind::Dyn { principal: p1, args: r1 }, InferTyKind::Dyn { principal: p2, .. }) => p1 == p2 && slice_eq(r1),
            (InferTyKind::Opaque { def: d1, args: r1 }, InferTyKind::Opaque { def: d2, .. }) => d1 == d2 && slice_eq(r1),
            _ => false,
        }
    }

    fn insert_hashed(&mut self, kind: InferTyKind, fl: TyFlags, hash: u64) -> InferTyId {
        if (self.dynamic_count + 1) * 2 >= self.buckets.len() {
            self.rehash();
        }
        let id = self.push_raw(kind, fl, hash);
        let bidx = (hash as usize) & (self.buckets.len() - 1);
        self.next[id.0 as usize] = self.buckets[bidx];
        self.buckets[bidx] = id.0;
        self.dynamic_count += 1;
        id
    }

    fn rehash(&mut self) {
        let new_len = self.buckets.len() * 2;
        self.buckets.clear();
        self.buckets.resize(new_len, u32::MAX);
        let mask = new_len - 1;
        for idx in InferTyId::BUILTIN_COUNT..self.kinds.len() {
            if matches!(self.kinds[idx], InferTyKind::Infer(_)) {
                continue;
            }
            let bidx = (self.hashes[idx] as usize) & mask;
            self.next[idx] = self.buckets[bidx];
            self.buckets[bidx] = idx as u32;
        }
    }
}

#[inline]
fn mix_hash(tag: u8, a: u64, b: u64, list: &[InferTyId]) -> u64 {
    let mut h = 0x9e37_79b9_7f4a_7c15u64 ^ (tag as u64);
    h = (h ^ a).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    h = (h ^ b).wrapping_mul(0x94d0_49bb_1331_11eb);
    for &item in list {
        h = (h ^ (item.0 as u64)).wrapping_mul(0x517c_c1b7_2722_0a95);
    }
    h ^ (h >> 31)
}

pub use super::ty::clone_ty_into;
