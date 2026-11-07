/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Simplified `impl` target keys, indexed `impl` descriptors, and in-memory [`OverlaySourceDb`]
//! for dirty-file overrides over an existing [`SourceDb`].

use std::collections::HashMap;

use super::db::{FileId, GlobalDefId, SourceDb};
use super::mod_tree::join_norm_path;
use super::per_ns::PrimTy;

/// Arena index into [`super::def_map::CrateDefMap::local_impls`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LocalImplId(pub u32);

impl LocalImplId {
    pub const NONE: Self = Self(u32::MAX);

    #[inline]
    pub const fn is_some(self) -> bool {
        self.0 != u32::MAX
    }
}

impl Default for LocalImplId {
    #[inline]
    fn default() -> Self {
        Self::NONE
    }
}

/// Canonical simplified self-type head of an `impl` block (matches `.ridx` `v5` `SimplifiedType` buckets).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ImplTargetKey {
    #[default]
    Unknown,
    /// `struct`, `enum`, or `union` identified by compiler or `LOCAL_OVERLAY` [`GlobalDefId`].
    Adt(GlobalDefId),
    /// Builtin primitive (`u32`, `str`, `bool`, ...).
    Prim(PrimTy),
    /// `[T]` slice type.
    Slice,
    /// `[T; N]` fixed-size array type.
    Array,
    /// `(T0, T1, ...)` tuple of the given arity (`0` for unit `()`).
    Tuple(u16),
    /// `&T` (`is_mut = false`) or `&mut T` (`is_mut = true`).
    Ref { is_mut: bool },
    /// `*const T` (`is_mut = false`) or `*mut T` (`is_mut = true`).
    Ptr { is_mut: bool },
    /// `fn(A, B) -> R` function pointer of the given parameter count.
    FnPtr { arity: u16 },
    /// `dyn Trait + ...` trait object keyed by principal trait [`GlobalDefId`].
    Dyn(GlobalDefId),
    /// `!` never type.
    Never,
    /// Blanket generic parameter self type (`impl<T> Trait for T`).
    Param,
}

/// One `impl` block entry in [`super::db::ItemDb`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IndexedImpl {
    pub id: GlobalDefId,
    /// Defining workspace [`FileId`] (`FileId::NONE` for external dependency crates).
    /// When `file` is in `dirty_files`, this indexed `impl` is suppressed in favor of the dirty CST.
    pub file: FileId,
    pub target: ImplTargetKey,
    /// Implemented trait [`GlobalDefId`] (`GlobalDefId::NONE` for inherent `impl` blocks).
    pub trait_def: GlobalDefId,
    pub is_negative: bool,
}

impl IndexedImpl {
    #[inline]
    pub const fn is_inherent(&self) -> bool {
        !self.trait_def.is_some()
    }
}

/// Overlay [`SourceDb`] layering unsaved/edited file contents (and newly created unsaved files)
/// on top of an immutable base [`SourceDb`] without copying clean files.
pub struct OverlaySourceDb<'a> {
    base: &'a dyn SourceDb,
    base_file_count: u32,
    overrides: HashMap<FileId, String>,
    added_files: Vec<(String, String)>,
    added_by_path: HashMap<String, FileId>,
    dirty_list: Vec<FileId>,
}

impl<'a> OverlaySourceDb<'a> {
    pub fn new(base: &'a dyn SourceDb, base_file_count: u32) -> Self {
        Self { base, base_file_count, overrides: HashMap::new(), added_files: Vec::new(), added_by_path: HashMap::new(), dirty_list: Vec::new() }
    }

    /// Overrides the text of an existing `file` and marks `file` dirty.
    pub fn set_file_override(&mut self, file: FileId, new_text: impl Into<String>) {
        self.overrides.insert(file, new_text.into());
        if !self.dirty_list.contains(&file) {
            self.dirty_list.push(file);
        }
    }

    /// Overrides an existing path in `base` or registers a brand-new unsaved virtual file at `rel_path`.
    pub fn upsert_path(&mut self, rel_path: &str, new_text: impl Into<String>) -> FileId {
        let norm = join_norm_path("", rel_path);
        if let Some(existing) = self.base.resolve_path(&norm) {
            self.set_file_override(existing, new_text);
            return existing;
        }
        if let Some(&added_id) = self.added_by_path.get(&norm) {
            let idx = (added_id.0 - self.base_file_count) as usize;
            self.added_files[idx].1 = new_text.into();
            return added_id;
        }
        let id = FileId(self.base_file_count + self.added_files.len() as u32);
        self.added_files.push((norm.clone(), new_text.into()));
        self.added_by_path.insert(norm, id);
        self.dirty_list.push(id);
        id
    }

    /// Slice of all dirty [`FileId`]s modified or added in this overlay.
    #[inline]
    pub fn dirty_files(&self) -> &[FileId] {
        &self.dirty_list
    }
}

impl SourceDb for OverlaySourceDb<'_> {
    fn file_path(&self, file: FileId) -> &str {
        if file.0 >= self.base_file_count {
            if let Some((p, _)) = self.added_files.get((file.0 - self.base_file_count) as usize) {
                return p.as_str();
            }
        }
        self.base.file_path(file)
    }

    fn file_text(&self, file: FileId) -> &str {
        if let Some(ov) = self.overrides.get(&file) {
            return ov.as_str();
        }
        if file.0 >= self.base_file_count {
            if let Some((_, t)) = self.added_files.get((file.0 - self.base_file_count) as usize) {
                return t.as_str();
            }
        }
        self.base.file_text(file)
    }

    fn resolve_path(&self, rel_path: &str) -> Option<FileId> {
        let norm = join_norm_path("", rel_path);
        self.added_by_path.get(&norm).copied().or_else(|| self.base.resolve_path(&norm))
    }
}
