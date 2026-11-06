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
use super::action_inline_callers_helpers::{contains_recursive_call, find_path_start, find_receiver_start, format_inlined_call, parse_fn_params, FnParamInfo, SelfKind};
use super::action_inline_callers_transform::{has_only_lifetime_bounds, has_only_lifetime_generics, rewrite_self_to_this, rewrite_self_type_to_name, unwrap_tail_return};
use super::action_inline_local::line_deletion_span;
use super::action_parse::{mask_comments_and_strings, ItemKindInfo, ParsedCursorItem};
use super::action_parse_items::{contains_word, skip_balanced};
use super::action_struct_conv_helpers::to_lsp_edits;
use super::action_struct_conv_index::byte_to_line_col;
use ridx_format::WorkspaceIndex;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

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
    let vis = item.vis_prefix.trim();
    if !item.cfg_attrs.is_empty() || (!vis.is_empty() && !vis.starts_with("pub")) {
        return None;
    }
    let is_pub = vis.starts_with("pub");
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
    let self_type_name = match &item.enclosing {
        super::action_parse::EnclosingContext::InherentImpl { self_ty, generics_src, .. } => {
            if has_only_lifetime_generics(generics_src) {
                let base_raw = self_ty.split('<').next().unwrap_or(self_ty).trim().rsplit("::").next().unwrap_or("").trim();
                let base_name = base_raw.strip_prefix("r#").unwrap_or(base_raw);
                if !base_name.is_empty() && base_name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                    Some(base_name.to_string())
                } else {
                    None
                }
            } else {
                None
            }
        }
        _ => None,
    };
    let effective_ret_ty = if let Some(r) = ret_src.as_deref() {
        if r.contains("impl ") || r.starts_with('&') || r.contains('\'') {
            return None;
        }
        if r == "Self" {
            Some(self_type_name.as_ref()?.clone())
        } else if r.contains("Self") {
            return None;
        } else {
            Some(r.to_string())
        }
    } else {
        None
    };

    let hdr_m = &masked_doc[item.decl_start_byte..*bo];
    if contains_word(hdr_m, "async") || contains_word(hdr_m, "const") {
        return None;
    }
    let body_inner_m = &masked_doc[*bo + 1..*bc - 1];
    let raw_body_slice = &doc_text[*bo + 1..*bc - 1];
    let (raw_body, raw_masked_body) = if contains_word(body_inner_m, "return") {
        if let Some((ub, um)) = unwrap_tail_return(raw_body_slice, body_inner_m) {
            (ub, um)
        } else {
            return None;
        }
    } else {
        (raw_body_slice.to_string(), body_inner_m.to_string())
    };

    if raw_masked_body.contains('?')
        || contains_word(&raw_masked_body, "super")
        || raw_masked_body.contains('$')
        || (contains_word(&raw_masked_body, "Self") && self_type_name.is_none())
        || contains_recursive_call(&raw_masked_body, &item.name)
    {
        return None;
    }

    let parsed_params = parse_fn_params(params_src)?;
    let self_param = parsed_params.iter().find(|p| p.is_self);
    let self_kind = self_param.and_then(|p| p.self_kind);

    if is_pub && (self_kind == Some(SelfKind::MutRef) || self_kind == Some(SelfKind::Value) || item.name == "new") {
        return None;
    }

    let body_with_self_ty = if let Some(ty_name) = &self_type_name {
        if contains_word(&raw_masked_body, "Self") {
            rewrite_self_type_to_name(&raw_body, &raw_masked_body, ty_name)
        } else {
            raw_body
        }
    } else {
        raw_body
    };
    let masked_with_self_ty = if contains_word(&raw_masked_body, "Self") { mask_comments_and_strings(&body_with_self_ty) } else { raw_masked_body };
    let body_transformed = if self_param.is_some() {
        rewrite_self_to_this(&body_with_self_ty, &masked_with_self_ty).trim().to_string()
    } else {
        if contains_word(&masked_with_self_ty, "self") {
            return None;
        }
        body_with_self_ty.trim().to_string()
    };
    let body_transformed = if let Some(ret_ty) = &effective_ret_ty { format!("let ret: {ret_ty} = {{ {body_transformed} }};\nret") } else { body_transformed };

    let ws = ws?;
    let (decl_line, decl_col) = byte_to_line_col(doc_text, starts, item.name_byte);
    let refs_obj = ws.find_direct_references_at(file, decl_line, decl_col)?;
    if refs_obj.references.is_empty() {
        return None;
    }

    let canonical_file = std::fs::canonicalize(file).ok();
    let has_cross_file = refs_obj.references.iter().any(|r| {
        let abs = resolve_ref_to_absolute(ws, file, r);
        !is_same_doc(&abs, file, canonical_file.as_deref())
    });

    if !is_pub && has_cross_file {
        return None;
    }

    let (del_lo, del_hi) = line_deletion_span(doc_text, item.first_attr_byte, *bc);
    let mut local_edits = Vec::new();
    let other_file_edits: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    let def_lo = item.first_attr_byte.min(item.decl_start_byte);
    let mut any_skipped = false;

    let mut refs_by_file: BTreeMap<PathBuf, Vec<(u32, u32)>> = BTreeMap::new();
    for r in &refs_obj.references {
        let abs = resolve_ref_to_absolute(ws, file, r);
        refs_by_file.entry(abs).or_default().push((r.span.start_line(), r.span.start_col()));
    }

    for (abs_path, spans) in refs_by_file {
        if is_same_doc(&abs_path, file, canonical_file.as_deref()) {
            for (span_line, span_col) in spans {
                let Some(ref_byte) = line_col_to_byte(doc_text, starts, span_line, span_col) else {
                    any_skipped = true;
                    continue;
                };
                if ref_byte >= def_lo && ref_byte < *bc {
                    continue;
                }
                if let Some((site_start, site_end, inlined_text)) = process_call_site(doc_text, masked_doc, ref_byte, &item.name, self_kind, &parsed_params, &body_transformed) {
                    local_edits.push((site_start, site_end, inlined_text));
                } else {
                    any_skipped = true;
                }
            }
        }
    }

    if local_edits.is_empty() {
        return None;
    }

    if !any_skipped && !is_pub {
        local_edits.push((del_lo, del_hi, String::new()));
    }
    let local_lsp_edits = to_lsp_edits(doc_text, starts, local_edits)?;

    Some((local_lsp_edits, other_file_edits))
}

