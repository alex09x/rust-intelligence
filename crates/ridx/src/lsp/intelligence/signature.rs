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

use super::super::error::LspResult;
use super::super::protocol::{uri_to_path, PositionConverter};
use super::hover::format_signature;
use super::signature_fmt::{extract_callee_before_delim, extract_parameters_from_sig, format_resolved_def_sig};
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

pub fn handle_signature_help(msg: &Value, workspace: Option<&WorkspaceIndex>, overlay_mgr: &OverlayManager, conv: &mut PositionConverter) -> LspResult<Value> {
    let (uri, line_0based, utf16_col_0based) = crate::lsp::params::text_document_position(msg)?;
    let line_1based = line_0based + 1;

    let file = uri_to_path(uri);
    let overlay_opt = overlay_mgr.get(&file);
    let col_1based = if let Some(overlay) = overlay_opt {
        overlay.utf16_col_to_char_col(line_1based, utf16_col_0based)
    } else {
        conv.utf16_col_to_char_col(&file, line_1based, utf16_col_0based)
    };

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
                let col_bytes = line.chars().take(col_idx).map(|c| c.len_utf8()).sum::<usize>();
                byte_pos += col_bytes;
                break;
            }
        }
        let start_byte = byte_pos.saturating_sub(8192);
        safe_slice(&content, start_byte, byte_pos).to_string()
    } else {
        return Ok(Value::Null);
    };

    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut brace_depth = 0usize;
    let mut angle_depth = 0usize;
    let mut active_param = 0usize;
    let mut found_call: Option<(usize, usize, usize, bool, bool)> = None;

    let chars: Vec<char> = text_preceding.chars().collect();
    for (i, &c) in chars.iter().enumerate().rev() {
        match c {
            ')' => paren_depth += 1,
            '(' => {
                if paren_depth > 0 {
                    paren_depth -= 1;
                } else if bracket_depth == 0 && brace_depth == 0 {
                    if let Some((ns, ne, is_macro)) = extract_callee_before_delim(&chars, i) {
                        found_call = Some((i, ns, ne, false, is_macro));
                        break;
                    }
                    active_param = 0;
                    angle_depth = 0;
                }
            }
            ']' => bracket_depth += 1,
            '[' => {
                if bracket_depth > 0 {
                    bracket_depth -= 1;
                } else if paren_depth == 0 && brace_depth == 0 {
                    active_param = 0;
                    angle_depth = 0;
                }
            }
            '}' => brace_depth += 1,
            '{' => {
                if brace_depth > 0 {
                    brace_depth -= 1;
                } else if paren_depth == 0 && bracket_depth == 0 {
                    if let Some((ns, ne, false)) = extract_callee_before_delim(&chars, i) {
                        found_call = Some((i, ns, ne, true, false));
                    }
                    break;
                } else {
                    break;
                }
            }
            '>' => angle_depth += 1,
            '<' => {
                angle_depth = angle_depth.saturating_sub(1);
            }
            ',' if paren_depth == 0 && bracket_depth == 0 && brace_depth == 0 && angle_depth == 0 => {
                active_param += 1;
            }
            _ => {}
        }
    }

    let Some((_call_start, name_start, name_end, is_brace_literal, is_macro)) = found_call else {
        return Ok(Value::Null);
    };

    let callee_name: String = chars[name_start..name_end].iter().collect();
    let mut qual_end = name_start;
    while qual_end > 0 && chars[qual_end - 1].is_whitespace() {
        qual_end -= 1;
    }
    let is_colon_call = qual_end >= 2 && chars[qual_end - 1] == ':' && chars[qual_end - 2] == ':';
    let is_dot_call = !is_brace_literal && qual_end >= 1 && chars[qual_end - 1] == '.';

    let mut sig_opt = None;

    if is_macro && matches!(callee_name.as_str(), "assert_eq" | "assert_ne" | "debug_assert_eq" | "debug_assert_ne") {
        sig_opt = Some(format!("fn {callee_name}(left, right)"));
    }

    // 1. Try resolving exact call-site definition from compiled index
    if sig_opt.is_none() {
        if let Some(ws) = workspace {
            let newlines_between = chars[name_start..].iter().filter(|&&c| c == '\n').count() as u32;
            let callee_line_1based = line_1based.saturating_sub(newlines_between);
            let last_nl = chars[..name_start].iter().rposition(|&c| c == '\n');
            let callee_col_1based = if let Some(nl_pos) = last_nl {
                (name_start - nl_pos) as u32
            } else {
                let chars_from_cursor = chars.len() - name_start;
                col_1based.saturating_sub(chars_from_cursor as u32)
            };

            for probe_col in [callee_col_1based, callee_col_1based.saturating_add(1)] {
                if let Some(def) = ws.find_definition_at(&file, callee_line_1based, probe_col) {
                    if def.name == callee_name {
                        sig_opt = format_resolved_def_sig(&def, &callee_name, is_brace_literal, ws, overlay_mgr, conv);
                        if sig_opt.is_some() {
                            break;
                        }
                    }
                }
            }
        }
    }

    // 2. Check for qualified call (e.g. ExternalCoreService::new or receiver.method)
    if sig_opt.is_none() && (is_colon_call || is_dot_call) {
        let qual_sep_len = if is_colon_call { 2 } else { 1 };
        let mut qual_tok_end = qual_end - qual_sep_len;
        while qual_tok_end > 0 && chars[qual_tok_end - 1].is_whitespace() {
            qual_tok_end -= 1;
        }
        let mut qual_tok_start = qual_tok_end;
        while qual_tok_start > 0 && (chars[qual_tok_start - 1] == '_' || chars[qual_tok_start - 1].is_alphanumeric() || chars[qual_tok_start - 1] == ':') {
            qual_tok_start -= 1;
        }
        if qual_tok_start < qual_tok_end {
            let qualifier: String = chars[qual_tok_start..qual_tok_end].iter().collect();
            if let Some(ws) = workspace {
                let def_opt = ws.find_definition_by_path(&qualifier);
                let target_path = def_opt.as_ref().map(|d| d.path).unwrap_or(&qualifier);
                let method_path = format!("{target_path}::{callee_name}");
                if let Some(method_def) = ws.find_definition_by_path(&method_path) {
                    sig_opt = format_resolved_def_sig(&method_def, &callee_name, is_brace_literal, ws, overlay_mgr, conv);
                }
            }
        }
    }

    // 3. Fallback to overlay symbols
    if sig_opt.is_none() && !is_brace_literal {
        if let Some(overlay) = overlay_opt {
            for s in &overlay.symbols {
                if s.name == callee_name && s.kind == DefKind::Fn {
                    if let Some(ref sig) = s.signature {
                        sig_opt = Some(format_signature(&callee_name, sig));
                        break;
                    }
                }
            }
        }
    }

    // 4. Fallback to workspace symbol search
    if sig_opt.is_none() {
        if let Some(ws) = workspace {
            for s in ws.search_symbols(&callee_name) {
                if s.name != callee_name {
                    continue;
                }
                if is_brace_literal {
                    if matches!(s.kind, DefKind::Struct | DefKind::Variant | DefKind::Union) {
                        sig_opt = Some(format!("struct {callee_name} {{ }}"));
                        break;
                    }
                } else if matches!(s.kind, DefKind::Fn | DefKind::AssocFn | DefKind::Variant | DefKind::VariantCtor | DefKind::StructCtor) {
                    if let Some(sig) = s.signature {
                        let clean = sig.strip_prefix("variant ").unwrap_or(sig);
                        sig_opt = Some(if matches!(s.kind, DefKind::Variant | DefKind::VariantCtor) {
                            if clean.contains('(') {
                                format!("enum {clean}")
                            } else {
                                format!("enum {clean}(_)")
                            }
                        } else {
                            format_signature(&callee_name, sig)
                        });
                        break;
                    }
                }
            }
        }
    }

    let Some(sig) = sig_opt else {
        return Ok(Value::Null);
    };

    let parameters = extract_parameters_from_sig(&sig, is_dot_call);

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
