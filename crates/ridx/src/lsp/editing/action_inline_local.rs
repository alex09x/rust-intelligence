/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Compile-safe local `Convert nested function to closure`, `Inline type alias into all uses`, and `Inline into all callers`.

use super::action_crate_checks::{is_leaf_scope_without_macros, nested_fn_has_non_runtime_ref};
use super::action_default_scope::enclosing_module_span;
use super::action_generics_helpers::clean_param_ident;
use super::action_generics_sub::{parse_generics_list, split_top_level_colon, split_top_level_commas, GenKind};
use super::action_inline_callers_helpers::contains_recursive_call;
use super::action_parse::{byte_to_utf16_pos, is_ident_byte, mask_comments_and_strings, EnclosingContext, ItemKindInfo, ParsedCursorItem};
use super::action_parse_decl::{collapse_ws, extract_angle_generics, next_ident_token};
use super::action_parse_items::{contains_word, skip_balanced};
use serde_json::{json, Value};

pub fn build_nested_fn_to_closure(doc_text: &str, masked_doc: &str, starts: &[usize], item: &ParsedCursorItem) -> Option<Vec<Value>> {
    if !item.in_fn_body || !matches!(item.enclosing, EnclosingContext::None) || !item.vis_prefix.is_empty() || !item.cfg_attrs.is_empty() || item.has_doc {
        return None;
    }
    let ItemKindInfo::Fn { is_unsafe: false, extern_abi: None, generics_src, params_src, ret_src, where_src, has_body: true, body_range: Some((bo, bc)) } = &item.kind else {
        return None;
    };
    if !generics_src.trim().is_empty() || !where_src.trim().is_empty() {
        return None;
    }
    // Functions returning `impl Trait` cannot be reliably inlined into expressions without losing type inference (e.g. E0283).
    if ret_src.as_deref().is_some_and(|r| r.contains("impl ")) {
        return None;
    }
    let hdr_m = &masked_doc[item.decl_start_byte..*bo];
    if contains_word(hdr_m, "async") || contains_word(hdr_m, "const") {
        return None;
    }
    // Avoid closure return-reference lifetime inference pitfalls (`|x: &T| -> &T`).
    if let Some(r) = ret_src {
        if (r.contains('&') || r.contains('\'')) && params_src.contains('&') {
            return None;
        }
    }
    let (blk_open, blk_close) = item.enclosing_block?;
    // Reject forward calls before declaration or recursive self-calls inside body, or turbofish calls.
    if contains_word(&masked_doc[blk_open + 1..item.decl_start_byte], &item.name) || contains_word(&masked_doc[*bo..*bc], &item.name) {
        return None;
    }
    let after_slice = &masked_doc[*bc..blk_close];
    if after_slice.contains(&format!("{}::<", item.name)) || nested_fn_has_non_runtime_ref(after_slice, &item.name) {
        return None;
    }

    let params_clean = params_src.trim();
    let body_text = &doc_text[*bo..*bc];
    let new_text = match ret_src {
        Some(r) => format!("let {} = |{params_clean}| -> {r} {body_text};", item.raw_name),
        None => format!("let {} = |{params_clean}| {body_text};", item.raw_name),
    };
    let (sl, sc) = byte_to_utf16_pos(doc_text, starts, item.decl_start_byte);
    let (el, ec) = byte_to_utf16_pos(doc_text, starts, *bc);
    Some(vec![json!({
        "range": { "start": { "line": sl, "character": sc }, "end": { "line": el, "character": ec } },
        "newText": new_text
    })])
}

