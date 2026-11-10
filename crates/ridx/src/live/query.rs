/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Production LSP query API (`LiveDb`) over an already-open `WorkspaceIndex` and optional
//! `OverlayManager` buffers (`Checkbox 4.2` / `4.3`):
//! - [`LiveDb::open`]: zero-cost lazy handle; builds `CrateSnapshot` and reachable `.ridx` shard
//!   tables on first query per crate.
//! - [`LiveDb::type_at`]: resolves expression, local variable, pattern, or type-annotation under
//!   `byte_offset` into rustc/RA display text plus definition locations (`file`, `line`, `col`)
//!   of all participating ADTs, traits, and type aliases (`textDocument/typeDefinition` & `hover`).
//! - [`LiveDb::dot_candidates`]: resolves receiver type before `.` and returns accessible fields,
//!   inherent methods, and in-scope trait methods in exact `method_res` autoderef order.
//! - [`LiveDb::path_candidates`]: resolves `Type::`, `Self::`, `<T as Trait>::`, `Enum::`, and
//!   `module::` qualifiers to associated items, enum variants, and module items.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use ridx_format::{DefKind, WorkspaceIndex};

use crate::overlay::OverlayManager;

use super::query_dot::query_dot_candidates;
use super::query_path::query_path_candidates;
use super::query_state::LiveEngine;
use super::query_ty_fmt::query_type_at;

/// 1-based `(line, col)` .. `(end_line, end_col)` character span in a resolved file.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LiveLocation {
    pub file: PathBuf,
    pub line: u32,
    pub col: u32,
    pub end_line: u32,
    pub end_col: u32,
}

/// Inferred type at cursor formatted in rustc/RA style together with component type definitions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveTy {
    /// Concise rustc/RA display string (`Vec<u8>`, `&mut Foo<Bar>`, `impl Fn(u32) -> bool`).
    pub display: String,
    /// Module-qualified rustc path display (`alloc::vec::Vec<u8>`, `my_crate::Foo<my_crate::Bar>`).
    pub full_display: String,
    /// Deduplicated pre-order definitions of all ADTs, traits, and aliases occurring in this type.
    pub defs: Vec<LiveLocation>,
}

/// Completion candidate returned by [`LiveDb::dot_candidates`] and [`LiveDb::path_candidates`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cand {
    pub label: String,
    pub kind: DefKind,
    pub detail: Option<String>,
    pub def_location: Option<LiveLocation>,
    pub from_trait: Option<String>,
}

/// Lazy live type-inference and completion database over an open [`WorkspaceIndex`].
pub struct LiveDb<'w> {
    ws: &'w WorkspaceIndex,
    engine: Option<LiveEngine<'w>>,
    manual_overlays: HashMap<PathBuf, String>,
}

impl<'w> LiveDb<'w> {
    /// Opens a lazy `LiveDb` view over `ws` without reading files or materializing shard tables upfront.
    #[inline]
    pub fn open(ws: &'w WorkspaceIndex) -> Self {
        Self { ws, engine: None, manual_overlays: HashMap::new() }
    }

    /// Overrides the live buffer text of `file` in memory (used alongside or instead of [`OverlayManager`]).
    pub fn set_file_overlay(&mut self, file: impl AsRef<Path>, text: String) {
        let p = file.as_ref();
        let key = p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
        self.manual_overlays.insert(key, text);
    }

    /// Removes any manual buffer override previously registered for `file`.
    pub fn clear_file_overlay(&mut self, file: impl AsRef<Path>) {
        let p = file.as_ref();
        self.manual_overlays.remove(p);
        if let Ok(c) = p.canonicalize() {
            self.manual_overlays.remove(&c);
        }
    }

    /// Returns the inferred type of the expression, local variable, pattern, or type annotation at `byte_offset`.
    pub fn type_at(&mut self, file: impl AsRef<Path>, byte_offset: u32) -> Option<LiveTy> {
        self.type_at_with_overlay(file, byte_offset, None)
    }

    /// Same as [`Self::type_at`], reading dirty/open file buffers from `overlays` when present.
    pub fn type_at_with_overlay(&mut self, file: impl AsRef<Path>, byte_offset: u32, overlays: Option<&OverlayManager>) -> Option<LiveTy> {
        let engine = self.engine.get_or_insert_with(|| LiveEngine::new(self.ws));
        let (r_idx, fid, canon) = engine.resolve_unit_and_file(file.as_ref(), &self.manual_overlays, overlays)?;
        let env_salt = engine.prepare_file_and_def_map(r_idx, fid, &canon, &self.manual_overlays, overlays)?;
        query_type_at(engine, r_idx, fid, &canon, env_salt, byte_offset)
    }

    /// Returns accessible fields and callable methods (inherent + traits in scope across autoderef)
    /// for the receiver expression before `.` at `byte_offset`, ordered as in `method_res`.
    pub fn dot_candidates(&mut self, file: impl AsRef<Path>, byte_offset: u32) -> Vec<Cand> {
        self.dot_candidates_with_overlay(file, byte_offset, None)
    }

    /// Same as [`Self::dot_candidates`], reading dirty/open file buffers from `overlays` when present.
    pub fn dot_candidates_with_overlay(&mut self, file: impl AsRef<Path>, byte_offset: u32, overlays: Option<&OverlayManager>) -> Vec<Cand> {
        let engine = self.engine.get_or_insert_with(|| LiveEngine::new(self.ws));
        let Some((r_idx, fid, canon)) = engine.resolve_unit_and_file(file.as_ref(), &self.manual_overlays, overlays) else {
            return Vec::new();
        };
        let Some(env_salt) = engine.prepare_file_and_def_map(r_idx, fid, &canon, &self.manual_overlays, overlays) else {
            return Vec::new();
        };
        query_dot_candidates(engine, r_idx, fid, &canon, env_salt, byte_offset)
    }

    /// Returns associated items, enum variants, or module items for a `Type::` or `module::` path at `byte_offset`.
    pub fn path_candidates(&mut self, file: impl AsRef<Path>, byte_offset: u32) -> Vec<Cand> {
        self.path_candidates_with_overlay(file, byte_offset, None)
    }

    /// Same as [`Self::path_candidates`], reading dirty/open file buffers from `overlays` when present.
    pub fn path_candidates_with_overlay(&mut self, file: impl AsRef<Path>, byte_offset: u32, overlays: Option<&OverlayManager>) -> Vec<Cand> {
        let engine = self.engine.get_or_insert_with(|| LiveEngine::new(self.ws));
        let Some((r_idx, fid, canon)) = engine.resolve_unit_and_file(file.as_ref(), &self.manual_overlays, overlays) else {
            return Vec::new();
        };
        let Some(env_salt) = engine.prepare_file_and_def_map(r_idx, fid, &canon, &self.manual_overlays, overlays) else {
            return Vec::new();
        };
        query_path_candidates(engine, r_idx, fid, &canon, env_salt, byte_offset)
    }

    /// Number of currently cached per-function inference entries (for diagnostics and tests).
    #[inline]
    pub fn cached_fn_count(&self) -> usize {
        self.engine.as_ref().map_or(0, |e| e.fn_cache.len())
    }
}
