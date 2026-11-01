/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Signature help callee extraction and declaration formatting helpers.

use super::super::protocol::PositionConverter;
use super::hover::format_signature;
use super::hover_enrich::extract_source_decl_header;
use crate::overlay::OverlayManager;
use ridx_format::{DefKind, ResolvedDefinition, WorkspaceIndex};
use serde_json::{json, Value};

fn is_non_callee_keyword(w: &str) -> bool {
    matches!(
        w,
        "if" | "while"
            | "match"
            | "for"
            | "return"
            | "let"
            | "in"
            | "else"
            | "loop"
            | "where"
            | "type"
            | "fn"
            | "impl"
            | "trait"
            | "struct"
            | "enum"
            | "mod"
            | "unsafe"
            | "async"
            | "const"
    )
}

pub(crate) fn extract_callee_before_delim(chars: &[char], delim_pos: usize) -> Option<(usize, usize, bool)> {
    let mut name_end = delim_pos;
    while name_end > 0 && chars[name_end - 1].is_whitespace() {
        name_end -= 1;
    }
    if name_end > 0 && chars[name_end - 1] == '>' {
        let mut depth = 1i32;
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
    let mut is_macro = false;
    if name_end > 0 && chars[name_end - 1] == '!' {
        is_macro = true;
        name_end -= 1;
    }
    let mut name_start = name_end;
    while name_start > 0 && (chars[name_start - 1] == '_' || chars[name_start - 1].is_alphanumeric()) {
        name_start -= 1;
    }
    if name_start == name_end {
        return None;
    }
    let ident: String = chars[name_start..name_end].iter().collect();
    if is_non_callee_keyword(&ident) {
        return None;
    }
    Some((name_start, name_end, is_macro))
}

fn extract_struct_literal_header(def: &ResolvedDefinition<'_>, overlay_mgr: &OverlayManager, conv: &mut PositionConverter) -> Option<String> {
    if def.start_line == 0 || def.file.is_empty() || def.file == "external" {
        return None;
    }
    let resolved = def.resolved_path();
    let file_str = resolved.to_str().unwrap_or(def.file);
    let mut parts = Vec::new();
    let mut saw_open = false;
    let mut brace_depth = 0i32;
    for line_no in def.start_line..=(def.start_line + 30) {
        let line_opt = if let Some(ov) = overlay_mgr.get(file_str) {
            ov.text.lines().nth(line_no.saturating_sub(1) as usize).map(|s| s.to_string())
        } else {
            conv.get_line(file_str, line_no).map(|s| s.to_string())
        };
        let Some(raw) = line_opt else { break };
        let clean = raw.split("//").next().unwrap_or("").trim();
        if clean.is_empty() || clean.starts_with("#[") {
            continue;
        }
        for b in clean.bytes() {
            if b == b'{' {
                saw_open = true;
                brace_depth += 1;
            } else if b == b'}' {
                brace_depth -= 1;
            }
        }
        parts.push(clean.to_string());
        if saw_open && brace_depth <= 0 {
            break;
        }
    }
    if !saw_open {
        return None;
    }
    let joined = parts.join(" ").split_whitespace().collect::<Vec<_>>().join(" ");
    let prefix = if def.kind == DefKind::Variant && !joined.starts_with("enum ") && !joined.starts_with("struct ") {
        format!("enum {}::{}", def.path.rsplit_once("::").map(|(p, _)| p).unwrap_or(def.name), joined)
    } else {
        joined
    };
    Some(prefix)
}

pub(crate) fn format_resolved_def_sig(
    def: &ResolvedDefinition<'_>,
    callee_name: &str,
    is_brace_literal: bool,
    ws: &WorkspaceIndex,
    overlay_mgr: &OverlayManager,
    conv: &mut PositionConverter,
) -> Option<String> {
    if is_brace_literal {
        if !matches!(def.kind, DefKind::Struct | DefKind::Variant | DefKind::Union) {
            return None;
        }
        return extract_struct_literal_header(def, overlay_mgr, conv).or_else(|| Some(format!("struct {} {{ }}", def.name)));
    }
    if let Some(sig) = def.signature {
        if sig.starts_with(&format!("fn {callee_name}")) || sig.starts_with(&format!("pub fn {callee_name}")) {
            return Some(sig.to_string());
        }
    }
    if matches!(def.kind, DefKind::Fn | DefKind::AssocFn | DefKind::Struct | DefKind::StructCtor) {
        if let Some(hdr) = extract_source_decl_header(def, Some(ws), Some(overlay_mgr), conv) {
            if hdr.contains('(') && hdr.contains(')') {
                return Some(hdr);
            }
        }
    }
    if matches!(def.kind, DefKind::Variant | DefKind::VariantCtor) {
        if let Some(sig) = def.signature {
            let clean = sig.strip_prefix("variant ").unwrap_or(sig);
            if clean.contains('(') {
                return Some(format!("enum {clean}"));
            }
            return Some(format!("enum {clean}(_)"));
        }
        return Some(format!("enum {}(_)", def.name));
    }
    if matches!(def.kind, DefKind::Struct | DefKind::StructCtor) {
        return Some(format!("struct {}(_)", def.name));
    }
    if def.kind == DefKind::Macro {
        if matches!(callee_name, "assert_eq" | "assert_ne" | "debug_assert_eq" | "debug_assert_ne") {
            return Some(format!("fn {callee_name}(left, right)"));
        }
        return Some(format!("fn {callee_name}(...)"));
    }
    def.signature.map(|sig| format_signature(callee_name, sig))
}

pub(crate) fn is_self_receiver_param(param: &str) -> bool {
    let p = param.trim();
    matches!(p, "self" | "Self" | "&self" | "&mut self" | "mut self" | "&Self" | "&mut Self" | "self: Self" | "self: &Self" | "self: &mut Self")
        || p.starts_with("self:")
        || p.starts_with("mut self:")
        || (p.starts_with("&'") && (p.ends_with(" self") || p.ends_with(" Self")))
        || (!p.contains(':') && (p.ends_with(" self>") || p.ends_with(" Self>") || p.ends_with("&self>") || p.ends_with("&Self>")))
}

fn find_param_delimiters(sig: &str) -> Option<(usize, usize)> {
    let search_from = if let Some(idx) = sig.find("fn ") {
        idx + 3
    } else if let Some(rest) = sig.trim_start().strip_prefix("pub(") {
        let lead = sig.len() - sig.trim_start().len();
        rest.find(')').map(|close| lead + 4 + close + 1).unwrap_or(0)
    } else {
        0
    };
    let mut angle_depth = 0i32;
    let mut inner_paren_depth = 0i32;
    let mut prev_ch = '\0';
    let mut open_found = None;
    for (rel_idx, ch) in sig[search_from..].char_indices() {
        if angle_depth > 0 {
            if ch == '(' {
                inner_paren_depth += 1;
            } else if ch == ')' {
                inner_paren_depth = (inner_paren_depth - 1).max(0);
            } else if ch == '<' && inner_paren_depth == 0 {
                angle_depth += 1;
            } else if ch == '>' && inner_paren_depth == 0 && prev_ch != '-' && prev_ch != '=' {
                angle_depth -= 1;
            }
            prev_ch = ch;
            continue;
        }
        if ch == '<' {
            angle_depth = 1;
        } else if ch == '(' || ch == '{' {
            open_found = Some((search_from + rel_idx, ch, if ch == '(' { ')' } else { '}' }));
            break;
        }
        prev_ch = ch;
    }
    let (open_byte, open_ch, close_ch) = open_found?;
    let mut depth = 1i32;
    for (rel_idx, ch) in sig[open_byte + 1..].char_indices() {
        if ch == open_ch {
            depth += 1;
        } else if ch == close_ch {
            depth -= 1;
            if depth == 0 {
                return Some((open_byte, open_byte + 1 + rel_idx));
            }
        }
    }
    None
}

pub(crate) fn extract_parameters_from_sig(sig: &str, is_dot_call: bool) -> Vec<Value> {
    let Some((start, end)) = find_param_delimiters(sig) else {
        return Vec::new();
    };
    let inner = &sig[start + 1..end];
    let mut params = Vec::new();
    let mut cur_param = String::new();
    let mut depth = 0i32;
    let mut prev_c = '\0';
    for c in inner.chars() {
        match c {
            '<' | '(' | '[' | '{' => {
                depth += 1;
                cur_param.push(c);
            }
            '>' if prev_c == '-' || prev_c == '=' => {
                cur_param.push(c);
            }
            '>' | ')' | ']' | '}' => {
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
        prev_c = c;
    }
    let trimmed = cur_param.trim();
    if !trimmed.is_empty() {
        params.push(json!({ "label": trimmed }));
    }
    if is_dot_call && !params.is_empty() {
        if let Some(first_lbl) = params[0]["label"].as_str() {
            if is_self_receiver_param(first_lbl) {
                params.remove(0);
            }
        }
    }
    params
}