pub fn build_inline_type_alias(doc_text: &str, masked_doc: &str, starts: &[usize], item: &ParsedCursorItem) -> Option<Vec<Value>> {
    if !item.vis_prefix.is_empty() || !matches!(item.enclosing, EnclosingContext::None) || !item.cfg_attrs.is_empty() {
        return None;
    }
    let ItemKindInfo::AssocOrTypeAlias { end_byte } = &item.kind else {
        return None;
    };
    let (mod_lo, mod_hi) = enclosing_module_span(masked_doc, item.decl_start_byte);
    if !is_leaf_scope_without_macros(&masked_doc[mod_lo..mod_hi], &item.name) {
        return None;
    }

    let decl_src = &doc_text[item.decl_start_byte..*end_byte];
    let decl_m = &masked_doc[item.decl_start_byte..*end_byte];
    let (_, kw_end) = next_ident_token(decl_m, 0)?;
    let (_, name_end) = next_ident_token(decl_src, kw_end)?;
    let (gen_s, after_g) = extract_angle_generics(decl_src, decl_m, name_end);
    let gens = parse_generics_list(&gen_s);
    if gens.len() > 1 || gens.iter().any(|g| !g.inline_bounds.is_empty() || g.kind == GenKind::Const) {
        return None;
    }
    let eq_rel = decl_m[after_g..].find('=')?;
    if contains_word(&decl_m[after_g..after_g + eq_rel], "where") {
        return None;
    }
    let rhs_raw = decl_src[after_g + eq_rel + 1..decl_src.len() - 1].trim();
    if rhs_raw.is_empty() {
        return None;
    }
    let rhs_sub_base = if rhs_raw.contains('+') && !rhs_raw.starts_with('(') { format!("({rhs_raw})") } else { rhs_raw.to_string() };
    let rhs_complex = rhs_raw.contains('<') || rhs_raw.contains('&') || rhs_raw.contains('(') || rhs_raw.contains('[') || rhs_raw.contains(' ');

    let (del_lo, del_hi) = line_deletion_span(doc_text, item.first_attr_byte, *end_byte);
    let mut raw_edits: Vec<(usize, usize, String)> = Vec::new();
    let mb = masked_doc.as_bytes();
    let mut i = mod_lo;
    while i < mod_hi {
        if i >= del_lo && i < del_hi {
            i = del_hi;
            continue;
        }
        if mb[i].is_ascii_alphabetic() || mb[i] == b'_' {
            if let Some((raw_tok, tok_end)) = next_ident_token(masked_doc, i) {
                let tok = raw_tok.strip_prefix("r#").unwrap_or(raw_tok);
                if tok == item.name {
                    let prev_bad = (i > 0 && mb[i - 1] == b'.') || (i >= 2 && &mb[i - 2..i] == b"::");
                    if prev_bad {
                        i = tok_end;
                        continue;
                    }
                    let after_trim = masked_doc[tok_end..mod_hi].trim_start();
                    if (after_trim.starts_with("::") || after_trim.starts_with('{') || after_trim.starts_with('(')) && rhs_complex {
                        return None;
                    }
                    if gens.is_empty() {
                        if after_trim.starts_with('<') {
                            return None;
                        }
                        raw_edits.push((i, tok_end, rhs_sub_base.clone()));
                        i = tok_end;
                        continue;
                    } else {
                        let (arg_s, after_ag) = extract_angle_generics(doc_text, masked_doc, tok_end);
                        let args = split_top_level_commas(&arg_s);
                        if args.len() != 1 {
                            return None;
                        }
                        let sub = replace_param_token(&rhs_sub_base, &gens[0].name, args[0].trim());
                        raw_edits.push((i, after_ag, sub));
                        i = after_ag;
                        continue;
                    }
                }
                i = tok_end;
                continue;
            }
        }
        i += 1;
    }
    if raw_edits.is_empty() {
        return None;
    }
    raw_edits.push((del_lo, del_hi, String::new()));
    super::action_struct_conv_helpers::to_lsp_edits(doc_text, starts, raw_edits)
}

