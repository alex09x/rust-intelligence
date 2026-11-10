/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! `textDocument/typeDefinition` from the type inferred at the cursor: every type the inferred type names
//! (`Vec<Foo>` leads to `Vec`, `Foo` and the allocator), on saved and on edited buffers alike, backed by a
//! single process-wide [`LiveDb`].

use std::sync::{Mutex, OnceLock};

use super::type_definition_lookup::format_location;
use super::type_definition_walk::finalize_locations;
use crate::live::LiveDb;
use crate::lsp::protocol::PositionConverter;
use crate::overlay::OverlayManager;
use ridx_format::WorkspaceIndex;
use serde_json::Value;

static LIVE_DB: OnceLock<Mutex<LiveDb>> = OnceLock::new();

#[inline]
fn global_live_db() -> &'static Mutex<LiveDb> {
    LIVE_DB.get_or_init(|| Mutex::new(LiveDb::new()))
}

/// Drops all derived snapshots, shard tables, and inference caches in the process-wide [`LiveDb`].
pub(crate) fn clear_live_db_cache() {
    if let Some(lock) = LIVE_DB.get() {
        if let Ok(mut db) = lock.lock() {
            db.invalidate();
        }
    }
}

/// The byte offset of the 1-based `line` and character column `col` in `text`.
pub(crate) fn byte_offset_at(text: &str, line: u32, col: u32) -> Option<u32> {
    let mut start = 0usize;
    for _ in 1..line {
        start += text[start..].find('\n')? + 1;
    }
    let line_text = text[start..].split('\n').next().unwrap_or("");
    let in_line = line_text.char_indices().nth(col.saturating_sub(1) as usize).map_or(line_text.len(), |(i, _)| i);
    u32::try_from(start + in_line).ok()
}

fn byte_offset_from_conv_or_overlay(file: &str, line: u32, col: u32, overlay_mgr: &OverlayManager, conv: &mut PositionConverter) -> Option<u32> {
    if let Some(overlay) = overlay_mgr.get(file) {
        return byte_offset_at(&overlay.text, line, col);
    }
    let line_text = conv.get_line(file, line)?;
    let in_line = line_text.char_indices().nth(col.saturating_sub(1) as usize).map_or(line_text.len(), |(i, _)| i);
    let (_, starts) = conv.cache.get(file)?;
    let start = *starts.get(line.saturating_sub(1) as usize)?;
    u32::try_from(start + in_line).ok()
}

fn is_file_compiled_in_ws(ws: &WorkspaceIndex, file: &str) -> bool {
    let check = |idx: usize| ws.target_file_for_reader(idx, file).and_then(|tf| ws.readers()[idx].find_file_id(tf.as_ref())).is_some();
    ws.reader_indices_for_file(file).into_iter().any(check) || ws.candidate_reader_indices_for_file(file).into_iter().any(check)
}

/// The definitions of the types in the type inferred at the cursor (plus whether inference was complete without `{unknown}`), or `None`.
pub(super) fn resolve_inferred_type(ws: &WorkspaceIndex, file: &str, (line, col): (u32, u32), overlay_mgr: &OverlayManager, conv: &mut PositionConverter) -> Option<(Value, bool)> {
    let is_dirty = overlay_mgr.get(file).is_some_and(|o| o.is_dirty());
    if !is_dirty && !is_file_compiled_in_ws(ws, file) {
        return None;
    }
    let offset = byte_offset_from_conv_or_overlay(file, line, col, overlay_mgr, conv)?;
    let ty = {
        let mut db = global_live_db().lock().unwrap_or_else(|e| e.into_inner());
        db.type_at(ws, file, offset, Some(overlay_mgr))?
    };
    let complete = is_dirty || !ty.display.contains("{unknown}");
    let locs = ty.defs.iter().map(|d| format_location(&d.file, &d.file.to_string_lossy(), (d.line, d.col, d.end_line, d.end_col), conv)).collect();
    finalize_locations(locs).map(|v| (v, complete))
}

#[cfg(test)]
mod tests {
    use super::byte_offset_at;

    #[test]
    fn test_byte_offset_at_counts_characters_not_bytes() {
        let text = "let a = 1;\nlet é = b;\n";
        assert_eq!(byte_offset_at(text, 1, 1), Some(0));
        assert_eq!(byte_offset_at(text, 2, 5), Some(15));
        // `é` is two bytes: the column after it is one character, two bytes further.
        assert_eq!(byte_offset_at(text, 2, 6), Some(17));
        assert_eq!(byte_offset_at(text, 2, 99), Some(22));
        assert_eq!(byte_offset_at(text, 9, 1), None);
    }
}
