/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! File-level expression, type, pattern, and `use`-tree path resolution (`FileNameRes`, `ResolvedRef`,
//! `ResTarget`, `LocalBindData`, `GenericBindData`).

use super::body_ribs::RibStack;
use super::body_walk::FileResolver;
use super::crate_snapshot::CrateSnapshot;
use super::db::{FileId, GlobalDefId, ItemDb, ItemKind, SourceDb};
use super::db_impls::ImplTargetKey;
use super::def_map::CrateDefMap;
use super::ext_scope::ExtScopeCache;
use super::mod_tree::ModuleTree;
use super::per_ns::{LocalItemId, PrimTy};
use crate::parser::parse;
use crate::tree::{NodeId, SyntaxTree};

/// Dense index into [`FileNameRes::locals`] (`u32::MAX` = absent sentinel).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LocalBindId(pub u32);

impl LocalBindId {
    pub const NONE: Self = Self(u32::MAX);

    #[inline]
    pub const fn is_some(self) -> bool {
        self.0 != u32::MAX
    }
}

impl Default for LocalBindId {
    #[inline]
    fn default() -> Self {
        Self::NONE
    }
}

/// Dense index into [`FileNameRes::generics`] (`u32::MAX` = absent sentinel).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GenericBindId(pub u32);

impl GenericBindId {
    pub const NONE: Self = Self(u32::MAX);

    #[inline]
    pub const fn is_some(self) -> bool {
        self.0 != u32::MAX
    }
}

impl Default for GenericBindId {
    #[inline]
    fn default() -> Self {
        Self::NONE
    }
}

/// Declaration record of one local value binding (`self`, parameter, `let`, `match`, `for`, closure).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalBindData {
    pub id: LocalBindId,
    pub decl_node: NodeId,
    pub name_start: u32,
    pub name_end: u32,
    pub is_self: bool,
    pub is_mut: bool,
}

impl LocalBindData {
    #[inline]
    pub fn name<'s>(&self, src: &'s str) -> &'s str {
        &src[self.name_start as usize..self.name_end as usize]
    }
}

/// Declaration record of one generic `T` (`is_const = false`) or `const N: Ty` (`is_const = true`) parameter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GenericBindData {
    pub id: GenericBindId,
    pub def: GlobalDefId,
    pub owner_node: NodeId,
    pub owner_def: GlobalDefId,
    pub decl_node: NodeId,
    pub name_start: u32,
    pub name_end: u32,
    pub index: u16,
    pub is_const: bool,
}

impl GenericBindData {
    #[inline]
    pub fn name<'s>(&self, src: &'s str) -> &'s str {
        &src[self.name_start as usize..self.name_end as usize]
    }
}

/// Syntactic role of a resolved path or identifier site.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PathCtx {
    Expr = 0,
    RecordExpr = 1,
    Type = 2,
    TypeArg = 3,
    TypeBound = 4,
    PatValue = 5,
    PatRecord = 6,
    UseTree = 7,
    MacroCall = 8,
}

/// Semantic target of a resolved identifier / path segment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ResTarget {
    /// Module, ADT, variant, function, associated item, field, const, static, type alias, or macro.
    Def { def: GlobalDefId, local: LocalItemId, kind: ItemKind },
    /// Local variable or `self` parameter declared in the enclosing body.
    Local(LocalBindId),
    /// Generic type parameter (`!is_const`) or const generic parameter (`is_const`).
    Generic(GenericBindId),
    /// `Self` inside an `impl` (`target` known) or `trait` (`trait_def` known).
    SelfTy { target: ImplTargetKey, trait_def: GlobalDefId },
    /// Builtin primitive type (`u32`, `str`, `bool`, ...).
    Prim(PrimTy),
}

impl ResTarget {
    /// Returns the underlying [`GlobalDefId`] when this target names a concrete definition
    /// (including `Self` inside `impl Adt`, which maps to `adt_def` just like `rustc`'s `SelfTyAlias`).
    #[inline]
    pub const fn effective_def_id(self) -> Option<GlobalDefId> {
        match self {
            Self::Def { def, .. } if def.is_some() => Some(def),
            Self::SelfTy { target: ImplTargetKey::Adt(adt_def), .. } if adt_def.is_some() => Some(adt_def),
            _ => None,
        }
    }

    #[inline]
    pub fn effective_def_id_in(self, file_res: &FileNameRes) -> Option<GlobalDefId> {
        match self {
            Self::Generic(gid) => {
                let d = file_res.generic_data(gid).def;
                d.is_some().then_some(d)
            }
            other => other.effective_def_id(),
        }
    }

    #[inline]
    pub const fn is_local(self) -> bool {
        matches!(self, Self::Local(_))
    }
}

/// One resolved reference span `[start..end)` inside a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolvedRef {
    pub start: u32,
    pub end: u32,
    /// Enclosing `K::Path` node (`NodeId(u32::MAX)` for bare `IdentPat`, `RecordField`, `Use`, or `TokenTree` refs).
    pub path_node: NodeId,
    pub target: ResTarget,
    pub ctx: PathCtx,
    pub is_segment_tail: bool,
}

