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

use super::action_inline_callers_cross::{is_same_doc, is_safe_for_cross_file_inline, process_other_document_call_sites, resolve_ref_to_absolute};
use super::action_inline_callers_early_return::transform_early_returns;
use super::action_inline_callers_helpers::{contains_recursive_call, is_caller_return_compatible, is_primitive_scalar, parse_fn_params, FnParamInfo, SelfKind};
use super::action_inline_callers_transform::{
    extract_type_generic_names, find_enclosing_mod_range, has_only_lifetime_generics, process_call_site, rewrite_self_to_this, rewrite_self_type_to_name, unwrap_tail_return,
};
use super::action_inline_local::line_deletion_span;
use super::action_parse::{mask_comments_and_strings, ItemKindInfo, ParsedCursorItem};
use super::action_parse_items::contains_word;
use super::action_struct_conv_helpers::to_lsp_edits;
use super::action_struct_conv_index::byte_to_line_col;
use ridx_format::WorkspaceIndex;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;

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
    if !item.cfg_attrs.is_empty() || (!vis.is_empty() && !vis.starts_with("pub")) || is_primitive_scalar(&item.name) {
        return None;
    }
    let is_pub = vis.starts_with("pub");
    let is_fully_pub = vis == "pub";
    match &item.enclosing {
        super::action_parse::EnclosingContext::InherentImpl { cfg_attrs, .. }
        | super::action_parse::EnclosingContext::TraitImpl { cfg_attrs, .. }
        | super::action_parse::EnclosingContext::TraitDecl { cfg_attrs, .. } => {
            if cfg_attrs.iter().any(|a| !a.contains("doc(") && !a.contains("diagnostic::")) {
                return None;
            }
        }
        super::action_parse::EnclosingContext::None => {}
        _ => return None,
    }
    let ItemKindInfo::Fn { is_unsafe: false, extern_abi: None, generics_src, params_src, ret_src, where_src: _, has_body: true, body_range: Some((bo, bc)) } = &item.kind else {
        return None;
    };
    let mut type_generics = extract_type_generic_names(generics_src);
    if let super::action_parse::EnclosingContext::InherentImpl { generics_src: impl_gens, .. }
        | super::action_parse::EnclosingContext::TraitImpl { generics_src: impl_gens, .. }
        | super::action_parse::EnclosingContext::TraitDecl { generics_src: impl_gens, .. } = &item.enclosing
    {
        type_generics.extend(extract_type_generic_names(impl_gens));
    }
    let impl_has_type_generics = match &item.enclosing {
        super::action_parse::EnclosingContext::InherentImpl { generics_src, .. }
        | super::action_parse::EnclosingContext::TraitImpl { generics_src, .. }
        | super::action_parse::EnclosingContext::TraitDecl { generics_src, .. } => !has_only_lifetime_generics(generics_src),
        _ => false,
    };
    let self_type_name = match &item.enclosing {
        super::action_parse::EnclosingContext::InherentImpl { self_ty, .. }
        | super::action_parse::EnclosingContext::TraitImpl { self_ty, .. } => {
            let base_raw = self_ty.split('<').next().unwrap_or(self_ty).trim().rsplit("::").next().unwrap_or("").trim();
            let base_name = base_raw.strip_prefix("r#").unwrap_or(base_raw);
            (!base_name.is_empty() && base_name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')).then(|| base_name.to_string())
        }
        _ => None,
    };
    let effective_ret_ty = if let Some(r) = ret_src.as_deref() {
        if r.contains("impl ") || (r.starts_with('&') && !r.contains("'static")) {
            return None;
        }
        if (r.contains('\'') && !r.contains("'static")) || type_generics.iter().any(|g| contains_word(r, g)) {
            None
        } else if r == "Self" {
            if impl_has_type_generics {
                None
            } else {
                Some(self_type_name.as_ref()?.clone())
            }
        } else if contains_word(r, "Self") {
            if impl_has_type_generics || r.contains("::") {
                None
            } else {
                self_type_name.as_ref().map(|ty_name| rewrite_self_type_to_name(r, &mask_comments_and_strings(r), ty_name))
            }
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
        } else if let Some((ub, um)) = transform_early_returns(raw_body_slice, body_inner_m) {
            (ub, um)
        } else {
            return None;
        }
    } else {
        (raw_body_slice.to_string(), body_inner_m.to_string())
    };

    let body_has_question_mark = raw_masked_body.contains('?');
    if body_has_question_mark {
        let ret = effective_ret_ty.as_deref()?;
        if !ret.contains("Result") && !ret.contains("Option") {
            return None;
        }
    }

    if contains_word(&raw_masked_body, "super")
        || raw_masked_body.contains('$')
        || (contains_word(&raw_masked_body, "Self") && self_type_name.is_none())
        || contains_recursive_call(&raw_masked_body, &item.name)
        || type_generics.iter().any(|g| contains_word(&raw_masked_body, g))
        || (effective_ret_ty.is_none() && (raw_body_slice.trim().trim_end_matches(';').trim() == "None" || (raw_masked_body.contains(".collect()") && !raw_masked_body.contains("collect::<"))))
    {
        return None;
    }

    let parsed_params = parse_fn_params(params_src, &type_generics)?;
    let self_param = parsed_params.iter().find(|p| p.is_self);
    let self_kind = self_param.and_then(|p| p.self_kind);

    if is_fully_pub && item.name == "new" && !has_safe_pub_new_params(&parsed_params) {
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
    let body_transformed = if let Some(ret_ty) = &effective_ret_ty {
        if !ret_ty.contains('\'') && !ret_ty.contains("Self") {
            format!("let ret: {ret_ty} = {{ {body_transformed} }};\nret")
        } else {
            body_transformed
        }
    } else {
        body_transformed
    };

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

    let is_trait_member = matches!(&item.enclosing, super::action_parse::EnclosingContext::TraitImpl { .. } | super::action_parse::EnclosingContext::TraitDecl { .. });
    if !is_pub && !is_trait_member && has_cross_file {
        return None;
    }

    let (del_lo, del_hi) = line_deletion_span(doc_text, item.first_attr_byte, *bc);
    let mut local_edits = Vec::new();
    let mut other_file_edits: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    let def_lo = item.first_attr_byte.min(item.decl_start_byte);
    let mut any_skipped = false;

    let mut refs_by_file: BTreeMap<PathBuf, Vec<(u32, u32)>> = BTreeMap::new();
    for r in &refs_obj.references {
        let abs = resolve_ref_to_absolute(ws, file, r);
        refs_by_file.entry(abs).or_default().push((r.span.start_line(), r.span.start_col()));
    }

    if is_fully_pub || vis == "pub(super)" {
        if let Some((mod_lo, mod_hi)) = find_enclosing_mod_range(masked_doc, item.decl_start_byte) {
            let has_cross_mod = refs_by_file.iter().any(|(abs_path, spans)| {
                is_same_doc(abs_path, file, canonical_file.as_deref())
                    && spans.iter().any(|&(sl, sc)| line_col_to_byte(doc_text, starts, sl, sc).is_some_and(|b| b < mod_lo || b >= mod_hi))
            });
            if has_cross_mod {
                return None;
            }
        }
    }

    let cross_file_safe = is_pub && is_safe_for_cross_file_inline(&body_transformed);
    for (abs_path, spans) in &refs_by_file {
        if is_same_doc(abs_path, file, canonical_file.as_deref()) {
            let (edits, skipped) = process_call_sites_for_document(
                doc_text,
                masked_doc,
                starts,
                spans,
                &item.name,
                self_kind,
                &parsed_params,
                &body_transformed,
                body_has_question_mark,
                effective_ret_ty.as_deref(),
                Some((def_lo, *bc)),
                Some(file),
                Some(ws),
                is_fully_pub,
            );
            local_edits.extend(edits);
            if skipped {
                any_skipped = true;
            }
        } else if cross_file_safe {
            let skipped = process_other_document_call_sites(
                abs_path,
                spans,
                &item.name,
                self_kind,
                &parsed_params,
                &body_transformed,
                body_has_question_mark,
                effective_ret_ty.as_deref(),
                file,
                Some(ws),
                is_fully_pub,
                &mut other_file_edits,
            );
            if skipped {
                any_skipped = true;
            }
        } else if !is_pub {
            any_skipped = true;
        }
    }

    if (local_edits.is_empty() && other_file_edits.is_empty()) || (body_has_question_mark && any_skipped) {
        return None;
    }

    if !any_skipped && !is_pub && !is_trait_member {
        local_edits.push((del_lo, del_hi, String::new()));
    }
    let local_lsp_edits = if local_edits.is_empty() {
        Vec::new()
    } else {
        to_lsp_edits(doc_text, starts, local_edits)?
    };

    Some((local_lsp_edits, other_file_edits))
}

fn has_safe_pub_new_params(params: &[FnParamInfo]) -> bool {
    params.is_empty()
        || params.iter().all(|p| {
            p.ty.as_deref().is_some_and(|ty| {
                let n = super::action_inline_callers_helpers::normalize_param_type_for_binding(ty);
                !n.contains('\'') && !n.contains("impl ") && !n.contains("dyn ") && !n.contains('<')
            })
        })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn process_call_sites_for_document(
    doc: &str,
    masked: &str,
    starts: &[usize],
    spans: &[(u32, u32)],
    fn_name: &str,
    self_kind: Option<SelfKind>,
    params: &[FnParamInfo],
    body_src: &str,
    body_has_question_mark: bool,
    effective_ret_ty: Option<&str>,
    def_range: Option<(usize, usize)>,
    file_opt: Option<&str>,
    ws_opt: Option<&WorkspaceIndex>,
    is_fully_pub: bool,
) -> (Vec<(usize, usize, String)>, bool) {
    let mut edits = Vec::new();
    let mut any_skipped = false;
    for &(span_line, span_col) in spans {
        let Some(ref_byte) = line_col_to_byte(doc, starts, span_line, span_col) else {
            any_skipped = true;
            continue;
        };
        if let Some((def_lo, def_hi)) = def_range {
            if ref_byte >= def_lo && ref_byte < def_hi {
                continue;
            }
        }
        if body_has_question_mark {
            let ret = effective_ret_ty.unwrap_or("");
            if !is_caller_return_compatible(doc, masked, starts, ref_byte, ret) {
                any_skipped = true;
                continue;
            }
        }
        if let Some((site_start, site_end, inlined_text)) = process_call_site(doc, masked, starts, ref_byte, fn_name, self_kind, params, body_src, file_opt, ws_opt, is_fully_pub) {
            edits.push((site_start, site_end, inlined_text));
        } else {
            any_skipped = true;
        }
    }
    (edits, any_skipped)
}
