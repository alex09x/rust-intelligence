/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Cursor-in-string-literal `Extract into variable / constant / static` assists (`refactor.extract`).

use super::action_extract_lit_name::suggest_literal_var_name;
use super::action_generics_helpers::unique_ident;
use super::action_parse::byte_to_utf16_pos;
use super::action_parse_items::{contains_word, skip_balanced};
use super::action_raw_string::{build_multi_edits_workspace_edit, find_string_lit_at_cursor, is_inside_attr_or_extern_abi, ScannedStringLit};
use crate::lsp::protocol::uri_to_path;
use ridx_format::WorkspaceIndex;
use serde_json::{json, Value};

pub const KIND_EXTRACT_LIT_VAR: &str = "refactor.extract_lit_var";
pub const KIND_EXTRACT_LIT_CONST: &str = "refactor.extract_lit_const";
pub const KIND_EXTRACT_LIT_STATIC: &str = "refactor.extract_lit_static";

pub fn is_extract_lit_data_kind(k: &str) -> bool {
    matches!(k, KIND_EXTRACT_LIT_VAR | KIND_EXTRACT_LIT_CONST | KIND_EXTRACT_LIT_STATIC)
}

#[allow(clippy::too_many_arguments)]
pub fn collect_extract_literal_assists<F>(
    doc_text: &str,
    masked_doc: &str,
    starts: &[usize],
    uri: &str,
    cursor_line0: u32,
    cursor_col_u16: u32,
    cursor_byte: usize,
    doc_version: Option<i32>,
    ws: &WorkspaceIndex,
    matches_kind: &F,
) -> Vec<Value>
where
    F: Fn(&str) -> bool,
{
    if !matches_kind("refactor.extract") {
        return Vec::new();
    }
    let Some(plan) = plan_extract_literal(doc_text, masked_doc, cursor_byte, &uri_to_path(uri), ws) else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(3);
    for (title, dkind) in [("Extract into variable", KIND_EXTRACT_LIT_VAR), ("Extract into constant", KIND_EXTRACT_LIT_CONST), ("Extract into static", KIND_EXTRACT_LIT_STATIC)] {
        let edit_obj = render_extract_plan(&plan, dkind, doc_text, starts, uri, doc_version);
        out.push(json!({
            "title": title,
            "kind": "refactor.extract",
            "data": {
                "uri": uri,
                "version": doc_version,
                "kind": dkind,
                "cursor": { "line": cursor_line0, "character": cursor_col_u16 }
            },
            "edit": edit_obj
        }));
    }
    out
}

#[allow(clippy::too_many_arguments)]
pub fn resolve_extract_literal_assist(
    data_kind: &str,
    doc_text: &str,
    masked_doc: &str,
    starts: &[usize],
    uri: &str,
    cursor_byte: usize,
    doc_version: Option<i32>,
    ws: &WorkspaceIndex,
) -> Option<Value> {
    let plan = plan_extract_literal(doc_text, masked_doc, cursor_byte, &uri_to_path(uri), ws)?;
    Some(render_extract_plan(&plan, data_kind, doc_text, starts, uri, doc_version))
}

struct ExtractLitPlan {
    stmt_line_start: usize,
    indent: String,
    lit_start: usize,
    lit_end: usize,
    lit_src: String,
    ty_annot: &'static str,
    var_ident: String,
    upper_ident: String,
}

fn render_extract_plan(plan: &ExtractLitPlan, dkind: &str, doc_text: &str, starts: &[usize], uri: &str, doc_version: Option<i32>) -> Value {
    let (ident, decl_line) = match dkind {
        KIND_EXTRACT_LIT_CONST => (&plan.upper_ident, format!("{}const {}: {} = {};\n", plan.indent, plan.upper_ident, plan.ty_annot, plan.lit_src)),
        KIND_EXTRACT_LIT_STATIC => (&plan.upper_ident, format!("{}static {}: {} = {};\n", plan.indent, plan.upper_ident, plan.ty_annot, plan.lit_src)),
        _ => (&plan.var_ident, format!("{}let {} = {};\n", plan.indent, plan.var_ident, plan.lit_src)),
    };
    let (ins_l, ins_c) = byte_to_utf16_pos(doc_text, starts, plan.stmt_line_start);
    let (ls_l, ls_c) = byte_to_utf16_pos(doc_text, starts, plan.lit_start);
    let (le_l, le_c) = byte_to_utf16_pos(doc_text, starts, plan.lit_end);
    build_multi_edits_workspace_edit(
        uri,
        vec![
            json!({
                "range": { "start": { "line": ins_l, "character": ins_c }, "end": { "line": ins_l, "character": ins_c } },
                "newText": decl_line
            }),
            json!({
                "range": { "start": { "line": ls_l, "character": ls_c }, "end": { "line": le_l, "character": le_c } },
                "newText": ident
            }),
        ],
        doc_version,
    )
}

