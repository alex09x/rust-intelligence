/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! The `ridx/*` extension methods answered in dispatch (docs/integration.md §7); `ridx/status` and `minVersion` are
//! answered before it (`fresh_loop`).

use super::error::{LspError, LspResult};
use super::protocol::{uri_to_path, PositionConverter};
use crate::overlay::OverlayManager;
use ridx_format::WorkspaceIndex;
use serde_json::Value;

/// The answer to the `ridx/*` request `msg` (method `method`).
pub fn dispatch(method: &str, msg: &Value, ws: Option<&WorkspaceIndex>, ov: &OverlayManager, conv: &mut PositionConverter) -> LspResult<Value> {
    match method {
        "ridx/safeDelete" => Ok(super::editing::safe_delete::handle_safe_delete(msg, ws, ov, conv)),
        "ridx/syntaxTree" => {
            let uri = msg["params"]["textDocument"]["uri"].as_str().ok_or_else(|| LspError::invalid_params("Missing textDocument.uri"))?;
            let file = uri_to_path(uri);
            let text = match ov.get(&file) {
                Some(o) => o.text.clone(),
                None => std::fs::read_to_string(&file).map_err(|e| LspError::invalid_params(format!("Cannot read {file}: {e}")))?,
            };
            let range = &msg["params"]["range"];
            let range = range.is_object().then(|| {
                let starts = super::editing::action_parse::line_starts_of(&text);
                let at = |p: &Value| super::editing::action_parse::utf16_pos_to_byte(&text, &starts, p["line"].as_u64()? as u32, p["character"].as_u64()? as u32);
                Some(at(&range["start"])?..at(&range["end"])?)
            });
            Ok(Value::String(super::syntax_tree::syntax_tree_json(&text, range.flatten())))
        }
        // `{name, expansion}` of the macro called at the position, or `null` when ridx cannot expand it fully.
        "ridx/expandMacro" => {
            let uri = msg["params"]["textDocument"]["uri"].as_str().ok_or_else(|| LspError::invalid_params("Missing textDocument.uri"))?;
            let file = uri_to_path(uri);
            let text = match ov.get(&file) {
                Some(o) => o.text.clone(),
                None => std::fs::read_to_string(&file).unwrap_or_default(),
            };
            let starts = super::editing::action_parse::line_starts_of(&text);
            let p = &msg["params"]["position"];
            let cursor = p["line"].as_u64().zip(p["character"].as_u64()).and_then(|(l, c)| super::editing::action_parse::utf16_pos_to_byte(&text, &starts, l as u32, c as u32));
            let expanded = cursor.zip(ws).and_then(|(at, ws)| super::editing::action_inline_macro::expand_macro_at(&text, &file, at, ws));
            Ok(expanded.map_or(Value::Null, |(name, expansion)| serde_json::json!({ "name": name, "expansion": expansion })))
        }
        _ => Err(LspError::method_not_found(format!("Method not found: {method}"))),
    }
}
