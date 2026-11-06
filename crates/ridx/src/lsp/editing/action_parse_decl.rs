/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Declaration and enclosing `impl`/`trait` header parsers for compile-safe code actions.

use super::action_parse::{AdtFieldsKind, EnclosingContext, ItemKindInfo, ParsedCursorItem};
use super::action_parse_items::{contains_word, skip_balanced};
pub use super::action_parse_lex::{collapse_ws, extract_angle_generics, find_header_terminator, next_ident_token, split_impl_trait_for, split_top_level_where};

#[allow(clippy::too_many_arguments)]
pub fn classify_enclosing_brace(
    src: &str,
    masked: &str,
    hdr_kw_start: usize,
    open_brace_byte: usize,
    close_brace_byte: usize,
    cfg_attrs: Vec<String>,
    insert_line0: u32,
    indent: String,
) -> EnclosingContext {
    if hdr_kw_start >= open_brace_byte {
        return EnclosingContext::None;
    }
    let hdr_src = &src[hdr_kw_start..open_brace_byte];
    let hdr_masked = &masked[hdr_kw_start..open_brace_byte];
    if hdr_masked.contains('$') {
        return EnclosingContext::ExternBlock;
    }

    let (_, mut pos, _, has_extern) = skip_item_modifiers(hdr_src, hdr_masked, 0);
    if has_extern && !contains_word(&hdr_masked[pos..], "fn") {
        return EnclosingContext::ExternBlock;
    }
    let Some((kw, kw_end)) = next_ident_token(hdr_masked, pos) else {
        return EnclosingContext::None;
    };
    pos = kw_end;

    if kw == "impl" {
        let (generics_src, after_gen) = extract_angle_generics(hdr_src, hdr_masked, pos);
        let (head_src, head_masked, where_src) = split_top_level_where(&hdr_src[after_gen..], &hdr_masked[after_gen..]);
        if let Some((trait_part, self_part)) = split_impl_trait_for(head_src, head_masked) {
            let trait_ty = collapse_ws(trait_part.trim());
            let self_ty = collapse_ws(self_part.trim());
            if trait_ty.is_empty() || self_ty.is_empty() || trait_ty.starts_with('!') || trait_ty.starts_with('~') {
                return EnclosingContext::ExternBlock;
            }
            return EnclosingContext::TraitImpl {
                generics_src,
                trait_ty,
                self_ty,
                where_src,
                cfg_attrs,
                insert_line0,
                indent,
                body_open_byte: open_brace_byte,
                body_close_byte: close_brace_byte,
            };
        }
        let self_ty = collapse_ws(head_src.trim());
        if self_ty.is_empty() {
            return EnclosingContext::None;
        }
        return EnclosingContext::InherentImpl {
            generics_src,
            self_ty,
            where_src,
            cfg_attrs,
            insert_line0,
            indent,
            body_open_byte: open_brace_byte,
            body_close_byte: close_brace_byte,
        };
    }

    if kw == "trait" {
        let Some((trait_raw, name_end)) = next_ident_token(hdr_src, pos) else {
            return EnclosingContext::None;
        };
        let (generics_src, after_gen) = extract_angle_generics(hdr_src, hdr_masked, name_end);
        let (_, _, where_src) = split_top_level_where(&hdr_src[after_gen..], &hdr_masked[after_gen..]);
        return EnclosingContext::TraitDecl { generics_src, trait_name: trait_raw.to_string(), where_src, cfg_attrs, insert_line0, indent };
    }

    EnclosingContext::None
}

