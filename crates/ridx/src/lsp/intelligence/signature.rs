/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! LSP signature help intelligence handler.

use super::super::error::{LspError, LspResult};
use super::super::protocol::{uri_to_path, PositionConverter};
use crate::overlay::OverlayManager;
use ridx_format::{DefKind, WorkspaceIndex};
use serde_json::{json, Value};

fn safe_slice(text: &str, raw_start: usize, raw_end: usize) -> &str {
    let mut start = raw_start.min(text.len());
    while start < text.len() && !text.is_char_boundary(start) {
        start += 1;
    }
    let mut end = raw_end.min(text.len());
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    if start <= end {
        &text[start..end]
    } else {
        ""
    }
}

pub fn handle_signature_help(
    msg: &Value,
    workspace: Option<&WorkspaceIndex>,
    overlay_mgr: &OverlayManager,
    conv: &mut PositionConverter,
) -> LspResult<Value> {
    let params = msg
        .get("params")
        .ok_or_else(|| LspError::invalid_params("Missing params object"))?;
    if !params.is_object() {
        return Err(LspError::invalid_params("Params must be an object"));
    }

    let uri = params["textDocument"]["uri"]
        .as_str()
        .ok_or_else(|| LspError::invalid_params("Missing textDocument.uri"))?;
    if uri.is_empty() {
        return Err(LspError::invalid_params("Empty textDocument.uri"));
    }

    let pos = &params["position"];
    if !pos.is_object() {
        return Err(LspError::invalid_params("Missing position object"));
    }
    let line_0based = pos["line"]
        .as_u64()
        .ok_or_else(|| LspError::invalid_params("Missing position.line"))? as u32;
    let utf16_col_0based = pos["character"]
        .as_u64()
        .ok_or_else(|| LspError::invalid_params("Missing position.character"))? as u32;
    let line_1based = line_0based + 1;

    let file = uri_to_path(uri);
    let overlay_opt = overlay_mgr.get(&file);
    let col_1based = if let Some(overlay) = overlay_opt {
        overlay.utf16_col_to_char_col(line_1based, utf16_col_0based)
    } else {
        conv.utf16_col_to_char_col(&file, line_1based, utf16_col_0based)
    };

    // Extract text preceding cursor across multiple lines if needed
    let text_preceding = if let Some(overlay) = overlay_opt {
        let byte_pos = overlay.line_char_col_to_byte(line_1based, col_1based);
        let start_byte = byte_pos.saturating_sub(8192);
        safe_slice(&overlay.text, start_byte, byte_pos).to_string()
    } else if let Ok(content) = std::fs::read_to_string(&file) {
        let mut byte_pos = 0;
        for (idx, line) in content.lines().enumerate() {
            if idx < line_0based as usize {
                byte_pos += line.len() + 1;
            } else if idx == line_0based as usize {
                let col_idx = col_1based.saturating_sub(1) as usize;
                let col_bytes = line
                    .chars()
                    .take(col_idx)
                    .map(|c| c.len_utf8())
                    .sum::<usize>();
                byte_pos += col_bytes;
                break;
            }
        }
        let start_byte = byte_pos.saturating_sub(8192);
        safe_slice(&content, start_byte, byte_pos).to_string()
    } else {
        return Ok(Value::Null);
    };

    let mut paren_depth = 0;
    let mut bracket_depth = 0;
    let mut brace_depth = 0;
    let mut angle_depth = 0;
    let mut active_param = 0;
    let mut open_paren_pos = None;

    let chars: Vec<char> = text_preceding.chars().collect();
    for (i, &c) in chars.iter().enumerate().rev() {
        match c {
            ')' => paren_depth += 1,
            '(' => {
                if paren_depth > 0 {
                    paren_depth -= 1;
                } else if bracket_depth == 0 && brace_depth == 0 {
                    open_paren_pos = Some(i);
                    break;
                }
            }
            ']' => bracket_depth += 1,
            '[' => {
                if bracket_depth > 0 {
                    bracket_depth -= 1;
                }
            }
            '}' => brace_depth += 1,
            '{' => {
                if brace_depth > 0 {
                    brace_depth -= 1;
                }
            }
            '>' => angle_depth += 1,
            '<' => {
                if angle_depth > 0 {
                    angle_depth -= 1;
                }
            }
            ',' if paren_depth == 0
                && bracket_depth == 0
                && brace_depth == 0
                && angle_depth == 0 =>
            {
                active_param += 1;
            }
            _ => {}
        }
    }

    let Some(call_start) = open_paren_pos else {
        return Ok(Value::Null);
    };

    let mut name_end = call_start;
    while name_end > 0 && chars[name_end - 1].is_whitespace() {
        name_end -= 1;
    }
    if name_end > 0 && chars[name_end - 1] == '>' {
        let mut depth = 1;
        name_end -= 1;
        while name_end > 0 && depth > 0 {
            name_end -= 1;
            if chars[name_end] == '>' {
                depth += 1;
            } else if chars[name_end] == '<' {
                depth -= 1;
            }
        }
        if name_end >= 2 && chars[name_end - 1] == ':' && chars[name_end - 2] == ':' {
            name_end -= 2;
        }
    }
    while name_end > 0 && chars[name_end - 1].is_whitespace() {
        name_end -= 1;
    }

    let mut name_start = name_end;
    while name_start > 0
        && (chars[name_start - 1] == '_' || chars[name_start - 1].is_alphanumeric())
    {
        name_start -= 1;
    }

    if name_start == name_end {
        return Ok(Value::Null);
    }

    let callee_name: String = chars[name_start..name_end].iter().collect();

    let mut sig_opt = None;
    if let Some(overlay) = overlay_opt {
        for s in &overlay.symbols {
            if s.name == callee_name && s.kind == DefKind::Fn {
                if let Some(ref sig) = s.signature {
                    sig_opt = Some(sig.clone());
                    break;
                }
            }
        }
    }

    if sig_opt.is_none() {
        if let Some(ws) = workspace {
            let syms = ws.search_symbols(&callee_name);
            for s in syms {
                if s.name == callee_name && matches!(s.kind, DefKind::Fn | DefKind::AssocFn) {
                    if let Some(sig) = s.signature {
                        sig_opt = Some(sig.to_string());
                        break;
                    }
                }
            }
        }
    }

    let Some(sig) = sig_opt else {
        return Ok(Value::Null);
    };

    let parameters: Vec<Value> = if let Some(start) = sig.find('(') {
        if let Some(end) = sig.rfind(')') {
            let inner = &sig[start + 1..end];
            let mut params = Vec::new();
            let mut cur_param = String::new();
            let mut depth = 0;
            for c in inner.chars() {
                match c {
                    '<' | '(' | '[' => {
                        depth += 1;
                        cur_param.push(c);
                    }
                    '>' | ')' | ']' => {
                        if depth > 0 {
                            depth -= 1;
                        }
                        cur_param.push(c);
                    }
                    ',' if depth == 0 => {
                        let trimmed = cur_param.trim();
                        if !trimmed.is_empty() {
                            params.push(json!({ "label": trimmed }));
                        }
                        cur_param.clear();
                    }
                    _ => cur_param.push(c),
                }
            }
            let trimmed = cur_param.trim();
            if !trimmed.is_empty() {
                params.push(json!({ "label": trimmed }));
            }
            params
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    Ok(json!({
        "signatures": [
            {
                "label": sig,
                "parameters": parameters
            }
        ],
        "activeSignature": 0,
        "activeParameter": active_param
    }))
}