/// Reusable output table of all resolved paths, locals, and generic parameters in one `.rs` file.
#[derive(Clone, Debug, Default)]
pub struct FileNameRes {
    pub file: FileId,
    pub refs: Vec<ResolvedRef>,
    pub locals: Vec<LocalBindData>,
    pub generics: Vec<GenericBindData>,
    pub(super) ribs: RibStack,
}

impl FileNameRes {
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub fn clear(&mut self) {
        self.file = FileId::NONE;
        self.refs.clear();
        self.locals.clear();
        self.generics.clear();
        self.ribs.clear();
    }

    pub(super) fn finalize(&mut self) {
        self.refs.sort_unstable_by(|a, b| a.start.cmp(&b.start).then_with(|| a.end.cmp(&b.end)));
        self.refs.dedup_by(|b, a| a.start == b.start && a.end == b.end && a.target == b.target);
    }

    /// Binary-searches for a reference covering `byte_offset` (`start <= byte_offset < end`).
    pub fn ref_at_offset(&self, byte_offset: u32) -> Option<&ResolvedRef> {
        let idx = self.refs.partition_point(|r| r.start <= byte_offset);
        self.refs[..idx].iter().rfind(|r| byte_offset < r.end)
    }

    /// Returns all resolved references with exact byte span `[start..end)`.
    pub fn refs_at_span(&self, start: u32, end: u32) -> &[ResolvedRef] {
        let lo = self.refs.partition_point(|r| (r.start, r.end) < (start, end));
        let hi = self.refs[lo..].partition_point(|r| r.start == start && r.end == end);
        &self.refs[lo..lo + hi]
    }

    /// Returns the tail segment resolution of `path_node` (`K::Path`), if resolved.
    pub fn path_tail_ref(&self, path_node: NodeId) -> Option<&ResolvedRef> {
        self.refs.iter().find(|r| r.path_node == path_node && r.is_segment_tail)
    }

    #[inline]
    pub fn local_data(&self, id: LocalBindId) -> &LocalBindData {
        &self.locals[id.0 as usize]
    }

    #[inline]
    pub fn generic_data(&self, id: GenericBindId) -> &GenericBindData {
        &self.generics[id.0 as usize]
    }

    /// Iterates over every reference resolving to `local_id` in source order.
    pub fn local_uses(&self, local_id: LocalBindId) -> impl Iterator<Item = &ResolvedRef> {
        self.refs.iter().filter(move |r| r.target == ResTarget::Local(local_id))
    }
}

impl CrateDefMap {
    /// Parses `file` from `sources` and resolves all expressions, types, patterns, and `use` paths in `file`.
    pub fn resolve_file_paths(&self, db: &dyn ItemDb, sources: &dyn SourceDb, mod_tree: &ModuleTree, file: FileId) -> FileNameRes {
        let st = parse(sources.file_text(file));
        let mut ext_cache = ExtScopeCache::new();
        let mut out = FileNameRes::new();
        self.resolve_file_paths_into(db, mod_tree, &mut ext_cache, file, &st, &mut out);
        out
    }

    /// Resolves all paths in pre-parsed `st` for `file` into caller-reusable `out` (retaining `Vec` capacities).
    pub fn resolve_file_paths_into(&self, db: &dyn ItemDb, mod_tree: &ModuleTree, ext_cache: &mut ExtScopeCache, file: FileId, st: &SyntaxTree<'_>, out: &mut FileNameRes) {
        out.clear();
        out.file = file;
        let ribs = std::mem::take(&mut out.ribs);
        let mut resolver = FileResolver::new(self, db, mod_tree, ext_cache, file, st, out, ribs);
        resolver.walk_file();
        let ribs = resolver.ribs;
        out.ribs = ribs;
        out.finalize();
    }
}

impl CrateSnapshot {
    /// Resolves all paths in `st` for clean `file` reusing the snapshot's warm [`ExtScopeCache`].
    pub fn resolve_file_paths_into(&self, db: &dyn ItemDb, file: FileId, st: &SyntaxTree<'_>, out: &mut FileNameRes) {
        let mut ext_c = self.ext_cache.borrow_mut();
        self.clean_def_map.resolve_file_paths_into(db, &self.mod_tree, &mut ext_c, file, st, out);
    }

    /// Resolves all paths in `st` for `file` against a dirty overlay `def_map` (and optional `new_mod_tree`),
    /// reusing the snapshot's warm [`ExtScopeCache`].
    pub fn resolve_dirty_file_paths_into(
        &self,
        db: &dyn ItemDb,
        new_mod_tree: Option<&ModuleTree>,
        def_map: &CrateDefMap,
        file: FileId,
        st: &SyntaxTree<'_>,
        out: &mut FileNameRes,
    ) {
        let mut ext_c = self.ext_cache.borrow_mut();
        def_map.resolve_file_paths_into(db, new_mod_tree.unwrap_or(&self.mod_tree), &mut ext_c, file, st, out);
    }
}