pub fn build_inline_into_all_callers(doc_text: &str, masked_doc: &str, starts: &[usize], item: &ParsedCursorItem) -> Option<Vec<Value>> {
    if !item.vis_prefix.is_empty() || !matches!(item.enclosing, EnclosingContext::None) || !item.cfg_attrs.is_empty() {
        return None;
    }
    let ItemKindInfo::Fn { is_unsafe: false, extern_abi: None, generics_src, params_src, ret_src, where_src, has_body: true, body_range: Some((bo, bc)) } = &item.kind else {
        return None;
    };
    if !generics_src.trim().is_empty() || !where_src.trim().is_empty() {
        return None;
    }
    // Functions returning `impl Trait` cannot be reliably inlined into expressions without losing type inference (e.g. E0283).
    if ret_src.as_deref().is_some_and(|r| r.contains("impl ")) {
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
        || contains_word(body_inner_m, "self")
        || contains_word(body_inner_m, "Self")
        || body_inner_m.contains('$')
        || contains_recursive_call(body_inner_m, &item.name)
    {
        return None;
    }

    let mut typed_params: Vec<(String, String)> = Vec::new();
    for raw_p in split_top_level_commas(params_src) {
        let t = raw_p.trim();
        if t.is_empty() {
            continue;
        }
        let (lhs, Some(rhs)) = split_top_level_colon(t) else { return None };
        let l_trim = lhs.trim();
        let (is_mut, raw_id) = if let Some(r) = l_trim.strip_prefix("mut ") { (true, r.trim()) } else { (false, l_trim) };
        let pid = clean_param_ident(raw_id)?;
        let pty = collapse_ws(rhs);
        if pty.is_empty() || pty.contains("impl ") || pty.contains('_') {
            return None;
        }
        let bind_lhs = if is_mut { format!("mut {pid}") } else { pid };
        typed_params.push((bind_lhs, pty));
    }

    let (scope_lo, scope_hi) = item.enclosing_block.unwrap_or_else(|| enclosing_module_span(masked_doc, item.decl_start_byte));
    if !is_leaf_scope_without_macros(&masked_doc[scope_lo..scope_hi], &item.name) {
        return None;
    }

    let body_inner_src = doc_text[*bo + 1..*bc - 1].trim();
    let (del_lo, del_hi) = line_deletion_span(doc_text, item.first_attr_byte, *bc);
    let mut raw_edits: Vec<(usize, usize, String)> = Vec::new();
    let mb = masked_doc.as_bytes();
    let mut i = scope_lo;
    while i < scope_hi {
        if i >= del_lo && i < del_hi {
            i = del_hi;
            continue;
        }
        if mb[i].is_ascii_alphabetic() || mb[i] == b'_' {
            if let Some((raw_tok, tok_end)) = next_ident_token(masked_doc, i) {
                let tok = raw_tok.strip_prefix("r#").unwrap_or(raw_tok);
                if tok == item.name {
                    let prev_qual = (i > 0 && mb[i - 1] == b'.') || (i >= 2 && &mb[i - 2..i] == b"::");
                    if prev_qual {
                        i = tok_end;
                        continue;
                    }
                    let mut p = tok_end;
                    while p < scope_hi && mb[p].is_ascii_whitespace() {
                        p += 1;
                    }
                    if p >= scope_hi || mb[p] != b'(' {
                        return None;
                    }
                    let call_end = skip_balanced(mb, p, b'(', b')')?;
                    let args: Vec<String> = split_top_level_commas(&doc_text[p + 1..call_end - 1]).into_iter().filter(|a| !a.trim().is_empty()).collect();
                    if args.len() != typed_params.len() {
                        return None;
                    }
                    let returns_borrow = ret_src.as_deref().is_some_and(|r| r.contains('&') || r.contains('\''));
                    if returns_borrow && !typed_params.is_empty() && args.iter().any(|a| a.contains('(')) {
                        return None;
                    }
                    let stmts = match typed_params.len() {
                        0 => String::new(),
                        1 => format!("let {}: {} = {}; ", typed_params[0].0, typed_params[0].1, args[0].trim()),
                        _ => {
                            let pat = typed_params.iter().map(|(p, _)| p.as_str()).collect::<Vec<_>>().join(", ");
                            let tys = typed_params.iter().map(|(_, t)| t.as_str()).collect::<Vec<_>>().join(", ");
                            let vals = args.iter().map(|a| a.trim()).collect::<Vec<_>>().join(", ");
                            format!("let ({pat}): ({tys}) = ({vals}); ")
                        }
                    };
                    let blk = if stmts.is_empty() { format!("{{ {body_inner_src} }}") } else { format!("{{ {stmts}{body_inner_src} }}") };
                    let prev_ch = masked_doc[scope_lo..i].trim_end().as_bytes().last().copied().unwrap_or(b'{');
                    let next_ch = masked_doc[call_end..scope_hi].trim_start().as_bytes().first().copied().unwrap_or(b'}');
                    let standalone = matches!(prev_ch, b'{' | b';') && matches!(next_ch, b';' | b'}');
                    let inlined = if standalone { blk } else { format!("({blk})") };
                    raw_edits.push((i, call_end, inlined));
                    i = call_end;
                    continue;
                }
                i = tok_end;
                continue;
            }
        }
        i += 1;
    }
    if raw_edits.is_empty() {
        return None;
    }
    raw_edits.push((del_lo, del_hi, String::new()));
    super::action_struct_conv_helpers::to_lsp_edits(doc_text, starts, raw_edits)
}

pub(crate) fn line_deletion_span(src: &str, first_byte: usize, end_byte: usize) -> (usize, usize) {
    let line_start = src[..first_byte].rfind('\n').map(|p| p + 1).unwrap_or(0);
    let lo = if src[line_start..first_byte].bytes().all(|b| b == b' ' || b == b'\t') { line_start } else { first_byte };
    let hi = if src.as_bytes().get(end_byte).copied() == Some(b'\n') { end_byte + 1 } else { end_byte };
    (lo, hi)
}

fn replace_param_token(rhs: &str, param: &str, arg: &str) -> String {
    let masked = mask_comments_and_strings(rhs);
    let mb = masked.as_bytes();
    let pb = param.as_bytes();
    let mut out = String::with_capacity(rhs.len() + 8);
    let mut i = 0usize;
    while i < mb.len() {
        if i + pb.len() <= mb.len() && &mb[i..i + pb.len()] == pb {
            let prev_ok = i == 0 || (!is_ident_byte(mb[i - 1]) && mb[i - 1] != b'.' && !(i >= 2 && &mb[i - 2..i] == b"::"));
            let next_ok = i + pb.len() == mb.len() || !is_ident_byte(mb[i + pb.len()]);
            if prev_ok && next_ok {
                out.push_str(arg);
                i += pb.len();
                continue;
            }
        }
        let ch = rhs[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}