#[allow(clippy::too_many_arguments)]
pub fn parse_declaration_at(
    src: &str,
    masked: &str,
    kw_start: usize,
    first_attr_byte: usize,
    cursor_byte: usize,
    has_doc: bool,
    cfg_attrs: Vec<String>,
    attr_start_line0: u32,
    indent: String,
    in_fn_body: bool,
    enclosing_block: Option<(usize, usize)>,
    enclosing: EnclosingContext,
) -> Option<ParsedCursorItem> {
    let mb = masked.as_bytes();
    let (_, pos_after_mods, is_unsafe, extern_abi) = parse_decl_modifiers(src, masked, kw_start);
    let (kw, kw_end) = next_ident_token(masked, pos_after_mods)?;
    let kw_byte_start = pos_after_mods + masked[pos_after_mods..].find(kw)?;
    let (raw_name_s, name_end) = next_ident_token(src, kw_end)?;
    if cursor_byte < kw_byte_start || cursor_byte > name_end {
        return None;
    }
    let name_byte = name_end - raw_name_s.len();
    let raw_name = raw_name_s.to_string();
    let name = raw_name.strip_prefix("r#").unwrap_or(&raw_name).to_string();
    let vis_prefix = extract_vis_prefix(src[kw_start..kw_byte_start].trim());

    match kw {
        "fn" => {
            let (generics_src, after_gen) = extract_angle_generics(src, masked, name_end);
            let mut p = after_gen;
            while p < mb.len() && mb[p].is_ascii_whitespace() {
                p += 1;
            }
            if mb.get(p).copied() != Some(b'(') {
                return None;
            }
            let paren_end = skip_balanced(mb, p, b'(', b')')?;
            let term_idx = find_header_terminator(mb, paren_end)?;
            if masked[kw_start..term_idx].contains('$') {
                return None;
            }
            let params_src = src[p + 1..paren_end - 1].to_string();
            let (ret_raw, _, where_src) = split_top_level_where(&src[paren_end..term_idx], &masked[paren_end..term_idx]);
            let ret_trim = ret_raw.trim();
            let ret_src = if let Some(rest) = ret_trim.strip_prefix("->") {
                let r = collapse_ws(rest.trim());
                if r.is_empty() {
                    None
                } else {
                    Some(r)
                }
            } else if ret_trim.is_empty() {
                None
            } else {
                return None;
            };
            let has_body = mb[term_idx] == b'{';
            let body_range = if has_body { skip_balanced(mb, term_idx, b'{', b'}').map(|end_b| (term_idx, end_b)) } else { None };
            Some(ParsedCursorItem {
                name,
                raw_name,
                name_byte,
                vis_prefix,
                decl_start_byte: kw_start,
                first_attr_byte,
                has_doc,
                cfg_attrs,
                attr_start_line0,
                indent,
                in_fn_body,
                enclosing_block,
                enclosing,
                kind: ItemKindInfo::Fn { is_unsafe, extern_abi, generics_src, params_src, ret_src, where_src, has_body, body_range },
            })
        }
        "struct" | "enum" | "union" => {
            let adt_kw = match kw {
                "struct" => "struct",
                "enum" => "enum",
                _ => "union",
            };
            let (generics_src, after_gen) = extract_angle_generics(src, masked, name_end);
            let mut p = after_gen;
            while p < mb.len() && mb[p].is_ascii_whitespace() {
                p += 1;
            }
            if adt_kw == "struct" && mb.get(p).copied() == Some(b'(') {
                let paren_end = skip_balanced(mb, p, b'(', b')')?;
                let term_idx = find_header_terminator(mb, paren_end)?;
                if mb[term_idx] != b';' || masked[kw_start..term_idx].contains('$') {
                    return None;
                }
                let (_, _, where_src) = split_top_level_where(&src[paren_end..term_idx], &masked[paren_end..term_idx]);
                let tuple_src = src[p + 1..paren_end - 1].to_string();
                return Some(ParsedCursorItem {
                    name,
                    raw_name,
                    name_byte,
                    vis_prefix,
                    decl_start_byte: kw_start,
                    first_attr_byte,
                    has_doc,
                    cfg_attrs,
                    attr_start_line0,
                    indent,
                    in_fn_body,
                    enclosing_block,
                    enclosing,
                    kind: ItemKindInfo::Adt { adt_kw, generics_src, where_src, fields_kind: AdtFieldsKind::Tuple(tuple_src), open_delim_byte: p, end_byte: term_idx + 1 },
                });
            }
            let term_idx = find_header_terminator(mb, p)?;
            if masked[kw_start..term_idx].contains('$') {
                return None;
            }
            let (_, _, where_src) = split_top_level_where(&src[p..term_idx], &masked[p..term_idx]);
            if mb[term_idx] == b';' {
                if adt_kw != "struct" {
                    return None;
                }
                return Some(ParsedCursorItem {
                    name,
                    raw_name,
                    name_byte,
                    vis_prefix,
                    decl_start_byte: kw_start,
                    first_attr_byte,
                    has_doc,
                    cfg_attrs,
                    attr_start_line0,
                    indent,
                    in_fn_body,
                    enclosing_block,
                    enclosing,
                    kind: ItemKindInfo::Adt { adt_kw, generics_src, where_src, fields_kind: AdtFieldsKind::Unit, open_delim_byte: term_idx, end_byte: term_idx + 1 },
                });
            }
            let body_end = skip_balanced(mb, term_idx, b'{', b'}')?;
            let fields_kind = if adt_kw == "struct" { AdtFieldsKind::Named(src[term_idx + 1..body_end - 1].to_string()) } else { AdtFieldsKind::EnumOrUnion };
            Some(ParsedCursorItem {
                name,
                raw_name,
                name_byte,
                vis_prefix,
                decl_start_byte: kw_start,
                first_attr_byte,
                has_doc,
                cfg_attrs,
                attr_start_line0,
                indent,
                in_fn_body,
                enclosing_block,
                enclosing,
                kind: ItemKindInfo::Adt { adt_kw, generics_src, where_src, fields_kind, open_delim_byte: term_idx, end_byte: body_end },
            })
        }
        "type" => {
            let term_idx = find_header_terminator(mb, name_end)?;
            if masked[kw_start..term_idx].contains('$') || mb[term_idx] != b';' {
                return None;
            }
            Some(ParsedCursorItem {
                name,
                raw_name,
                name_byte,
                vis_prefix,
                decl_start_byte: kw_start,
                first_attr_byte,
                has_doc,
                cfg_attrs,
                attr_start_line0,
                indent,
                in_fn_body,
                enclosing_block,
                enclosing,
                kind: ItemKindInfo::AssocOrTypeAlias { end_byte: term_idx + 1 },
            })
        }
        _ => None,
    }
}

