/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Index-backed workspace `Inline into all callers` (`refactor.inline`).

use super::action_generics_sub::split_top_level_commas;
use super::action_inline_callers_helpers::{
    contains_recursive_call, find_path_start, find_receiver_start, format_inlined_call, has_only_lifetime_bounds, has_only_lifetime_generics, parse_fn_params,
    rewrite_self_to_this, FnParamInfo, SelfKind,
};
use super::action_inline_local::line_deletion_span;
use super::action_parse::{ItemKindInfo, ParsedCursorItem};
use super::action_parse_items::{contains_word, skip_balanced};
use super::action_struct_conv_helpers::to_lsp_edits;
use super::action_struct_conv_index::byte_to_line_col;
use ridx_format::WorkspaceIndex;
use serde_json::Value;
use std::collections::BTreeMap;

pub type InlineEdits = (Vec<Value>, BTreeMap<String, Vec<Value>>);

pub(crate) fn line_col_to_byte(doc: &str, starts: &[usize], line_1b: u32, col_1b: u32) -> Option<usize> {
    let start = *starts.get(line_1b.checked_sub(1)? as usize)?;
    let line = doc[start..].split('\n').next().unwrap_or("");
    let n = col_1b.checked_sub(1)? as usize;
    if n == line.chars().count() {
        return Some(start + line.len());
    }
    line.char_indices().nth(n).map(|(b, _)| start + b)
}

pub fn build_inline_into_all_callers_workspace(
    doc_text: &str,
    masked_doc: &str,
    starts: &[usize],
    file: &str,
    item: &ParsedCursorItem,
    ws: Option<&WorkspaceIndex>,
) -> Option<InlineEdits> {
    if !item.cfg_attrs.is_empty() || !item.vis_prefix.is_empty() {
        return None;
    }
    match &item.enclosing {
        super::action_parse::EnclosingContext::InherentImpl { cfg_attrs, .. } => {
            if !cfg_attrs.is_empty() {
                return None;
            }
        }
        super::action_parse::EnclosingContext::None => {}
        _ => return None,
    }
    let ItemKindInfo::Fn { is_unsafe: false, extern_abi: None, generics_src, params_src, ret_src, where_src, has_body: true, body_range: Some((bo, bc)) } = &item.kind else {
        return None;
    };
    if !has_only_lifetime_generics(generics_src) || !has_only_lifetime_bounds(where_src) {
        return None;
    }
    if ret_src.as_deref().is_some_and(|r| r.contains("Result") || r.contains("impl ")) {
        return None;
    }
    let hdr_m = &masked_doc[item.decl_start_byte..*bo];
    if contains_word(hdr_m, "async") || contains_word(hdr_m, "const") {
        return None;
    }
    let body_inner_m = &masked_doc[*bo + 1..*bc - 1];
    if contains_word(body_inner_m, "return")
        || body_inner_m.contains('?')
        || contains_word(body_inner_m, "super")
        || contains_word(body_inner_m, "Self")
        || body_inner_m.contains('$')
        || contains_recursive_call(body_inner_m, &item.name)
    {
        return None;
    }

    let parsed_params = parse_fn_params(params_src)?;
    let self_param = parsed_params.iter().find(|p| p.is_self);
    let self_kind = self_param.and_then(|p| p.self_kind);

    let raw_body = &doc_text[*bo + 1..*bc - 1];
    let raw_masked_body = &masked_doc[*bo + 1..*bc - 1];
    let body_transformed = if self_param.is_some() {
        rewrite_self_to_this(raw_body, raw_masked_body).trim().to_string()
    } else {
        if contains_word(body_inner_m, "self") {
            return None;
        }
        raw_body.trim().to_string()
    };

    let ws = ws?;
    let (decl_line, decl_col) = byte_to_line_col(doc_text, starts, item.name_byte);
    let refs_obj = ws.find_direct_references_at(file, decl_line, decl_col)?;
    if refs_obj.references.is_empty() {
        return None;
    }

    let (del_lo, del_hi) = line_deletion_span(doc_text, item.first_attr_byte, *bc);
    let mut local_edits = Vec::new();
    let canonical_file = std::fs::canonicalize(file).ok();
    let def_lo = item.first_attr_byte.min(item.decl_start_byte);
    let mut any_skipped = false;

    for r in &refs_obj.references {
        let canon = std::fs::canonicalize(r.resolved_path()).ok();
        let is_same_file = r.file == file || (canonical_file.is_some() && canon == canonical_file);
        if !is_same_file {
            return None;
        }
        let span_line = r.span.start_line();
        let span_col = r.span.start_col();

        let ref_byte = line_col_to_byte(doc_text, starts, span_line, span_col)?;
        if ref_byte >= def_lo && ref_byte < *bc {
            continue;
        }
        if let Some((site_start, site_end, inlined_text)) = process_call_site(doc_text, masked_doc, ref_byte, &item.name, self_kind, &parsed_params, &body_transformed) {
            local_edits.push((site_start, site_end, inlined_text));
        } else {
            any_skipped = true;
        }
    }

    if local_edits.is_empty() {
        return None;
    }

    if !any_skipped {
        local_edits.push((del_lo, del_hi, String::new()));
    }
    let local_lsp_edits = to_lsp_edits(doc_text, starts, local_edits)?;

    Some((local_lsp_edits, BTreeMap::new()))
}

fn process_call_site(src: &str, masked: &str, ref_byte: usize, name: &str, self_kind: Option<SelfKind>, params: &[FnParamInfo], body_src: &str) -> Option<(usize, usize, String)> {
    let mb = masked.as_bytes();
    let name_end = ref_byte + name.len();
    if name_end > src.len() || &src[ref_byte..name_end] != name {
        return None;
    }

    let mut p = name_end;
    while p < src.len() && mb[p].is_ascii_whitespace() {
        p += 1;
    }
    if p >= src.len() || mb[p] != b'(' {
        return None;
    }
    let call_end = skip_balanced(mb, p, b'(', b')')?;
    let raw_args: Vec<String> = split_top_level_commas(&src[p + 1..call_end - 1]).into_iter().filter(|a| !a.trim().is_empty()).collect();

    let (call_start, recv_expr) = if self_kind.is_some() {
        let before_dot = src[..ref_byte].trim_end();
        if !before_dot.ends_with('.') || before_dot.ends_with("..") {
            return None;
        }
        let dot_byte = before_dot.len() - 1;
        let recv_start = find_receiver_start(src, masked, dot_byte)?;
        let recv = src[recv_start..dot_byte].trim();
        if recv.is_empty() || recv.starts_with('.') || recv.ends_with('.') || recv.contains('(') || recv.contains('[') {
            return None;
        }
        (recv_start, Some(recv))
    } else {
        let path_start = find_path_start(masked, ref_byte);
        (path_start, None)
    };

    let non_self_count = params.iter().filter(|p| !p.is_self).count();
    if raw_args.len() != non_self_count || raw_args.iter().any(|a| a.contains('(')) {
        return None;
    }

    let prev_ch = masked[..call_start].trim_end().as_bytes().last().copied().unwrap_or(b'{');
    let next_ch = masked[call_end..].trim_start().as_bytes().first().copied().unwrap_or(b'}');
    let standalone = matches!(prev_ch, b'{' | b';') && matches!(next_ch, b';' | b'}');

    let inlined = format_inlined_call(recv_expr, self_kind, params, &raw_args, body_src, standalone);
    Some((call_start, call_end, inlined))
}
