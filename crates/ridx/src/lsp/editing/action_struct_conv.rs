/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Compile-safe intra-module `Convert to tuple struct` and `Convert to named struct` (`refactor.rewrite`).

use super::action_default_scope::enclosing_module_span;
use super::action_parse::{is_ident_byte, AdtFieldsKind, EnclosingContext, ItemKindInfo, ParsedCursorItem};
use super::action_parse_decl::next_ident_token;
use super::action_parse_items::skip_balanced;
use super::action_struct_conv_helpers::{
    collect_typed_locals, convert_brace_init_to_tuple, convert_tuple_init_to_brace, find_impl_body_spans_for, is_bare_value_ctor_fn_ref, is_brace_struct_literal_or_pat,
    is_isolated_leaf_module, outer_attrs_allow_shape_change, parse_named_fields_clean, parse_tuple_fields_clean, receiver_is_proven_owner, skip_optional_turbofish, to_lsp_edits,
};
use serde_json::Value;
use std::collections::HashMap;

pub fn build_convert_to_tuple_struct(doc_text: &str, masked_doc: &str, starts: &[usize], item: &ParsedCursorItem) -> Option<Vec<Value>> {
    if !item.vis_prefix.is_empty() || !matches!(item.enclosing, EnclosingContext::None) {
        return None;
    }
    let ItemKindInfo::Adt { adt_kw: "struct", where_src, fields_kind: AdtFieldsKind::Named(raw_fields), open_delim_byte, end_byte, .. } = &item.kind else {
        return None;
    };
    if !where_src.trim().is_empty() || !outer_attrs_allow_shape_change(&doc_text[item.first_attr_byte..item.decl_start_byte]) {
        return None;
    }
    let (mod_lo, mod_hi) = item.enclosing_block.unwrap_or_else(|| enclosing_module_span(masked_doc, item.decl_start_byte));
    if !is_isolated_leaf_module(&masked_doc[mod_lo..mod_hi], &item.name) {
        return None;
    }

    let fields = parse_named_fields_clean(raw_fields)?;
    if fields.is_empty() || fields.len() > 16 {
        return None;
    }

    let impl_spans = find_impl_body_spans_for(doc_text, masked_doc, mod_lo, mod_hi, &item.name);
    let known_locals = collect_typed_locals(&masked_doc[mod_lo..mod_hi], &item.name);
    let mut raw_edits: Vec<(usize, usize, String)> = Vec::new();

    let tuple_defs: Vec<String> = fields.iter().map(|(vis, _, ty)| if vis.is_empty() { ty.clone() } else { format!("{vis} {ty}") }).collect();
    raw_edits.push((*open_delim_byte, *end_byte, format!("({});", tuple_defs.join(", "))));

    let field_index: HashMap<&str, usize> = fields.iter().enumerate().map(|(idx, (_, n, _))| (n.as_str(), idx)).collect();

    let mb = masked_doc.as_bytes();
    let mut i = mod_lo;
    while i < mod_hi {
        if i >= item.decl_start_byte && i < *end_byte {
            i = *end_byte;
            continue;
        }
        if mb[i].is_ascii_alphabetic() || mb[i] == b'_' {
            if let Some((raw_tok, tok_end)) = next_ident_token(masked_doc, i) {
                let tok = raw_tok.strip_prefix("r#").unwrap_or(raw_tok);
                let in_impl = impl_spans.iter().any(|&(bo, bc)| i > bo && i < bc);
                if (tok == item.name || (tok == "Self" && in_impl)) && !(i > 0 && mb[i - 1] == b'.') {
                    let (_, after_tf) = skip_optional_turbofish(doc_text, masked_doc, tok_end);
                    let mut k = after_tf;
                    while k < mod_hi && mb[k].is_ascii_whitespace() {
                        k += 1;
                    }
                    if k < mod_hi && mb[k] == b'{' && is_brace_struct_literal_or_pat(masked_doc, mod_lo, i, k, &impl_spans) {
                        let brace_end = skip_balanced(mb, k, b'{', b'}')?;
                        let tup_text = convert_brace_init_to_tuple(&doc_text[k + 1..brace_end - 1], &masked_doc[k + 1..brace_end - 1], fields.len(), &field_index)?;
                        raw_edits.push((after_tf, brace_end, tup_text));
                        i = brace_end;
                        continue;
                    }
                }
                i = tok_end;
                continue;
            }
        }
        i += 1;
    }

    let mut d = mod_lo;
    while d + 1 < mod_hi {
        if d >= item.decl_start_byte && d < *end_byte {
            d = *end_byte;
            continue;
        }
        if raw_edits.iter().any(|&(lo, hi, _)| d >= lo && d < hi) {
            d += 1;
            continue;
        }
        if mb[d] == b'.' && mb.get(d + 1).copied() != Some(b'.') && (d == 0 || mb[d - 1] != b'.') {
            let mut k = d + 1;
            while k < mod_hi && mb[k].is_ascii_whitespace() {
                k += 1;
            }
            if let Some((raw_f, f_end)) = next_ident_token(doc_text, k) {
                let fname = raw_f.strip_prefix("r#").unwrap_or(raw_f);
                if let Some(&idx) = field_index.get(fname) {
                    let after_trim = masked_doc[f_end..mod_hi].trim_start();
                    if !after_trim.starts_with('(') && !after_trim.starts_with("::<") {
                        let in_impl = impl_spans.iter().any(|&(bo, bc)| d > bo && d < bc);
                        if !receiver_is_proven_owner(masked_doc, mod_lo, d, in_impl, &known_locals) {
                            return None;
                        }
                        raw_edits.push((k, f_end, idx.to_string()));
                        d = f_end;
                        continue;
                    }
                }
            }
        }
        d += 1;
    }

    to_lsp_edits(doc_text, starts, raw_edits)
}