fn plan_extract_literal(doc_text: &str, masked_doc: &str, cursor_byte: usize, file: &str, ws: &WorkspaceIndex) -> Option<ExtractLitPlan> {
    let lit: ScannedStringLit = find_string_lit_at_cursor(doc_text, cursor_byte)?;
    if is_inside_attr_or_extern_abi(masked_doc, lit.start_byte) {
        return None;
    }
    let ty_annot = match lit.prefix {
        "" => "&str",
        "b" => "&[u8]",
        "c" => "&core::ffi::CStr",
        _ => return None,
    };

    let mb = masked_doc.as_bytes();
    let stack = active_delims_at(mb, lit.start_byte)?;
    for &(ch, open_b) in &stack {
        let prev_non_ws = masked_doc[..open_b].trim_end();
        if prev_non_ws.ends_with('!') && !(ch == b'[' && prev_non_ws.ends_with("#!")) {
            return None;
        }
    }

    let &(_, fn_open) = stack.iter().rev().find(|&&(ch, open_b)| ch == b'{' && brace_belongs_to_fn(masked_doc, open_b))?;
    let &(_, block_open) = stack.iter().rev().find(|&&(ch, open_b)| ch == b'{' && is_stmt_block_brace(masked_doc, open_b))?;
    if has_unbraced_closure_bar(&masked_doc[block_open + 1..lit.start_byte]) {
        return None;
    }
    if is_pattern_or_direct_binding_rhs(masked_doc, block_open + 1, lit.start_byte, lit.end_byte, lit.prefix) {
        return None;
    }

    let stmt_start = find_enclosing_stmt_start(masked_doc, block_open + 1, lit.start_byte);
    let stmt_line_start = doc_text[..stmt_start].rfind('\n').map(|p| p + 1).unwrap_or(0);
    if stmt_line_start <= block_open || !masked_doc[stmt_line_start..stmt_start].bytes().all(|b| b == b' ' || b == b'\t') {
        return None;
    }
    let indent: String = doc_text[stmt_line_start..].chars().take_while(|c| *c == ' ' || *c == '\t').collect();

    let fn_hdr_len = stmt_header_before_brace(masked_doc, fn_open).len();
    let fn_lo = fn_open.saturating_sub(fn_hdr_len);
    let fn_hi = skip_balanced(mb, fn_open, b'{', b'}').unwrap_or(doc_text.len());
    let lit_src = &doc_text[lit.start_byte..lit.end_byte];
    let base_snake = suggest_literal_var_name(doc_text, masked_doc, &stack, lit.start_byte, lit_src, file, ws).unwrap_or_else(|| "var_name".to_string());
    let var_ident = unique_ident(&base_snake, &doc_text[fn_lo..fn_hi]);
    let base_upper = base_snake.to_ascii_uppercase();
    let upper_ident = unique_ident(if base_upper.is_empty() { "VAR_NAME" } else { &base_upper }, doc_text);

    Some(ExtractLitPlan {
        stmt_line_start,
        indent,
        lit_start: lit.start_byte,
        lit_end: lit.end_byte,
        lit_src: doc_text[lit.start_byte..lit.end_byte].to_string(),
        ty_annot,
        var_ident,
        upper_ident,
    })
}

fn active_delims_at(mb: &[u8], target: usize) -> Option<Vec<(u8, usize)>> {
    let mut stack = Vec::new();
    for (idx, &ch) in mb.iter().enumerate().take(target.min(mb.len())) {
        match ch {
            b'{' | b'(' | b'[' => stack.push((ch, idx)),
            b'}' | b')' | b']' => {
                let exp = match ch {
                    b'}' => b'{',
                    b')' => b'(',
                    _ => b'[',
                };
                if stack.pop()?.0 != exp {
                    return None;
                }
            }
            _ => {}
        }
    }
    Some(stack)
}

