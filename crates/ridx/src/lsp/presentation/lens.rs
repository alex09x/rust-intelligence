/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! LSP code lens generation and resolution.

use crate::lsp::protocol::uri_to_path;
use crate::overlay::OverlayManager;
use ridx_format::WorkspaceIndex;
use serde_json::{json, Value};

/// Handles `textDocument/codeLens` requests.
pub fn handle_code_lens(
    msg: &Value,
    workspace: Option<&WorkspaceIndex>,
    overlay_mgr: &OverlayManager,
) -> Value {
    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
    let file = uri_to_path(uri);

    let mut lenses = Vec::new();

    // 1. Overlay symbols code lenses
    if let Some(overlay) = overlay_mgr.get(&file) {
        for sym in &overlay.symbols {
            if sym.start_line == sym.end_line {
                let sl = sym.start_line.saturating_sub(1);
                let sc = overlay.char_col_to_utf16_col(sym.start_line, sym.start_col);
                let ec = overlay.char_col_to_utf16_col(sym.end_line, sym.end_col);

                let ref_count = if let Some(ws) = workspace {
                    ws.find_references_at(&file, sym.start_line, sym.start_col)
                        .map(|lr| lr.references.len())
                        .unwrap_or(0)
                } else {
                    0
                };

                let title = if ref_count > 0 {
                    format!("{ref_count} reference{}", if ref_count == 1 { "" } else { "s" })
                } else {
                    format!("References: {}", sym.name)
                };

                lenses.push(json!({
                    "range": {
                        "start": { "line": sl, "character": sc },
                        "end": { "line": sl, "character": ec }
                    },
                    "command": {
                        "title": title,
                        "command": "ridx.showReferences",
                        "arguments": [uri, { "line": sl, "character": sc }]
                    }
                }));
            }
        }
        return json!(lenses);
    }

    // 2. Fallback to WorkspaceIndex definitions if not in overlay
    if let Some(ws) = workspace {
        for r in ws.readers() {
            if let Some(fid) = r.find_file_id(&file) {
                for def in r.definitions() {
                    if def.span.file_id == fid {
                        let sl = def.span.start_line.saturating_sub(1);
                        let sc = def.span.start_col.saturating_sub(1);
                        let ec = def.span.end_col.saturating_sub(1);
                        let name = r.get_str(def.name_offset);
                        let ref_count = ws
                            .find_references_at(&file, def.span.start_line, def.span.start_col)
                            .map(|lr| lr.references.len())
                            .unwrap_or(0);

                        let title = if ref_count > 0 {
                            format!("{ref_count} reference{}", if ref_count == 1 { "" } else { "s" })
                        } else {
                            format!("References: {name}")
                        };

                        lenses.push(json!({
                            "range": {
                                "start": { "line": sl, "character": sc },
                                "end": { "line": sl, "character": ec }
                            },
                            "command": {
                                "title": title,
                                "command": "ridx.showReferences",
                                "arguments": [uri, { "line": sl, "character": sc }]
                            }
                        }));
                    }
                }
            }
        }
    }

    json!(lenses)
}

/// Handles `codeLens/resolve` requests.
pub fn handle_code_lens_resolve(msg: &Value) -> Value {
    msg.get("params").cloned().unwrap_or(Value::Null)
}
