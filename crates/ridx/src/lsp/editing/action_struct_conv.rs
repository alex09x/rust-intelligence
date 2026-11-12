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
use super::action_struct_conv_attrs::{outer_attrs_allow_shape_change, AttrCtx, Shape};
use super::action_struct_conv_fields::{field_access_edits, splice};
use super::action_struct_conv_helpers::{
    collect_typed_locals, convert_brace_init_to_tuple, convert_tuple_init_to_brace, find_impl_body_spans_for, is_bare_value_ctor_fn_ref, is_brace_struct_literal_or_pat,
    is_isolated_leaf_module, parse_named_fields_clean, receiver_is_proven_owner, skip_optional_turbofish, to_lsp_edits,
};
use super::action_struct_conv_index::{byte_to_line_col, field_name_bytes, indexed_field_refs, struct_name_byte, struct_ref_sites, FieldRefs};
use super::action_struct_conv_outside::{convert_outside, doc_outside_edits, StructAt};
use super::action_struct_conv_tuple_fields::{named_field_decl, parse_tuple_fields_clean};
use ridx_format::WorkspaceIndex;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};

/// Edits for this document, plus `.name` → `.N` edits for other files keyed by URI.
pub type ConversionEdits = (Vec<Value>, BTreeMap<String, Vec<Value>>);

