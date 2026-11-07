/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Flat arena-backed crate module tree (`ModuleTree`) constructed from parsed `.rs` files
//! (`mod foo;`, inline `mod foo { ... }`, `#[path = "..."]`, `#[cfg(...)]` / `#![cfg(...)]` /
//! `#[cfg_attr(...)]`, and `#[macro_use]` / item-position `macro_rules!` wrappers).

use super::db::{CrateId, Edition, FileId, ItemDb, ModuleId, SourceDb, VisKind};
use super::mod_builder::Builder;
use crate::sema_fn::norm_ident;
use crate::tree::SyntaxTree;

pub const MOD_FLAG_INLINE: u8 = 1 << 0;
pub const MOD_FLAG_UNRESOLVED: u8 = 1 << 1;
pub const MOD_FLAG_NO_IMPLICIT_PRELUDE: u8 = 1 << 2;
pub const MOD_FLAG_MACRO_USE: u8 = 1 << 3;
/// Out-of-line `mod foo;` resolved to `<dir>/foo.rs` (Rust 2018 non-`mod.rs` layout):
/// normal submodules search `<dir>/foo/`, whereas `#[path = "..."]` resolves from `<dir>/`.
pub const MOD_FLAG_NON_MOD_RS: u8 = 1 << 4;

/// Compact 64-byte flat descriptor of one module inside [`ModuleTree::modules`].
#[derive(Clone, Copy, Debug)]
pub struct ModuleData {
    pub id: ModuleId,
    pub parent: ModuleId,
    pub file: FileId,
    pub decl_file: FileId,
    /// `NodeId` of `K::Module` when inline (`mod foo { ... }`), or `u32::MAX` for a file root.
    pub inline_node: u32,
    pub decl_start: u32,
    pub decl_end: u32,
    pub(super) name_start: u32,
    pub(super) name_end: u32,
    pub(super) crate_path_start: u32,
    pub(super) crate_path_end: u32,
    pub(super) dir_start: u32,
    pub(super) dir_end: u32,
    pub(super) children_start: u32,
    pub(super) children_len: u32,
    pub vis: VisKind,
    pub flags: u8,
}

impl ModuleData {
    #[inline]
    pub const fn is_inline(&self) -> bool {
        (self.flags & MOD_FLAG_INLINE) != 0
    }

    #[inline]
    pub const fn is_unresolved(&self) -> bool {
        (self.flags & MOD_FLAG_UNRESOLVED) != 0
    }

    #[inline]
    pub const fn has_no_implicit_prelude(&self) -> bool {
        (self.flags & MOD_FLAG_NO_IMPLICIT_PRELUDE) != 0
    }

    #[inline]
    pub const fn is_non_mod_rs(&self) -> bool {
        (self.flags & MOD_FLAG_NON_MOD_RS) != 0
    }
}

/// One child edge in [`ModuleTree::children`].
#[derive(Clone, Copy, Debug)]
pub struct ModChild {
    pub child: ModuleId,
    pub(super) name_start: u32,
    pub(super) name_end: u32,
    pub vis: VisKind,
}

/// Non-fatal module-tree discovery diagnostics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModDiag {
    DisabledCrateRoot { file: FileId },
    UnresolvedModFile { parent: ModuleId, name: String, tried_primary: String, tried_mod_rs: Option<String> },
    AmbiguousModFile { parent: ModuleId, name: String, chosen: FileId, alternate: FileId },
    CyclicModFile { parent: ModuleId, name: String, file: FileId },
    DuplicateModDecl { parent: ModuleId, name: String },
}

/// Flat, arena-allocated module hierarchy of one crate.
#[derive(Debug)]
pub struct ModuleTree {
    pub krate: CrateId,
    pub edition: Edition,
    pub no_std: bool,
    pub no_core: bool,
    pub modules: Vec<ModuleData>,
    pub(super) children: Vec<ModChild>,
    /// Sorted by `(file.0, is_inline, decl_start)` for `O(log N)` file -> module lookup.
    pub(super) file_to_modules: Vec<(FileId, ModuleId)>,
    pub(super) strings: String,
    pub diagnostics: Vec<ModDiag>,
}

impl ModuleTree {
    /// Builds the module tree of `krate` starting at `root_file`, parsing each reached `.rs` file once.
    pub fn build(db: &dyn ItemDb, sources: &dyn SourceDb, krate: CrateId, root_file: FileId) -> Self {
        Self::build_cached(db, sources, krate, root_file, |_| None)
    }