fn extract_vis_prefix(mods: &str) -> String {
    let t = mods.trim();
    if let Some(after_pub) = t.strip_prefix("pub") {
        let b = after_pub.as_bytes();
        if b.is_empty() || b[0].is_ascii_whitespace() {
            return "pub ".to_string();
        }
        if b[0] == b'(' {
            if let Some(end_p) = skip_balanced(b, 0, b'(', b')') {
                return format!("pub{} ", collapse_ws(&after_pub[..end_p]));
            }
        }
    }
    String::new()
}

fn skip_item_modifiers(src: &str, masked: &str, start: usize) -> (bool, usize, bool, bool) {
    let (_, pos, is_unsafe, abi) = parse_decl_modifiers(src, masked, start);
    (false, pos, is_unsafe, abi.is_some())
}

fn parse_decl_modifiers(src: &str, masked: &str, start: usize) -> (bool, usize, bool, Option<String>) {
    let mb = masked.as_bytes();
    let mut pos = start;
    let mut is_unsafe = false;
    let mut extern_abi = None;
    loop {
        while pos < mb.len() && mb[pos].is_ascii_whitespace() {
            pos += 1;
        }
        let Some((tok, tok_end)) = next_ident_token(masked, pos) else { break };
        match tok {
            "pub" => {
                pos = tok_end;
                let mut k = pos;
                while k < mb.len() && mb[k].is_ascii_whitespace() {
                    k += 1;
                }
                if mb.get(k).copied() == Some(b'(') {
                    if let Some(after_p) = skip_balanced(mb, k, b'(', b')') {
                        pos = after_p;
                    }
                }
            }
            "default" | "const" | "async" | "auto" => pos = tok_end,
            "unsafe" => {
                is_unsafe = true;
                pos = tok_end;
            }
            "extern" => {
                let abi_start = pos;
                pos = tok_end;
                let mut k = pos;
                while k < mb.len() && mb[k].is_ascii_whitespace() {
                    k += 1;
                }
                if src.as_bytes().get(k).copied() == Some(b'"') {
                    let mut q = k + 1;
                    while q < src.len() && src.as_bytes()[q] != b'"' {
                        q += 1;
                    }
                    if q < src.len() {
                        pos = q + 1;
                    }
                }
                extern_abi = Some(collapse_ws(src[abi_start..pos].trim()));
            }
            _ => break,
        }
    }
    (false, pos, is_unsafe, extern_abi)
}
