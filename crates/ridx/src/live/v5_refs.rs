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
use ridx_format::{CtorShape, DefKind, IndexReader, ItemVis, TyKindView, UseOwner, UsePrimTy, WorkspaceIndex, USE_FLAG_GLOB, USE_FLAG_UNDERSCORE};
use ridx_syntax::name_res::{CrateId, GlobalDefId, ItemKind, PrimTy, VisKind};

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

#[inline]
pub fn map_vis(v: u8) -> VisKind {
    match v {
        x if x == ItemVis::Public as u8 => VisKind::Public,
        x if x == ItemVis::Crate as u8 => VisKind::Crate,
        x if x == ItemVis::Restricted as u8 => VisKind::Restricted,
        _ => VisKind::Private,
    }
}

#[inline]
pub fn map_use_prim(p: UsePrimTy) -> u8 {
    match p {
        UsePrimTy::None => 0,
        UsePrimTy::Bool => PrimTy::Bool as u8,
        UsePrimTy::Char => PrimTy::Char as u8,
        UsePrimTy::Str => PrimTy::Str as u8,
        UsePrimTy::I8 => PrimTy::I8 as u8,
        UsePrimTy::I16 => PrimTy::I16 as u8,
        UsePrimTy::I32 => PrimTy::I32 as u8,
        UsePrimTy::I64 => PrimTy::I64 as u8,
        UsePrimTy::I128 => PrimTy::I128 as u8,
        UsePrimTy::Isize => PrimTy::Isize as u8,
        UsePrimTy::U8 => PrimTy::U8 as u8,
        UsePrimTy::U16 => PrimTy::U16 as u8,
        UsePrimTy::U32 => PrimTy::U32 as u8,
        UsePrimTy::U64 => PrimTy::U64 as u8,
        UsePrimTy::U128 => PrimTy::U128 as u8,
        UsePrimTy::Usize => PrimTy::Usize as u8,
        UsePrimTy::F32 => PrimTy::F32 as u8,
        UsePrimTy::F64 => PrimTy::F64 as u8,
        UsePrimTy::F16 | UsePrimTy::F128 => 0,
    }
}

#[inline]
pub fn map_ridx_kind(dk: DefKind) -> Option<ItemKind> {
    match dk {
        DefKind::Mod => Some(ItemKind::Mod),
        DefKind::Struct | DefKind::StructCtor => Some(ItemKind::Struct),
        DefKind::Enum => Some(ItemKind::Enum),
        DefKind::Union => Some(ItemKind::Union),
        DefKind::Trait | DefKind::TraitAlias => Some(ItemKind::Trait),
        DefKind::Fn => Some(ItemKind::Fn),
        DefKind::TyAlias | DefKind::ForeignTy => Some(ItemKind::TypeAlias),
        DefKind::Const => Some(ItemKind::Const),
        DefKind::Static => Some(ItemKind::Static),
        DefKind::Variant | DefKind::VariantCtor => Some(ItemKind::Variant),
        DefKind::Field => Some(ItemKind::Field),
        DefKind::AssocFn => Some(ItemKind::AssocFn),
        DefKind::AssocTy => Some(ItemKind::AssocTy),
        DefKind::AssocConst => Some(ItemKind::AssocConst),
        DefKind::Macro => Some(ItemKind::Macro),
        DefKind::Impl => Some(ItemKind::Impl),
        DefKind::Use => Some(ItemKind::Use),
        DefKind::ExternCrate => Some(ItemKind::ExternCrate),
        _ => None,
    }
}

#[inline]
pub fn def_has_value_ctor(reader: &IndexReader<Mmap>, def_idx: u32, kind: ItemKind) -> bool {
    if !matches!(kind, ItemKind::Struct | ItemKind::Variant) {
        return false;
    }
    if let Some(dt) = reader.def_ty_for_def(def_idx) {
        return matches!(dt.ctor_shape(), CtorShape::Tuple | CtorShape::Unit);
    }
    reader.fn_sig_for_def(def_idx).is_some() || reader.fields_for_owner(def_idx).next().is_none()
}

#[inline]
fn def_idx_of_ref(defs: &[ridx_format::DefEntry], def: &ridx_format::DefEntry) -> u32 {
    ((def as *const ridx_format::DefEntry as usize - defs.as_ptr() as usize) / std::mem::size_of::<ridx_format::DefEntry>()) as u32
}

pub fn lookup_def_idx_by_exact_path(reader: &IndexReader<Mmap>, shard: &super::v5_shard::ShardTables, path: &str) -> Option<u32> {
    let defs = reader.definitions();
    let def = reader.find_definition_by_path(path)?;
    let dk = DefKind::from(def.kind);
    let didx = def_idx_of_ref(defs, def);
    if !def.is_reexport() && !matches!(dk, DefKind::Impl | DefKind::Use | DefKind::StructCtor | DefKind::VariantCtor) {
        return Some(didx);
    }
    let mut fallback = (!matches!(dk, DefKind::Impl | DefKind::Use)).then_some(didx);
    let p_off = def.path_offset();
    let lo = didx.saturating_sub(3) as usize;
    let hi = (didx as usize + 4).min(defs.len());
    for (off, cd) in defs[lo..hi].iter().enumerate() {
        let cdk = DefKind::from(cd.kind);
        if cd.path_offset() == p_off && !matches!(cdk, DefKind::Impl | DefKind::Use) {
            let ci = (lo + off) as u32;
            if !cd.is_reexport() && !matches!(cdk, DefKind::StructCtor | DefKind::VariantCtor) {
                return Some(ci);
            }
            fallback.get_or_insert(ci);
        }
    }
    if let Some((parent, _)) = path.rsplit_once("::") {
        for cd in reader.find_definitions_by_owner(parent) {
            let cdk = DefKind::from(cd.kind);
            if reader.get_str(cd.path_offset()) == path && !matches!(cdk, DefKind::Impl | DefKind::Use) {
                let ci = def_idx_of_ref(defs, cd);
                if !cd.is_reexport() && !matches!(cdk, DefKind::StructCtor | DefKind::VariantCtor) {
                    return Some(ci);
                }
                fallback.get_or_insert(ci);
            }
        }
    } else if let Some(children) = shard.mod_children.get(&shard.root_mod_def_idx.unwrap_or(super::v5_db::CRATE_ROOT_DEF_IDX)) {
        for &ci in children {
            if let Some(cd) = defs.get(ci as usize) {
                let cdk = DefKind::from(cd.kind);
                if reader.get_str(cd.path_offset()) == path && !matches!(cdk, DefKind::Impl | DefKind::Use) {
                    if !cd.is_reexport() && !matches!(cdk, DefKind::StructCtor | DefKind::VariantCtor) {
                        return Some(ci);
                    }
                    fallback.get_or_insert(ci);
                }
            }
        }
    }
    fallback
}