fn resolve_ref_to_absolute(ws: &WorkspaceIndex, doc_file: &str, r: &ridx_format::WorkspaceReference) -> PathBuf {
    let p = r.resolved_path();
    let doc_path = Path::new(doc_file);
    if p == doc_path || r.file == doc_file {
        return doc_path.to_path_buf();
    }
    let r_norm = r.file.strip_prefix('/').unwrap_or(r.file);
    if doc_file.ends_with(r_norm) {
        let prefix_len = doc_file.len() - r_norm.len();
        if prefix_len == 0 || doc_file.as_bytes()[prefix_len - 1] == b'/' {
            return doc_path.to_path_buf();
        }
    }
    if let Some(p_str) = p.to_str() {
        let p_norm = p_str.strip_prefix('/').unwrap_or(p_str);
        if doc_file.ends_with(p_norm) {
            let prefix_len = doc_file.len() - p_norm.len();
            if prefix_len == 0 || doc_file.as_bytes()[prefix_len - 1] == b'/' {
                return doc_path.to_path_buf();
            }
        }
    }
    if p.is_absolute() && p.exists() {
        return p;
    }
    if let Some(root) = ws.workspace_root() {
        let joined = root.join(&p);
        if joined.exists() {
            return joined;
        }
        let joined_rf = root.join(Path::new(r.file));
        if joined_rf.exists() {
            return joined_rf;
        }
    }
    if let Some(prefix) = doc_file.strip_suffix(r_norm) {
        let prefix_path = Path::new(prefix);
        let joined = prefix_path.join(&p);
        if joined.exists() {
            return joined;
        }
        let joined_rf = prefix_path.join(Path::new(r.file));
        if joined_rf.exists() {
            return joined_rf;
        }
    }
    p
}

fn is_same_doc(abs: &Path, doc_file: &str, canonical_file: Option<&Path>) -> bool {
    if abs == Path::new(doc_file) {
        return true;
    }
    if let Some(canon) = canonical_file {
        if let Ok(abs_canon) = std::fs::canonicalize(abs) {
            if abs_canon == canon {
                return true;
            }
        }
    }
    false
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
