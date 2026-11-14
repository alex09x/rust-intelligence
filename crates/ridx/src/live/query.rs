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

/// Whether `line` is only a top-level import (`use …`, `pub use …`, `pub(in a::b) use …`), up to its `;` or open
/// on the next line: no expression on it has a type. `use a::B; fn f() { … }` holds a function, so it is not.
pub(crate) fn is_top_level_use_line(line: &str) -> bool {
    if line.starts_with([' ', '\t']) {
        return false;
    }
    let rest = line.strip_prefix("pub").map_or(line, |r| match r.trim_start().strip_prefix('(') {
        Some(inner) => inner.split_once(')').map_or("", |(_, after)| after),
        None if r.starts_with([' ', '\t']) => r,
        None => "",
    });
    let Some(tree) = rest.trim_start().strip_prefix("use ") else { return false };
    tree.find(';').is_none_or(|semi| tree[semi + 1..].trim().is_empty() || tree[semi + 1..].trim_start().starts_with("//"))
}

/// Resolved cursor type, candidate units on fallback, and 1-based `(col, line_text)`.
pub(crate) type CursorTypeResolution = (Option<LiveTy>, Option<Vec<(usize, u32)>>, Option<(u32, String)>);

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
    /// The last dot completion, under the file, offset, unit, signature salt and text hash it was made for:
    /// a request repeated on unchanged text (a re-trigger, a retried cancelled request) is answered from it.
    dot_memo: Option<(DotKey, DotCompletion)>,
}

