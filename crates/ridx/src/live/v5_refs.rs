/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Lifetime-free string-pool references (`PoolStr`), atomic owner cell, temporary state wrappers,
//! and `.ridx` shard alias-pair collection.

use std::collections::HashSet;
use std::ops::Deref;
use std::sync::atomic::{AtomicU64, Ordering};

use memmap2::Mmap;
use ridx_format::{DefKind, IndexReader, TyKindView, UseOwner, WorkspaceIndex, USE_FLAG_GLOB, USE_FLAG_UNDERSCORE};
use ridx_syntax::name_res::{CrateId, GlobalDefId};

use super::meta::CorpusSourceDb;
use super::v5_db::LiveState;

#[inline]
pub fn fnv1a_str(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in s.as_bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// Lifetime-free `(reader_idx, string_pool_offset)` handle resolved against `&WorkspaceIndex` on demand.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PoolStr {
    pub r_idx: u32,
    pub offset: u32,
}

impl PoolStr {
    #[inline]
    pub fn from_offset(r_idx: usize, reader: &IndexReader<Mmap>, raw_off: u32, strip_raw: bool) -> Self {
        let off = if strip_raw && reader.get_str(raw_off).starts_with("r#") { raw_off + 2 } else { raw_off };
        Self { r_idx: r_idx as u32, offset: off }
    }

    #[inline]
    pub fn from_pool_slice(r_idx: usize, reader: &IndexReader<Mmap>, s: &str) -> Option<Self> {
        let pool_len = reader.header().string_pool_len as usize;
        if pool_len == 0 {
            return None;
        }
        let base = reader.get_str(0).as_ptr() as usize;
        let off = (s.as_ptr() as usize).checked_sub(base)?;
        (off < pool_len && std::ptr::eq(reader.get_str(off as u32).as_ptr(), s.as_ptr())).then_some(Self { r_idx: r_idx as u32, offset: off as u32 })
    }

    #[inline]
    pub fn as_str(self, ws: &WorkspaceIndex) -> &str {
        ws.readers().get(self.r_idx as usize).map_or("", |r| r.get_str(self.offset))
    }
}

/// Thread-safe (`Send + Sync`) atomic cell storing an active `GlobalDefId`.
pub struct AtomicDefCell(AtomicU64);

impl Default for AtomicDefCell {
    fn default() -> Self {
        Self::new(GlobalDefId::NONE)
    }
}

impl AtomicDefCell {
    #[inline]
    pub const fn new(def: GlobalDefId) -> Self {
        Self(AtomicU64::new(((def.krate.0 as u64) << 32) | (def.index as u64)))
    }

    #[inline]
    pub fn get(&self) -> GlobalDefId {
        let raw = self.0.load(Ordering::Relaxed);
        GlobalDefId::new(CrateId((raw >> 32) as u32), raw as u32)
    }

    #[inline]
    pub fn set(&self, def: GlobalDefId) {
        let raw = ((def.krate.0 as u64) << 32) | (def.index as u64);
        self.0.store(raw, Ordering::Relaxed);
    }
}

/// Borrowed or mutably-borrowed view of [`CorpusSourceDb`] inside a temporary [`super::v5_db::RidxV5ItemDb`].
pub enum SourceStore<'a> {
    Borrowed(&'a CorpusSourceDb),
    BorrowedMut(&'a mut CorpusSourceDb),
}

impl Deref for SourceStore<'_> {
    type Target = CorpusSourceDb;
    #[inline]
    fn deref(&self) -> &Self::Target {
        match self {
            Self::Borrowed(s) => s,
            Self::BorrowedMut(s) => s,
        }
    }
}

impl SourceStore<'_> {
    #[inline]
    pub(crate) fn owned_mut(&mut self) -> Option<&mut CorpusSourceDb> {
        match self {
            Self::Borrowed(_) => None,
            Self::BorrowedMut(s) => Some(s),
        }
    }
}

/// Temporary or owned reference to [`LiveState`] inside [`super::v5_db::RidxV5ItemDb`].
pub enum LiveStateRef<'a> {
    Borrowed(&'a LiveState),
    BorrowedMut(&'a mut LiveState),
    Owned(Box<LiveState>),
}

impl Deref for LiveStateRef<'_> {
    type Target = LiveState;
    #[inline]
    fn deref(&self) -> &Self::Target {
        match self {
            Self::Borrowed(s) => s,
            Self::BorrowedMut(s) => s,
            Self::Owned(s) => s,
        }
    }
}

impl LiveStateRef<'_> {
    #[inline]
    pub(crate) fn as_mut(&mut self) -> &mut LiveState {
        match self {
            Self::Borrowed(_) => unreachable!("immutable LiveStateRef cannot be mutated"),
            Self::BorrowedMut(s) => s,
            Self::Owned(s) => s,
        }
    }
}

pub fn collect_shard_alias_pairs(reader: &IndexReader<Mmap>, out: &mut HashSet<(String, String)>) {
    let arena = reader.ty_arena();
    let mut owners = vec![UseOwner::CrateRoot];
    for (didx, d) in reader.definitions().iter().enumerate() {
        let dk = DefKind::from(d.kind);
        if !d.is_reexport() && dk == DefKind::Mod {
            owners.push(UseOwner::Def(didx as u32));
        }
        if d.is_reexport() {
            let alias = reader.get_str(d.name_offset()).trim_start_matches("r#");
            let sig = reader.get_str(d.sig_offset());
            let orig_leaf = sig.rsplit("::").next().unwrap_or(sig).trim_start_matches("r#");
            if !alias.is_empty() && !orig_leaf.is_empty() && alias != orig_leaf {
                out.insert((alias.to_string(), orig_leaf.to_string()));
            }
        } else if dk == DefKind::TyAlias {
            let alias = reader.get_str(d.name_offset()).trim_start_matches("r#");
            if let Some(dt) = reader.def_ty_for_def(didx as u32) {
                if let TyKindView::Adt { def, .. } = arena.kind(dt.ty()) {
                    if let Some(td) = arena.def(def) {
                        let orig_leaf = td.path.rsplit("::").next().unwrap_or(td.path).trim_start_matches("r#");
                        if !alias.is_empty() && !orig_leaf.is_empty() && alias != orig_leaf {
                            out.insert((alias.to_string(), orig_leaf.to_string()));
                        }
                    }
                }
            }
        }
    }
    for owner in owners {
        for u in reader.uses_for_owner(owner) {
            if (u.flags & (USE_FLAG_GLOB | USE_FLAG_UNDERSCORE)) != 0 || u.name.is_empty() {
                continue;
            }
            let alias = u.name.trim_start_matches("r#");
            for td_id in [u.type_def, u.value_def, u.macro_def].into_iter().flatten() {
                if let Some(td) = arena.def(td_id) {
                    let orig_leaf = td.path.rsplit("::").next().unwrap_or(td.path).trim_start_matches("r#");
                    if !orig_leaf.is_empty() && alias != orig_leaf {
                        out.insert((alias.to_string(), orig_leaf.to_string()));
                    }
                }
            }
        }
    }
}