fn brace_belongs_to_fn(masked_doc: &str, open_brace: usize) -> bool {
    contains_word(stmt_header_before_brace(masked_doc, open_brace), "fn")
}

fn is_stmt_block_brace(masked_doc: &str, open_brace: usize) -> bool {
    let hdr = stmt_header_before_brace(masked_doc, open_brace).trim();
    if hdr.is_empty() || contains_word(hdr, "fn") {
        return true;
    }
    if contains_word(hdr, "match") {
        return false;
    }
    let last_ch = hdr.as_bytes().last().copied().unwrap_or(0);
    if matches!(last_ch, b';' | b'{' | b'=' | b'>' | b'(' | b'[' | b',' | b'|') {
        return !hdr.ends_with("::") && !(last_ch == b'>' && !hdr.ends_with("=>"));
    }
    hdr.starts_with("if ")
        || hdr.starts_with("while ")
        || hdr.starts_with("for ")
        || hdr.ends_with("else")
        || hdr.ends_with("loop")
        || hdr.ends_with("unsafe")
        || hdr.ends_with("async")
}

fn stmt_header_before_brace(masked_doc: &str, open_brace: usize) -> &str {
    let mb = masked_doc.as_bytes();
    let (mut i, mut p, mut b) = (open_brace, 0i32, 0i32);
    while i > 0 {
        i -= 1;
        match mb[i] {
            b')' => p += 1,
            b'(' => p = (p - 1).max(0),
            b']' => b += 1,
            b'[' => b = (b - 1).max(0),
            b';' | b'{' | b'}' if p == 0 && b == 0 => return &masked_doc[i + 1..open_brace],
            _ => {}
        }
    }
    &masked_doc[..open_brace]
}

fn has_unbraced_closure_bar(slice: &str) -> bool {
    let b = slice.as_bytes();
    (0..b.len()).any(|i| b[i] == b'|' && b.get(i + 1).copied() != Some(b'|') && b.get(i + 1).copied() != Some(b'=') && (i == 0 || b[i - 1] != b'|'))
}

fn find_enclosing_stmt_start(masked_doc: &str, floor: usize, lit_start: usize) -> usize {
    let mb = masked_doc.as_bytes();
    let (mut p, mut b, mut c, mut last) = (0i32, 0i32, 0i32, floor);
    for (i, &ch) in mb.iter().enumerate().take(lit_start).skip(floor) {
        match ch {
            b'(' => p += 1,
            b')' => p = (p - 1).max(0),
            b'[' => b += 1,
            b']' => b = (b - 1).max(0),
            b'{' => c += 1,
            b'}' => {
                c = (c - 1).max(0);
                if p == 0 && b == 0 && c == 0 {
                    last = i + 1;
                }
            }
            b';' if p == 0 && b == 0 && c == 0 => last = i + 1,
            _ => {}
        }
    }
    let ws_skip = masked_doc[last..lit_start].len() - masked_doc[last..lit_start].trim_start().len();
    last + ws_skip
}

/// Whether the literal sits where extracting it changes nothing that compiles the same: a pattern (`"a" =>`, `"a" |`),
/// the whole initializer of a local `const`/`static` (an item's initializer cannot always read another static), or a
/// byte string bound by a typed `let` (the extracted `&[u8]` would not be the `&[u8; N]` the annotation may name).
/// A plain `let x = "a";` is extracted like rust-analyzer does: `let var_name = "a"; let x = var_name;`.
fn is_pattern_or_direct_binding_rhs(masked_doc: &str, floor: usize, lit_start: usize, lit_end: usize, lit_prefix: &str) -> bool {
    let stmt_s = find_enclosing_stmt_start(masked_doc, floor, lit_start);
    let prefix = masked_doc[stmt_s..lit_start].trim();
    let suffix = masked_doc[lit_end..].trim_start();
    if prefix.ends_with('=') && !prefix.ends_with("==") && !prefix.ends_with("!=") && suffix.starts_with(';') {
        if prefix.starts_with("const ") || prefix.starts_with("static ") {
            return true;
        }
        if prefix.starts_with("let ") && lit_prefix == "b" && prefix.contains(':') {
            return true;
        }
    }
    suffix.starts_with("=>") || (suffix.starts_with('|') && !suffix.starts_with("||"))
}