    /// Builds the module tree reusing caller-supplied pre-parsed [`SyntaxTree`]s when available.
    pub fn build_cached<'t>(db: &dyn ItemDb, sources: &dyn SourceDb, krate: CrateId, root_file: FileId, mut get_tree: impl FnMut(FileId) -> Option<&'t SyntaxTree<'t>>) -> Self {
        Builder::new(db, sources, krate, &mut get_tree).run(root_file)
    }

    #[inline]
    pub fn module(&self, id: ModuleId) -> &ModuleData {
        &self.modules[id.0 as usize]
    }

    #[inline]
    pub fn name(&self, id: ModuleId) -> &str {
        let m = self.module(id);
        &self.strings[m.name_start as usize..m.name_end as usize]
    }

    #[inline]
    pub fn crate_path(&self, id: ModuleId) -> &str {
        let m = self.module(id);
        &self.strings[m.crate_path_start as usize..m.crate_path_end as usize]
    }

    /// Directory searched for default submodules (`mod sub;`) declared inside `id`.
    #[inline]
    pub fn dir_path(&self, id: ModuleId) -> &str {
        let m = self.module(id);
        &self.strings[m.dir_start as usize..m.dir_end as usize]
    }

    /// Directory against which explicit `#[path = "..."]` attributes inside `id` resolve.
    #[inline]
    pub fn attr_base_dir(&self, id: ModuleId) -> &str {
        let m = self.module(id);
        let dir = &self.strings[m.dir_start as usize..m.dir_end as usize];
        if m.is_non_mod_rs() {
            parent_dir_str(dir)
        } else {
            dir
        }
    }

    #[inline]
    pub fn children(&self, id: ModuleId) -> &[ModChild] {
        let m = self.module(id);
        let s = m.children_start as usize;
        &self.children[s..s + m.children_len as usize]
    }

    #[inline]
    pub fn child_name(&self, edge: &ModChild) -> &str {
        &self.strings[edge.name_start as usize..edge.name_end as usize]
    }

    /// Finds direct child module `name` of `parent`.
    pub fn find_child(&self, parent: ModuleId, name: &str) -> Option<ModuleId> {
        let target = norm_ident(name);
        self.children(parent).iter().find(|e| self.child_name(e) == target).map(|e| e.child)
    }

    /// Resolves a module path like `"crate::sync::mpsc"` or `"sync::mpsc"` from crate root.
    pub fn find_by_path(&self, path: &str) -> Option<ModuleId> {
        if self.modules.is_empty() {
            return None;
        }
        let mut cur = ModuleId::ROOT;
        for seg in path.split("::").filter(|s| !s.is_empty()) {
            if seg == "crate" {
                cur = ModuleId::ROOT;
            } else {
                cur = self.find_child(cur, seg)?;
            }
        }
        Some(cur)
    }

    /// Returns all `ModuleId`s backed by `file` (file-root module first, followed by inline modules).
    pub fn modules_for_file(&self, file: FileId) -> impl Iterator<Item = ModuleId> + '_ {
        let start = self.file_to_modules.partition_point(|&(f, _)| f < file);
        let end = self.file_to_modules[start..].partition_point(|&(f, _)| f == file) + start;
        self.file_to_modules[start..end].iter().map(|&(_, m)| m)
    }

    /// Innermost module in `file` whose byte span covers `byte_offset`.
    pub fn module_at_offset(&self, file: FileId, byte_offset: u32) -> Option<ModuleId> {
        let mut best: Option<ModuleId> = None;
        for m_id in self.modules_for_file(file) {
            let m = self.module(m_id);
            if !m.is_inline() {
                if best.is_none() {
                    best = Some(m_id);
                }
            } else if m.decl_start <= byte_offset && byte_offset <= m.decl_end {
                best = Some(m_id);
            }
        }
        best
    }
}

#[inline]
pub(super) fn parent_dir_str(path: &str) -> &str {
    match path.rfind('/') {
        Some(idx) => &path[..idx],
        None => "",
    }
}

#[inline]
fn push_norm_segment(buf: &mut String, part: &str) {
    match part {
        "" | "." => {}
        ".." => {
            if let Some(slash) = buf.rfind('/') {
                if &buf[slash + 1..] != ".." {
                    buf.truncate(slash);
                    return;
                }
            } else if !buf.is_empty() && buf != ".." {
                buf.clear();
                return;
            }
            if !buf.is_empty() {
                buf.push('/');
            }
            buf.push_str("..");
        }
        normal => {
            if !buf.is_empty() {
                buf.push('/');
            }
            buf.push_str(normal);
        }
    }
}

/// Joins `base_dir`, `rel`, and `suffix` into `out` as a normalized `/` path with zero extra allocs.
pub fn join_norm_into(out: &mut String, base_dir: &str, rel: &str, suffix: &str) {
    out.clear();
    for part in base_dir.split(['/', '\\']) {
        push_norm_segment(out, part);
    }
    let mut rel_it = rel.split(['/', '\\']).peekable();
    while let Some(part) = rel_it.next() {
        if rel_it.peek().is_none() && !suffix.is_empty() {
            if suffix.starts_with('/') {
                push_norm_segment(out, part);
                for sp in suffix.split('/') {
                    push_norm_segment(out, sp);
                }
            } else {
                push_norm_segment(out, part);
                out.push_str(suffix);
            }
        } else {
            push_norm_segment(out, part);
        }
    }
}

/// Convenience wrapper returning a normalized joined `/` path.
pub fn join_norm_path(base_dir: &str, rel: &str) -> String {
    let mut out = String::with_capacity(base_dir.len() + rel.len() + 1);
    join_norm_into(&mut out, base_dir, rel, "");
    out
}
