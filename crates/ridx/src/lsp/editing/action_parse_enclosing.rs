/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Syntactic parser to resolve the innermost enclosing function item for a cursor anywhere inside it.

use super::action_parse::{EnclosingContext, ParsedCursorItem};
use super::action_parse_decl::{classify_enclosing_brace, next_ident_token, parse_decl_modifiers, parse_declaration_at};
use super::action_parse_items::{collect_enclosing_delims, contains_word, find_prev_item_boundary, is_macro_delimiter, scan_outer_attrs_and_docs};

pub fn find_enclosing_fn_at_cursor(src: &str, masked: &str, starts: &[usize], cursor_byte: usize) -> Option<ParsedCursorItem> {
    if src.is_empty() || cursor_byte > src.len() {
        return None;
    }
    let mb = masked.as_bytes();
    let enclosing_spans = collect_enclosing_delims(mb, cursor_byte)?;
    for sp in &enclosing_spans {
        if is_macro_delimiter(mb, sp.open_byte) {
            return None;
        }
    }

    let mut target_fn_open = None;
    let mut outer_brace = None;

    for (idx, sp) in enclosing_spans.iter().enumerate().rev() {
        if sp.ch == b'{' {
            let hf = find_prev_item_boundary(mb, sp.open_byte);
            let (_, hkw, _, _, _, _) = scan_outer_attrs_and_docs(src, masked, starts, hf, sp.open_byte);
            if hkw < sp.open_byte && contains_word(&masked[hkw..sp.open_byte], "fn") {
                target_fn_open = Some(sp.open_byte);
                outer_brace = enclosing_spans[..idx].iter().rev().find(|s| s.ch == b'{').copied();
                break;
            }
        } else if sp.ch == b'(' {
            let hf = find_prev_item_boundary(mb, sp.open_byte);
            let (_, hkw, _, _, _, _) = scan_outer_attrs_and_docs(src, masked, starts, hf, sp.open_byte);
            if hkw < sp.open_byte && contains_word(&masked[hkw..sp.open_byte], "fn") {
                let mut p = sp.close_byte + 1;
                while p < mb.len() && mb[p] != b'{' && mb[p] != b';' {
                    p += 1;
                }
                if p < mb.len() && mb[p] == b'{' {
                    target_fn_open = Some(p);
                    outer_brace = enclosing_spans[..idx].iter().rev().find(|s| s.ch == b'{').copied();
                    break;
                }
            }
        }
    }

    let fn_open_byte = target_fn_open?;
    let (enclosing, enclosing_block) = if let Some(br) = outer_brace {
        let hdr_floor = find_prev_item_boundary(mb, br.open_byte);
        let (_, hdr_kw_start, cfg_attrs, insert_line0, indent, _) = scan_outer_attrs_and_docs(src, masked, starts, hdr_floor, br.open_byte);
        let ctx = classify_enclosing_brace(src, masked, hdr_kw_start, br.open_byte, br.close_byte, cfg_attrs, insert_line0, indent);
        (ctx, Some((br.open_byte, br.close_byte)))
    } else {
        (EnclosingContext::None, None)
    };

    let floor = outer_brace.map(|b| b.open_byte + 1).unwrap_or(0);
    let hf = find_prev_item_boundary(mb, fn_open_byte).max(floor);
    let (has_doc, kw_start, cfg_attrs, attr_start_line0, indent, first_attr_byte) = scan_outer_attrs_and_docs(src, masked, starts, hf, fn_open_byte);
    if kw_start >= fn_open_byte {
        return None;
    }

    let (_, pos_after_mods, _, _) = parse_decl_modifiers(src, masked, kw_start);
    let (kw, kw_end) = next_ident_token(masked, pos_after_mods)?;
    if kw != "fn" {
        return None;
    }
    let (raw_name_s, name_end) = next_ident_token(src, kw_end)?;
    let name_byte = name_end - raw_name_s.len();

    parse_declaration_at(src, masked, kw_start, first_attr_byte, name_byte, has_doc, cfg_attrs, attr_start_line0, indent, false, enclosing_block, enclosing)
}