pub fn build_convert_to_named_struct(doc_text: &str, masked_doc: &str, starts: &[usize], item: &ParsedCursorItem) -> Option<Vec<Value>> {
    if !item.vis_prefix.is_empty() || !matches!(item.enclosing, EnclosingContext::None) {
        return None;
    }
    let ItemKindInfo::Adt { adt_kw: "struct", where_src, fields_kind: AdtFieldsKind::Tuple(raw_fields), open_delim_byte, end_byte, .. } = &item.kind else {
        return None;
    };
    if !where_src.trim().is_empty() || !outer_attrs_allow_shape_change(&doc_text[item.first_attr_byte..item.decl_start_byte]) {
        return None;
    }
    let (mod_lo, mod_hi) = item.enclosing_block.unwrap_or_else(|| enclosing_module_span(masked_doc, item.decl_start_byte));
    if !is_isolated_leaf_module(&masked_doc[mod_lo..mod_hi], &item.name) {
        return None;
    }

    let fields = parse_tuple_fields_clean(raw_fields)?;
    if fields.is_empty() || fields.len() > 16 {
        return None;
    }

    let impl_spans = find_impl_body_spans_for(doc_text, masked_doc, mod_lo, mod_hi, &item.name);
    let known_locals = collect_typed_locals(&masked_doc[mod_lo..mod_hi], &item.name);
    let mut raw_edits: Vec<(usize, usize, String)> = Vec::new();

    let named_defs: Vec<String> = fields
        .iter()
        .enumerate()
        .map(|(idx, (vis, ty))| {
            let fnm = format!("field{}", idx + 1);
            if vis.is_empty() {
                format!("{fnm}: {ty}")
            } else {
                format!("{vis} {fnm}: {ty}")
            }
        })
        .collect();
    raw_edits.push((*open_delim_byte, *end_byte, format!(" {{ {} }}", named_defs.join(", "))));

    let mb = masked_doc.as_bytes();
    let mut i = mod_lo;
    while i < mod_hi {
        if i >= item.decl_start_byte && i < *end_byte {
            i = *end_byte;
            continue;
        }
        if mb[i].is_ascii_alphabetic() || mb[i] == b'_' {
            if let Some((raw_tok, tok_end)) = next_ident_token(masked_doc, i) {
                let tok = raw_tok.strip_prefix("r#").unwrap_or(raw_tok);
                let in_impl = impl_spans.iter().any(|&(bo, bc)| i > bo && i < bc);
                if (tok == item.name || (tok == "Self" && in_impl)) && !(i > 0 && mb[i - 1] == b'.') {
                    let (_, after_tf) = skip_optional_turbofish(doc_text, masked_doc, tok_end);
                    let mut k = after_tf;
                    while k < mod_hi && mb[k].is_ascii_whitespace() {
                        k += 1;
                    }
                    if k < mod_hi && mb[k] == b'(' {
                        let paren_end = skip_balanced(mb, k, b'(', b')')?;
                        let named_init = convert_tuple_init_to_brace(&doc_text[k + 1..paren_end - 1], &masked_doc[k + 1..paren_end - 1], fields.len())?;
                        raw_edits.push((after_tf, paren_end, named_init));
                        i = paren_end;
                        continue;
                    } else if tok == item.name && is_bare_value_ctor_fn_ref(masked_doc, mod_lo, i, after_tf) {
                        return None;
                    }
                }
                i = tok_end;
                continue;
            }
        }
        i += 1;
    }

    let mut d = mod_lo;
    while d + 1 < mod_hi {
        if d >= item.decl_start_byte && d < *end_byte {
            d = *end_byte;
            continue;
        }
        if raw_edits.iter().any(|&(lo, hi, _)| d >= lo && d < hi) {
            d += 1;
            continue;
        }
        if mb[d] == b'.' && mb.get(d + 1).copied() != Some(b'.') && (d == 0 || (!mb[d - 1].is_ascii_digit() && mb[d - 1] != b'.')) {
            let mut k = d + 1;
            while k < mod_hi && mb[k].is_ascii_whitespace() {
                k += 1;
            }
            let num_start = k;
            while k < mod_hi && mb[k].is_ascii_digit() {
                k += 1;
            }
            if k > num_start && (k == mod_hi || !is_ident_byte(mb[k])) {
                if let Ok(idx) = doc_text[num_start..k].parse::<usize>() {
                    if idx < fields.len() {
                        let in_impl = impl_spans.iter().any(|&(bo, bc)| d > bo && d < bc);
                        if !receiver_is_proven_owner(masked_doc, mod_lo, d, in_impl, &known_locals) {
                            return None;
                        }
                        raw_edits.push((num_start, k, format!("field{}", idx + 1)));
                        d = k;
                        continue;
                    }
                }
            }
        }
        d += 1;
    }

    to_lsp_edits(doc_text, starts, raw_edits)
}