/// `ws` and `file` let the index decide which `.name` accesses belong to the struct, here and in other files; without
/// them (or when the index cannot vouch for every field) only the module-local text proof is used.
pub fn build_convert_to_tuple_struct(
    doc_text: &str,
    masked_doc: &str,
    starts: &[usize],
    item: &ParsedCursorItem,
    ws: Option<&WorkspaceIndex>,
    file: &str,
) -> Option<ConversionEdits> {
    if !matches!(item.enclosing, EnclosingContext::None) {
        return None;
    }
    let ItemKindInfo::Adt { adt_kw: "struct", where_src, fields_kind: AdtFieldsKind::Named(raw_fields), open_delim_byte, end_byte, .. } = &item.kind else {
        return None;
    };
    let attr_ctx = ws.map(|ws| AttrCtx { ws, file, doc: doc_text, starts });
    if !outer_attrs_allow_shape_change(doc_text, item.first_attr_byte, item.decl_start_byte, attr_ctx.as_ref(), Shape::Tuple) {
        return None;
    }
    // A tuple struct takes its `where` clause after the fields: `struct S<T>(T) where T: Copy;`.
    let where_at = if where_src.trim().is_empty() {
        None
    } else {
        let head = &masked_doc[item.decl_start_byte..*open_delim_byte];
        let b = masked_doc.as_bytes();
        let is_id = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
        let at = head.match_indices("where").map(|(p, _)| item.decl_start_byte + p).filter(|&p| (p == 0 || !is_id(b[p - 1])) && b.get(p + 5).is_none_or(|&c| !is_id(c))).last()?;
        Some((at, doc_text[at..*open_delim_byte].trim_end().trim_end_matches(',').to_string()))
    };
    let fields = parse_named_fields_clean(raw_fields)?;
    // A `macro_rules!` body naming a field (`$pat.tokens`) is no reference the index holds, and is left unrewritten.
    if fields.is_empty() || fields.len() > 16 || super::action_struct_conv_fields::macro_bodies_name_fields(masked_doc, fields.iter().map(|f| f.1.as_str())) {
        return None;
    }
    let (mod_lo, mod_hi) = item.enclosing_block.unwrap_or_else(|| enclosing_module_span(masked_doc, item.decl_start_byte));
    // Fields under `#[cfg(…)]` keep their indices only at the end, each carrying the predicates of those before it.
    let first_cfg = super::action_struct_conv_fields::first_cfg_field(&fields.iter().map(|f| f.3.as_str()).collect::<Vec<_>>())?;
    let shape = (fields.len(), first_cfg);
    let private_fields = fields.iter().all(|f| f.0.is_empty());
    let field_index: HashMap<&str, usize> = fields.iter().enumerate().map(|(idx, (_, n, _, _))| (n.as_str(), idx)).collect();
    let mut indexed = None;
    let mut other_files = BTreeMap::new();
    let mut doc_literals = Vec::new();
    let mut struct_sites = std::collections::HashSet::new();
    if let Some(ws) = ws {
        let names: Vec<&str> = fields.iter().map(|(_, n, _, _)| n.as_str()).collect();
        let name_byte = struct_name_byte(masked_doc, item.decl_start_byte, *open_delim_byte, &item.name);
        // The struct's references are looked up while the fields' are.
        let (field_refs, struct_refs) = std::thread::scope(|s| {
            let struct_refs = s.spawn(|| {
                let (line, col) = byte_to_line_col(doc_text, starts, name_byte?);
                ws.find_direct_references_at(file, line, col).filter(|r| r.target_line == line)
            });
            let field_refs =
                field_name_bytes(&masked_doc[*open_delim_byte..*end_byte], *open_delim_byte, &names).and_then(|nb| indexed_field_refs(ws, file, doc_text, starts, &nb));
            (field_refs, struct_refs.join().ok().flatten())
        });
        match (field_refs, struct_refs) {
            // Built, destructured or implemented elsewhere: those sites are rewritten too, or nothing is offered.
            (Some(FieldRefs { sites, other_files: files }), Some(refs)) => {
                let at = StructAt { file, doc: doc_text, starts, refs: &refs, module: (mod_lo, mod_hi), ws: Some(ws), private_fields };
                (other_files, doc_literals) = convert_outside(&at, files, &field_index, shape)?;
                struct_sites = struct_ref_sites(&refs, file, doc_text, starts);
                indexed = Some(sites);
            }
            // Without the fields' references only the module's text is rewritten: nothing elsewhere may need it.
            (None, Some(refs)) => {
                let at = StructAt { file, doc: doc_text, starts, refs: &refs, module: (mod_lo, mod_hi), ws: Some(ws), private_fields };
                if !convert_outside(&at, BTreeMap::new(), &field_index, shape).is_some_and(|(o, dl)| o.is_empty() && dl.is_empty()) {
                    return None;
                }
            }
            _ => {}
        }
    }
    // With the index every field reference is known, so child modules, macros and wider visibility are checked by
    // coverage below instead of being refused up front.
    if indexed.is_none()
        && (!is_isolated_leaf_module(&masked_doc[mod_lo..mod_hi], &item.name)
            || !uses_confined_to_module(&item.vis_prefix, fields.iter().map(|(vis, _, _, _)| vis.as_str()))
            || names_a_variant_too(&masked_doc[mod_lo..mod_hi], &item.name))
    {
        return None;
    }

    let impl_spans = find_impl_body_spans_for(doc_text, masked_doc, mod_lo, mod_hi, &item.name);
    let known_locals = collect_typed_locals(&masked_doc[mod_lo..mod_hi], &item.name);
    let mut raw_edits: Vec<(usize, usize, String)> = Vec::new();

    let tuple_defs: Vec<String> = fields.iter().map(|(vis, _, ty, _)| if vis.is_empty() { ty.clone() } else { format!("{vis} {ty}") }).collect();
    let name_end = where_at.as_ref().map_or_else(|| doc_text[..*open_delim_byte].trim_end().len(), |(at, _)| doc_text[..*at].trim_end().len());
    let decl = if fields.iter().all(|f| f.3.is_empty()) {
        format!("({});", tuple_defs.join(", "))
    } else {
        // Comments between fields stay with their field, one field per line.
        let line_start = doc_text[..item.decl_start_byte].rfind('\n').map_or(0, |p| p + 1);
        let indent: String = doc_text[line_start..].chars().take_while(|c| *c == ' ' || *c == '\t').collect();
        let mut s = String::from("(\n");
        for ((_, _, _, leading), def) in fields.iter().zip(&tuple_defs) {
            for c in leading.lines().map(str::trim).filter(|l| !l.is_empty()) {
                s.push_str(&format!("{indent}    {c}\n"));
            }
            s.push_str(&format!("{indent}    {def},\n"));
        }
        s.push_str(&format!("{indent});"));
        s
    };
    let decl = match &where_at {
        Some((_, clause)) => format!("{} {clause};", decl.trim_end_matches(';')),
        None => decl,
    };
    raw_edits.push((name_end, *end_byte, decl));

    let reaches_struct = |at: usize| ws.is_some_and(|ws| super::action_struct_conv_deref::tuple_field_reaches_struct(ws, file, doc_text, starts, at, &item.name));
    let mut field_edits =
        field_access_edits(doc_text, masked_doc, (mod_lo, mod_hi), (item.decl_start_byte, *end_byte), &field_index, indexed.as_ref(), &impl_spans, &known_locals, &reaches_struct)?;

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
                        if tok == item.name {
                            // `Enum::Name { .. }`: a variant that shares the struct's name, not the struct.
                            let qualifier =
                                masked_doc[..i].trim_end().strip_suffix("::").map(|q| q.trim_end().rsplit(|c: char| !(c.is_alphanumeric() || c == '_')).next().unwrap_or(""));
                            if qualifier.is_some_and(|q| q.starts_with(|c: char| c.is_ascii_uppercase())) {
                                i = tok_end;
                                continue;
                            }
                            // With the index, every literal of the struct is a reference to it.
                            if indexed.is_some() && !struct_sites.contains(&i) {
                                return None;
                            }
                        }
                        let brace_end = skip_balanced(mb, k, b'{', b'}')?;
                        // `Self { syntax: self.syntax.clone() }`: the accesses inside the literal are rewritten with it.
                        let (inner_lo, inner_hi) = (k + 1, brace_end - 1);
                        let nested: Vec<(usize, usize, String)> = field_edits.iter().filter(|e| e.0 >= inner_lo && e.1 <= inner_hi).cloned().collect();
                        field_edits.retain(|e| !(e.0 >= inner_lo && e.1 <= inner_hi));
                        let inner = splice(&doc_text[inner_lo..inner_hi], inner_lo, &nested);
                        let inner_masked = splice(&masked_doc[inner_lo..inner_hi], inner_lo, &nested);
                        let tup_text = convert_brace_init_to_tuple(&inner, &inner_masked, shape, &field_index)?;
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

    raw_edits.extend(field_edits);

    // Every place the compiler resolved to a field must be rewritten by one of the edits.
    if let Some(sites) = &indexed {
        raw_edits.extend(doc_outside_edits(doc_text, masked_doc, (mod_lo, mod_hi), sites, &doc_literals, &field_index, shape)?);
        if !sites.iter().all(|&s| raw_edits.iter().any(|&(lo, hi, _)| s >= lo && s < hi)) {
            return None;
        }
    }
    Some((to_lsp_edits(doc_text, starts, raw_edits)?, other_files))
}

