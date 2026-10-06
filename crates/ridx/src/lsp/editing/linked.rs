/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Linked editing range handler for simultaneous identifier editing.

use crate::lsp::protocol::{uri_to_path, PositionConverter};
use crate::overlay::OverlayManager;
use serde_json::{json, Value};

pub fn handle_linked_editing_range(
    msg: &Value,
    overlay_mgr: &OverlayManager,
    conv: &mut PositionConverter,
) -> Value {
    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
    let file = uri_to_path(uri);
    let line_0based = msg["params"]["position"]["line"].as_u64().unwrap_or(0) as u32;
    let utf16_col_0based = msg["params"]["position"]["character"].as_u64().unwrap_or(0) as u32;
    let line_1based = line_0based + 1;

    let overlay_opt = overlay_mgr.get(&file);
    let col_1based = if let Some(overlay) = overlay_opt {
        overlay.utf16_col_to_char_col(line_1based, utf16_col_0based)
    } else {
        conv.utf16_col_to_char_col(&file, line_1based, utf16_col_0based)
    };

    if let Some(overlay) = overlay_opt {
        let refs = overlay.find_local_binding_references(line_1based, col_1based);
        if !refs.is_empty() {
            let mut ranges = Vec::new();
            for (sl, sc, el, ec) in refs {
                let start_char = overlay.char_col_to_utf16_col(sl, sc);
                let end_char = overlay.char_col_to_utf16_col(el, ec);
                ranges.push(json!({
                    "start": { "line": sl.saturating_sub(1), "character": start_char },
                    "end": { "line": el.saturating_sub(1), "character": end_char }
                }));
            }
            return json!({ "ranges": ranges });
        }
    }

    Value::Null
}