type DotKey = (PathBuf, u32, usize, u64, u64);

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
        self.type_at_with_hint(ws, file.as_ref(), byte_offset, overlays, None)
    }

    pub(crate) fn type_at_with_hint(&mut self, ws: &WorkspaceIndex, file: &Path, byte_offset: u32, overlays: Option<&OverlayManager>, clean_hint: Option<&str>) -> Option<LiveTy> {
        self.sync_engine(ws);
        let engine = self.engine.as_mut()?;
        let (file_unit, fid, canon) = engine.locate_and_parse_file_with_hint(ws, file, &self.manual_overlays, overlays, clean_hint, Some(byte_offset))?;
        let r_idx = engine.unit_for_cursor(ws, file_unit, &canon, byte_offset);
        Self::type_at_after_parse(ws, engine, r_idx, fid, &canon, &self.manual_overlays, overlays, byte_offset)
    }

    #[allow(clippy::too_many_arguments)]
    fn type_at_after_parse(
        ws: &WorkspaceIndex,
        engine: &mut LiveEngine,
        r_idx: usize,
        fid: ridx_syntax::name_res::FileId,
        canon: &Path,
        manual: &HashMap<PathBuf, String>,
        overlays: Option<&OverlayManager>,
        byte_offset: u32,
    ) -> Option<LiveTy> {
        let can_infer = {
            let db = super::v5_db::RidxV5ItemDb::view(ws, &engine.src, &engine.state);
            let parsed = engine.parsed.as_ref()?;
            let st = parsed.tree();
            super::query_cursor::is_cursor_on_typeable_token(&st, byte_offset) && !super::query_cfg::is_inside_inactive_target_cfg(&st, byte_offset, &db, r_idx, canon) && {
                let macro_tt = super::query_cursor::enclosing_macro_tt_span(&st, byte_offset);
                super::query_loc::find_enclosing_fn_summary(&st, &parsed.fn_summaries, byte_offset).is_some()
                    || super::query_cursor::smallest_type_node_at(&st, byte_offset, macro_tt).is_some()
            }
        };
        if !can_infer {
            return None;
        }
        let env_salt = engine.activate_unit_and_def_map_at(ws, r_idx, fid, canon, manual, overlays, Some(byte_offset))?;
        query_type_at(ws, engine, r_idx, fid, canon, env_salt, byte_offset)
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
        // The signature salt covers edits to other items; the parsed text's hash covers an edit inside a body.
        // A clean file's text is the one the engine loaded once (`u64::MAX`), which no request changes.
        let text_hash = engine.parsed.as_ref().map_or(0, |p| p.text_hash);
        let key: DotKey = (canon.clone(), byte_offset, r_idx, env_salt, text_hash);
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

    #[inline]
    pub(crate) fn units_for_file(&mut self, ws: &WorkspaceIndex, file: &Path) -> Option<Vec<(usize, u32)>> {
        let engine = self.sync_engine(ws);
        if let Some(v) = engine.state.units_by_file.get(file) {
            return Some(v.clone());
        }
        if engine.state.units_complete
            && engine.src.canon_root == engine.src.ws_root
            && file.starts_with(&engine.src.ws_root)
            && ws.readers().iter().any(|r| r.header().files_count > 1)
        {
            return Some(Vec::new());
        }
        None
    }

    /// Resolved cursor type, candidate units on fallback, and 1-based column plus raw line text.
    pub(crate) fn resolve_pos_and_type_at(&mut self, ws: &WorkspaceIndex, file: &Path, line_1: u32, utf16_col_0: u32, overlays: Option<&OverlayManager>) -> CursorTypeResolution {
        self.sync_engine(ws);
        if !self.manual_overlays.is_empty() || overlays.is_some_and(|o| o.open_count() != 0) {
            return (None, None, None);
        }
        let Some((file_unit, fid, canon)) = self.engine.as_mut().and_then(|e| e.resolve_file_entry(ws, file, &self.manual_overlays, overlays, None)) else {
            return (None, self.units_for_file(ws, file), None);
        };
        let (col_1, skip_live, byte_offset) = {
            let cf = &self.engine.as_ref().unwrap().src.files[fid.0 as usize];
            let Some((line_start, line_text)) = cf.lines.line_str(&cf.text, line_1) else {
                return (None, self.units_for_file(ws, file), None);
            };
            let (col_1, in_line) = if cf.lines.is_ascii() {
                let b = (utf16_col_0 as usize).min(line_text.len());
                (b as u32 + 1, b)
            } else {
                let (mut u16_acc, mut char_col, mut byte_idx) = (0u32, 1u32, line_text.len());
                for (i, ch) in line_text.char_indices() {
                    if u16_acc >= utf16_col_0 {
                        byte_idx = i;
                        break;
                    }
                    u16_acc += ch.len_utf16() as u32;
                    char_col += 1;
                }
                (char_col, byte_idx)
            };
            let trimmed = line_text.trim_start();
            let is_use = is_top_level_use_line(line_text);
            let lb = line_text.as_bytes();
            let is_id = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'#' || b >= 0x80;
            let on_ident = lb.get(in_line).copied().is_some_and(is_id) || in_line.checked_sub(1).and_then(|i| lb.get(i).copied()).is_some_and(is_id);
            let skip = (is_use || trimmed.starts_with("//") || !on_ident).then(|| line_text.to_string());
            (col_1, skip, (line_start + in_line) as u32)
        };
        if let Some(l_str) = skip_live {
            return (None, self.units_for_file(ws, file), Some((col_1, l_str)));
        }
        let ty = {
            let engine = self.engine.as_mut().unwrap();
            engine.parse_resolved_file(ws, file_unit, fid, &canon, &self.manual_overlays, overlays, Some(byte_offset));
            let r_idx = engine.unit_for_cursor(ws, file_unit, &canon, byte_offset);
            Self::type_at_after_parse(ws, engine, r_idx, fid, &canon, &self.manual_overlays, overlays, byte_offset)
        };
        let need_fallback = ty.as_ref().is_none_or(|t| t.display.contains("{unknown}") || t.defs.is_empty());
        if !need_fallback {
            return (ty, None, Some((col_1, String::new())));
        }
        let l_str = {
            let cf = &self.engine.as_ref().unwrap().src.files[fid.0 as usize];
            cf.lines.line_str(&cf.text, line_1).map_or_else(String::new, |(_, s)| s.to_string())
        };
        (ty, self.units_for_file(ws, file), Some((col_1, l_str)))
    }
}
