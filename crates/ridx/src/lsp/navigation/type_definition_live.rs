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
//! (`Vec<Foo>` leads to `Vec`, `Foo` and the allocator), on saved and on edited buffers alike.

use super::type_definition_lookup::format_location;
use super::type_definition_walk::finalize_locations;
use crate::live::LiveDb;
use crate::lsp::protocol::PositionConverter;
use crate::overlay::OverlayManager;
use ridx_format::WorkspaceIndex;
use serde_json::Value;

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

/// The definitions of the types in the type inferred at the cursor, or `None` when nothing is inferred there.
pub(super) fn resolve_inferred_type(ws: &WorkspaceIndex, file: &str, (line, col): (u32, u32), overlay_mgr: &OverlayManager, conv: &mut PositionConverter) -> Option<Value> {
    let disk;
    let text = match overlay_mgr.get(file) {
        Some(overlay) => overlay.text.as_str(),
        None => {
            disk = std::fs::read_to_string(file).ok()?;
            disk.as_str()
        }
    };
    let offset = byte_offset_at(text, line, col)?;
    let ty = LiveDb::open(ws).type_at_with_overlay(file, offset, Some(overlay_mgr))?;
    let locs = ty.defs.iter().map(|d| format_location(&d.file, &d.file.to_string_lossy(), (d.line, d.col, d.end_line, d.end_col), conv)).collect();
    finalize_locations(locs)
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