pub fn build_convert_to_named_struct(doc_text: &str, masked_doc: &str, starts: &[usize], item: &ParsedCursorItem) -> Option<Vec<Value>> {
    if !matches!(item.enclosing, EnclosingContext::None) {
        return None;
    }
    let ItemKindInfo::Adt { adt_kw: "struct", where_src, fields_kind: AdtFieldsKind::Tuple(raw_fields), open_delim_byte, end_byte, .. } = &item.kind else {
        return None;
    };
    if !where_src.trim().is_empty() || !outer_attrs_allow_shape_change(doc_text, item.first_attr_byte, item.decl_start_byte, None, Shape::Named) {
        return None;
    }
    let (mod_lo, mod_hi) = item.enclosing_block.unwrap_or_else(|| enclosing_module_span(masked_doc, item.decl_start_byte));
    if !is_isolated_leaf_module(&masked_doc[mod_lo..mod_hi], &item.name) {
        return None;
    }

    let fields = parse_tuple_fields_clean(raw_fields)?;
    if fields.is_empty() || fields.len() > 16 || !uses_confined_to_module(&item.vis_prefix, fields.iter().map(|(_, vis, _)| vis.as_str())) {
        return None;
    }

    let impl_spans = find_impl_body_spans_for(doc_text, masked_doc, mod_lo, mod_hi, &item.name);
    let known_locals = collect_typed_locals(&masked_doc[mod_lo..mod_hi], &item.name);
    let mut raw_edits: Vec<(usize, usize, String)> = Vec::new();

    let named_defs: Vec<String> = fields.iter().enumerate().map(|(idx, f)| named_field_decl(idx, f)).collect();
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

/// Whether `Type::Name` appears in the module: a variant (or associated item) shares the struct's name, and without
/// the index its declaration and uses cannot be told from the struct's literals.
fn names_a_variant_too(mod_masked: &str, name: &str) -> bool {
    let b = mod_masked.as_bytes();
    mod_masked.match_indices(name).any(|(p, _)| {
        b.get(p + name.len()).is_none_or(|c| !is_ident_byte(*c))
            && mod_masked[..p]
                .strip_suffix("::")
                .is_some_and(|q| q.rsplit(|c: char| !(c.is_alphanumeric() || c == '_')).next().is_some_and(|s| s.starts_with(|c: char| c.is_ascii_uppercase())))
    })
}

/// Literals, patterns and field accesses need every touched field to be visible. Private fields are visible only in the
/// defining module and its descendants, so with all fields private every rewritten site lies in the scanned module text
/// even when the struct itself is `pub`.
fn uses_confined_to_module<'a>(struct_vis: &str, mut field_vis: impl Iterator<Item = &'a str>) -> bool {
    struct_vis.is_empty() || field_vis.all(str::is_empty)
}
