/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! LSP hover intelligence handler.

use super::super::error::LspResult;
use super::super::protocol::{uri_to_path, PositionConverter};
use crate::overlay::OverlayManager;
use ridx_format::WorkspaceIndex;
use serde_json::{json, Value};

pub fn handle_hover(
    msg: &Value,
    workspace: Option<&WorkspaceIndex>,
    overlay_mgr: &OverlayManager,
    conv: &mut PositionConverter,
) -> LspResult<Value> {
    let (uri, line_0based, utf16_col_0based) = crate::lsp::params::text_document_position(msg)?;
    let line_1based = line_0based + 1;

    let file = uri_to_path(uri);
    let overlay_opt = overlay_mgr.get(&file);
    let col_1based = if let Some(overlay) = overlay_opt {
        overlay.utf16_col_to_char_col(line_1based, utf16_col_0based)
    } else {
        conv.utf16_col_to_char_col(&file, line_1based, utf16_col_0based)
    };

    // 1. Check workspace compiled hover and type-at for clean buffers
    if let Some(ws) = workspace {
        let is_dirty = overlay_opt.map(|o| o.is_dirty()).unwrap_or(false);
        if !is_dirty {
            if let Some(hover) = ws.hover_at(&file, line_1based, col_1based) {
                return Ok(format_hover(&hover));
            }
        }
    }

    // 2. Check overlay definition and inferred signature (for dirty buffers or when index has no info)
    if let Some(def) = overlay_mgr.find_definition_at(&file, line_1based, col_1based) {
        let hover_text = if let Some(sig) = def.signature {
            format!("```rust\n{}\n```", sig)
        } else {
            format!("```rust\n{} {}\n```", def.kind.as_str(), def.name)
        };
        return Ok(json!({
            "contents": {
                "kind": "markdown",
                "value": hover_text
            }
        }));
    }

    // 3. Fallback to mapped compiled hover if dirty buffer preserves valid mapping
    if let Some(ws) = workspace {
        if let Some(overlay) = overlay_opt {
            if overlay.is_dirty() && !overlay.has_modified_imports() {
                if let Some((ml, mc)) = overlay.map_live_to_compiled(line_1based, col_1based) {
                    if let Some(hover) = ws.hover_at(&file, ml, mc) {
                        return Ok(format_hover(&hover));
                    }
                }
            }
        }
    }

    Ok(Value::Null)
}

fn format_hover(hover: &ridx_format::WorkspaceHover) -> Value {
    let mut parts = Vec::new();
    if let Some(ref d) = hover.definition {
        if let Some(sig) = d.signature {
            parts.push(format_signature(d.name, sig));
        } else {
            parts.push(format!("{} {}", d.kind.as_str(), d.name));
        }
    }
    if let Some(t) = hover.inferred_type {
        if !parts.iter().any(|p| p.contains(t)) {
            parts.push(format!("Type: {}", t));
        }
    }
    let hover_text = format!("```rust\n{}\n```", parts.join("\n"));
    json!({
        "contents": {
            "kind": "markdown",
            "value": hover_text
        }
    })
}

fn format_signature(name: &str, sig: &str) -> String {
    let s = if let Some(stripped) = sig.strip_prefix("for<") {
        if let Some(idx) = stripped.find("> ") {
            &stripped[idx + 2..]
        } else {
            sig
        }
    } else {
        sig
    };

    if let Some(rest) = s.strip_prefix("fn(") {
        let mut decl = format!("fn {name}(");
        let normalized = rest
            .replacen("&'a mut Self", "&mut self", 1)
            .replacen("&mut Self", "&mut self", 1)
            .replacen("&'a Self", "&self", 1)
            .replacen("&Self", "&self", 1);
        decl.push_str(&normalized);
        decl
    } else {
        sig.to_string()
    }
}

