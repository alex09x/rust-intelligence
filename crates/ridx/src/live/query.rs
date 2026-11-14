/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Production LSP query API (`LiveDb`) owning only derived snapshots/tables/caches (`'static`, no
//! `&WorkspaceIndex` borrow retained across requests) and taking `&WorkspaceIndex` + optional
//! `&OverlayManager` per query (`Checkbox 4.2` / `4.3`).

use std::collections::HashMap;
use std::fmt;
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

/// The members offered after `receiver.` and, when the receiver's expression was found, its postfix facts.
#[derive(Clone, Debug, Default)]
pub struct DotCompletion {
    pub cands: Vec<Cand>,
    pub postfix: Option<super::query_postfix::PostfixFacts>,
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

/// Lifetime-free live type-inference and completion database owning only derived snapshots and caches.
///
/// Safe to store alongside `WorkspaceIndex` inside `MultiRootWorkspace` across `sync_reloaded_index(&mut ws, ..)`
/// mutations; automatically invalidates derived state if `ws` changes `instance_id`, `generation`, or shard count.
#[derive(Default)]
pub struct LiveDb {
    ws_stamp: Option<(u64, u64, usize)>,
    engine: Option<LiveEngine>,
    manual_overlays: HashMap<PathBuf, String>,
    /// The last dot completion, under the file, offset, unit, buffer signature and on-disk time it was made for:
    /// a request repeated on unchanged text (a re-trigger, a retried cancelled request) is answered from it.
    dot_memo: Option<(DotKey, DotCompletion)>,
}

type DotKey = (PathBuf, u32, usize, u64, Option<std::time::SystemTime>);

impl fmt::Debug for LiveDb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LiveDb").field("ws_stamp", &self.ws_stamp).field("initialized", &self.engine.is_some()).field("cached_fns", &self.cached_fn_count()).finish()
    }
}

impl LiveDb {
    /// Creates an empty lazy `LiveDb` without retaining any borrow of a `WorkspaceIndex`.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Lazy constructor matching `LiveDb::new()` (does not retain `ws` or materialize tables upfront).
    #[inline]
    pub fn open(_ws: &WorkspaceIndex) -> Self {
        Self::new()
    }

    /// Drops all derived snapshots, shard tables, file caches, and function inference caches.
    #[inline]
    pub fn invalidate(&mut self) {
        self.ws_stamp = None;
        self.engine = None;
        self.dot_memo = None;
    }

    fn sync_engine(&mut self, ws: &WorkspaceIndex) -> &mut LiveEngine {
        let stamp = (ws.instance_id(), ws.generation(), ws.crate_count());
        if self.ws_stamp != Some(stamp) {
            self.invalidate();
            self.ws_stamp = Some(stamp);
        }
        self.engine.get_or_insert_with(|| LiveEngine::new(ws))
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
    pub fn type_at(&mut self, ws: &WorkspaceIndex, file: impl AsRef<Path>, byte_offset: u32, overlays: Option<&OverlayManager>) -> Option<LiveTy> {
        self.sync_engine(ws);
        let engine = self.engine.as_mut()?;
        let (file_unit, fid, canon) = engine.locate_and_parse_file(ws, file.as_ref(), &self.manual_overlays, overlays)?;
        let r_idx = engine.unit_for_cursor(ws, file_unit, &canon, byte_offset);
        let on_typeable = {
            let st = engine.parsed.as_ref()?.tree();
            super::query_cursor::is_cursor_on_typeable_token(&st, byte_offset)
        };
        if !on_typeable {
            engine.ensure_unit_shards(ws, r_idx);
            return None;
        }
        let env_salt = engine.activate_unit_and_def_map(ws, r_idx, fid, &canon, &self.manual_overlays, overlays)?;
        query_type_at(ws, engine, r_idx, fid, &canon, env_salt, byte_offset)
    }

    /// Returns accessible fields and callable methods (inherent + traits in scope across autoderef)
    /// for the receiver expression before `.` at `byte_offset`, ordered as in `method_res`.
    pub fn dot_candidates(&mut self, ws: &WorkspaceIndex, file: impl AsRef<Path>, byte_offset: u32, overlays: Option<&OverlayManager>) -> Vec<Cand> {
        self.dot_completion(ws, file, byte_offset, overlays).cands
    }

    /// [`Self::dot_candidates`] together with what decides the postfix snippets at the same dot.
    pub fn dot_completion(&mut self, ws: &WorkspaceIndex, file: impl AsRef<Path>, byte_offset: u32, overlays: Option<&OverlayManager>) -> DotCompletion {
        self.sync_engine(ws);
        let Some(engine) = self.engine.as_mut() else { return DotCompletion::default() };
        let Some((file_unit, fid, canon)) = engine.locate_and_parse_file(ws, file.as_ref(), &self.manual_overlays, overlays) else {
            return DotCompletion::default();
        };
        let r_idx = engine.unit_for_cursor(ws, file_unit, &canon, byte_offset);
        let Some(env_salt) = engine.activate_unit_and_def_map(ws, r_idx, fid, &canon, &self.manual_overlays, overlays) else {
            return DotCompletion::default();
        };
        // A clean file is read from disk: its modification time keeps an edit made outside any buffer apart.
        let disk_time = (env_salt == 0).then(|| std::fs::metadata(&canon).and_then(|m| m.modified()).ok()).flatten();
        let key: DotKey = (canon.clone(), byte_offset, r_idx, env_salt, disk_time);
        if let Some((k, memo)) = &self.dot_memo {
            if *k == key {
                return memo.clone();
            }
        }
        let (cands, postfix) = query_dot_candidates(ws, engine, r_idx, fid, &canon, env_salt, byte_offset);
        let result = DotCompletion { cands, postfix };
        self.dot_memo = Some((key, result.clone()));
        result
    }

    /// Returns associated items, enum variants, or module items for a `Type::` or `module::` path at `byte_offset`.
    pub fn path_candidates(&mut self, ws: &WorkspaceIndex, file: impl AsRef<Path>, byte_offset: u32, overlays: Option<&OverlayManager>) -> Vec<Cand> {
        self.sync_engine(ws);
        let Some(engine) = self.engine.as_mut() else { return Vec::new() };
        let Some((r_idx, fid, canon)) = engine.locate_and_parse_file(ws, file.as_ref(), &self.manual_overlays, overlays) else {
            return Vec::new();
        };
        let Some(env_salt) = engine.activate_unit_and_def_map(ws, r_idx, fid, &canon, &self.manual_overlays, overlays) else {
            return Vec::new();
        };
        query_path_candidates(ws, engine, r_idx, fid, &canon, env_salt, byte_offset)
    }

    /// Number of currently cached per-function inference entries (for diagnostics and tests).
    #[inline]
    pub fn cached_fn_count(&self) -> usize {
        self.engine.as_ref().map_or(0, |e| e.fn_cache.len())
    }
}
