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

use super::action_inline_callers_cross::{is_safe_for_cross_file_inline, is_same_doc, process_other_document_call_sites, resolve_ref_to_absolute};
use super::action_inline_callers_early_return::transform_early_returns;
use super::action_inline_callers_helpers::{contains_recursive_call, is_primitive_scalar, parse_fn_params, FnParamInfo};
pub(crate) use super::action_inline_callers_sites::process_call_sites_for_document;
use super::action_inline_callers_transform::{
    extract_type_generic_names, find_enclosing_mod_range, has_only_lifetime_generics, rewrite_self_to_this, rewrite_self_type_to_name, unwrap_tail_return,
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
/// Call sites as 1-based `(line, column)`.
type Spans = Vec<(u32, u32)>;

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
    // A `#[cfg]` on the function changes nothing here: the index holds the callers compiled with it, as for any
    // function.
    if (!vis.is_empty() && !vis.starts_with("pub")) || is_primitive_scalar(&item.name) {
        return None;
    }
    let is_pub = vis.starts_with("pub");
    let is_fully_pub = vis == "pub";
    // A `#[cfg(..)]` impl holds the function only where its calls compile: the body compiles there too. The index
    // sees the calls of one configuration (and none in macro bodies), so such a function is kept.
    let mut cfg_gated = false;
    match &item.enclosing {
        super::action_parse::EnclosingContext::InherentImpl { cfg_attrs, .. }
        | super::action_parse::EnclosingContext::TraitImpl { cfg_attrs, .. }
        | super::action_parse::EnclosingContext::TraitDecl { cfg_attrs, .. } => {
            let is_cfg = |a: &String| a.trim_start_matches("#[").trim_start().starts_with("cfg(");
            if cfg_attrs.iter().any(|a| !a.contains("doc(") && !a.contains("diagnostic::") && !is_cfg(a)) {
                return None;
            }
            cfg_gated = cfg_attrs.iter().any(is_cfg);
        }
        super::action_parse::EnclosingContext::None => {}
        _ => return None,
    }
    let ItemKindInfo::Fn { is_unsafe: false, extern_abi: None, generics_src, params_src, ret_src, where_src: _, has_body: true, body_range: Some((bo, bc)) } = &item.kind else {
        return None;
    };
    // The function's own type parameters take each call's turbofish; the impl's cannot be named at a call.
    let fn_generics = extract_type_generic_names(generics_src);
    let mut type_generics = fn_generics.clone();
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
        super::action_parse::EnclosingContext::InherentImpl { self_ty, .. } | super::action_parse::EnclosingContext::TraitImpl { self_ty, .. } => {
            let base_raw = self_ty.split('<').next().unwrap_or(self_ty).trim().rsplit("::").next().unwrap_or("").trim();
            let base_name = base_raw.strip_prefix("r#").unwrap_or(base_raw);
            (!base_name.is_empty() && base_name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')).then(|| base_name.to_string())
        }
        _ => None,
    };
    // An `impl Trait` result fixes the type its body infers (`std::iter::empty()`, a closure's parameters): inlined,
    // nothing would.
    if ret_src.as_deref().is_some_and(|r| r.contains("impl ")) {
        return None;
    }
    // A result that borrows (`-> &T`) may point into the receiver or an argument: each call site then takes only
    // places, which outlive the inlined block, and a `self` by reference.
    let borrowed_return = ret_src.as_deref().is_some_and(|r| r.starts_with('&') && !r.contains("'static"));
    let effective_ret_ty = if let Some(r) = ret_src.as_deref() {
        if type_generics.iter().any(|g| contains_word(r, g)) {
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
            // Named lifetimes are the callee's: the annotation keeps the type with them inferred (`Walker<I<'_>>`).
            Some(super::action_inline_callers_params::anonymize_lifetimes(r))
        }
    } else {
        None
    };

    // A `const fn` body is const-evaluable wherever it is called; an `async fn` call becomes `async move { body }` after
    // its arguments are bound, as eager and as lazy as the call. A const generic parameter is not written at the call.
    let is_async = contains_word(&masked_doc[item.decl_start_byte..*bo], "async");
    if contains_word(generics_src, "const") {
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

    let body_has_question_mark = super::action_inline_callers_params::has_own_question_mark(&raw_masked_body);
    if body_has_question_mark {
        let ret = effective_ret_ty.as_deref()?;
        if !ret.contains("Result") && !ret.contains("Option") {
            return None;
        }
    }
    // In an `async move` block a `?` returns from the block, as from the `async fn`: the caller's type is no matter.
    let body_has_question_mark = body_has_question_mark && !is_async;

    if contains_word(&raw_masked_body, "super")
        || raw_masked_body.contains('$')
        // A free function names `Self` only inside a macro's tokens (`quote! { Self }`), which are output, not its own.
        || (self_type_name.is_none() && contains_word(&super::action_inline_callers_params::mask_macro_tokens(&raw_masked_body), "Self"))
        || contains_recursive_call(&raw_masked_body, &item.name)
        || type_generics.iter().any(|g| !fn_generics.contains(g) && contains_word(&raw_masked_body, g))
        || (effective_ret_ty.is_none()
            && (raw_body_slice.trim().trim_end_matches(';').trim() == "None" || (raw_masked_body.contains(".collect()") && !raw_masked_body.contains("collect::<"))))
    {
        return None;
    }

    let parsed_params = parse_fn_params(params_src, &type_generics)?;
    if super::action_inline_callers_params::generic_param_receives_call(params_src, &type_generics, &raw_masked_body) {
        return None;
    }
    let self_param = parsed_params.iter().find(|p| p.is_self);
    let self_kind = self_param.and_then(|p| p.self_kind);

    if is_fully_pub && item.name == "new" && !has_safe_pub_new_params(&parsed_params) {
        return None;
    }

    let body_with_self_ty = match &self_type_name {
        Some(ty_name) if contains_word(&raw_masked_body, "Self") => rewrite_self_type_to_name(&raw_body, &raw_masked_body, ty_name),
        _ => raw_body,
    };
    let masked_with_self_ty = if contains_word(&raw_masked_body, "Self") { mask_comments_and_strings(&body_with_self_ty) } else { raw_masked_body };
    let body_transformed = if self_param.is_some() {
        rewrite_self_to_this(&body_with_self_ty, &masked_with_self_ty).trim().to_string()
    } else {
        // A `self` of an item the body declares (`impl Drop for ResetGuard { fn drop(&mut self) .. }`) is that item's.
        let own = super::action_inline_callers_early_return::without_nested_items(&super::action_inline_callers_params::mask_macro_tokens(&masked_with_self_ty));
        if contains_word(&own, "self") || masked_with_self_ty.contains("self::") {
            return None;
        }
        body_with_self_ty.trim().to_string()
    };
    let body_transformed = match &effective_ret_ty {
        Some(ret_ty) if !ret_ty.contains("Self") => format!("let ret: {ret_ty} = {{ {body_transformed} }};\nret"),
        _ => body_transformed,
    };
    let body_transformed = if is_async { format!("async move {{ {body_transformed} }}") } else { body_transformed };

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

    let is_trait_member = matches!(&item.enclosing, super::action_parse::EnclosingContext::TraitImpl { .. } | super::action_parse::EnclosingContext::TraitDecl { .. })
        || item.enclosing_block.is_some_and(|(bo, _)| {
            let hdr = masked_doc[..bo].trim_end();
            let start = hdr.rfind('}').map_or(0, |p| p + 1);
            let block_hdr = &hdr[start..];
            (block_hdr.contains("impl ") && block_hdr.contains(" for ")) || block_hdr.contains("trait ")
        });
    // A private function called from other files is called from modules inside its own, which see what its body
    // names; those files are rewritten as for a `pub` one.

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

    let cross_file_safe = is_safe_for_cross_file_inline(&body_transformed, masked_doc);
    // A call in a module nested in the function's (`mod tests`) sees only what that module imports: it gets the body
    // with its names written as paths, as another file does.
    let fn_mod = find_enclosing_mod_range(masked_doc, item.decl_start_byte);
    let at_home = |&(sl, sc): &(u32, u32)| cross_file_safe || line_col_to_byte(doc_text, starts, sl, sc).is_some_and(|b| find_enclosing_mod_range(masked_doc, b) == fn_mod);
    let has_nested = refs_by_file.iter().any(|(p, spans)| is_same_doc(p, file, canonical_file.as_deref()) && !spans.iter().all(at_home));
    // Otherwise, for other files of the crate: names written as the paths the index resolved, members checked per file.
    let cross = ((has_cross_file || has_nested) && !cross_file_safe)
        .then(|| super::action_inline_callers_body::cross_bodies(doc_text, file, &item.name, item.name_byte, *bc, params_src, ws, &body_transformed, &parsed_params));
    for (abs_path, spans) in &refs_by_file {
        if is_same_doc(abs_path, file, canonical_file.as_deref()) {
            let (home, nested): (Spans, Spans) = spans.iter().partition(|s| at_home(s));
            let qualified = cross.as_ref().and_then(|c| c.local.as_ref()).map(|cb| (&cb.params[..], cb.body.as_str()));
            for (spans, with) in [(home, Some((&parsed_params[..], body_transformed.as_str()))), (nested, qualified)] {
                let Some((params, body)) = with.filter(|_| !spans.is_empty()) else {
                    any_skipped |= !spans.is_empty();
                    continue;
                };
                let (edits, skipped) = process_call_sites_for_document(
                    doc_text,
                    masked_doc,
                    starts,
                    &spans,
                    &item.name,
                    self_kind,
                    params,
                    body,
                    body_has_question_mark,
                    effective_ret_ty.as_deref(),
                    Some((def_lo, *bc)),
                    Some(file),
                    Some(ws),
                    is_fully_pub,
                    borrowed_return,
                    &fn_generics,
                );
                local_edits.extend(edits);
                any_skipped |= skipped;
            }
        } else if cross_file_safe && std::fs::read_to_string(abs_path).is_ok_and(|t| super::action_inline_callers_macros::macros_in_scope(raw_body_slice, &t)) {
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
                borrowed_return,
                &mut other_file_edits,
                &fn_generics,
            );
            any_skipped |= skipped;
        } else if let Some(cb) = cross.as_ref().and_then(|c| c.for_dest(ws, abs_path, raw_body_slice, doc_text, file, *bo + 1)) {
            let skipped = process_other_document_call_sites(
                abs_path,
                spans,
                &item.name,
                self_kind,
                &cb.params,
                &cb.body,
                body_has_question_mark,
                effective_ret_ty.as_deref(),
                file,
                Some(ws),
                is_fully_pub,
                borrowed_return,
                &mut other_file_edits,
                &fn_generics,
            );
            any_skipped |= skipped;
        } else {
            any_skipped = true;
        }
    }

    // Calls left as they are (one inside `assert_eq!`, or in a function returning neither `Result` nor `Option`)
    // keep the function; the others are inlined.
    if local_edits.is_empty() && other_file_edits.is_empty() {
        return None;
    }

    if !any_skipped && !is_pub && !is_trait_member && !cfg_gated {
        local_edits.push((del_lo, del_hi, String::new()));
    }
    let local_lsp_edits = if local_edits.is_empty() { Vec::new() } else { to_lsp_edits(doc_text, starts, local_edits)? };

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
