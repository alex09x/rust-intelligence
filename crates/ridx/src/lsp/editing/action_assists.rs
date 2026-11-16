/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Declaration-site and literal-site code actions and deferred `codeAction/resolve` builders.

use super::action_adt::{build_generate_impl, build_generate_new, build_generate_trait_impl};
use super::action_adt_traits::{build_default_from_new, build_enum_discriminants, build_single_field_from};
use super::action_assists_edit::{add_other_file_edits, build_insertion_workspace_edit, make_assist_action, make_multi_edit_assist};
use super::action_default_members::build_implement_default_members;
use super::action_doc_template::{build_doc_template, indexed_item};
use super::action_extract_lit::{collect_extract_literal_assists, is_extract_lit_data_kind, resolve_extract_literal_assist};
use super::action_generics::build_fn_type_aliases;
use super::action_inline_callers::build_inline_into_all_callers_workspace;
use super::action_inline_local::{build_inline_into_all_callers, build_inline_type_alias, build_nested_fn_to_closure};
use super::action_parse::{line_starts_of, mask_comments_and_strings, utf16_pos_to_byte, ItemKindInfo};
use super::action_parse_enclosing::find_enclosing_fn_at_cursor;
use super::action_parse_items::parse_item_with_masked;
use super::action_raw_string::{build_multi_edits_workspace_edit, collect_string_literal_assists, is_string_assist_data_kind, resolve_string_assist};
use super::action_struct_conv::build_convert_to_tuple_struct;
use super::action_struct_conv_named::convert_to_named;
use crate::lsp::error::{LspError, LspResult};
use crate::lsp::protocol::uri_to_path;
use crate::overlay::OverlayManager;
use ridx_format::WorkspaceIndex;
use serde_json::Value;

pub const KIND_DOC_TEMPLATE: &str = "refactor.doc_template";
pub const KIND_FN_ALIAS_NAMED: &str = "refactor.fn_type_alias_named";
pub const KIND_FN_ALIAS_UNNAMED: &str = "refactor.fn_type_alias_unnamed";
pub const KIND_IMPL_DEFAULT_MEMBERS: &str = "quickfix.implement_default_members";
pub const KIND_GENERATE_IMPL: &str = "refactor.generate_impl";
pub const KIND_GENERATE_TRAIT_IMPL: &str = "refactor.generate_trait_impl";
pub const KIND_GENERATE_NEW: &str = "refactor.generate_new";
pub const KIND_SINGLE_FIELD_FROM: &str = "refactor.single_field_from";
pub const KIND_DEFAULT_FROM_NEW: &str = "refactor.default_from_new";
pub const KIND_ENUM_DISCRIMINANTS: &str = "refactor.enum_discriminants";
pub const KIND_CONV_TUPLE_STRUCT: &str = "refactor.conv_tuple_struct";
pub const KIND_CONV_NAMED_STRUCT: &str = "refactor.conv_named_struct";
pub const KIND_NESTED_FN_CLOSURE: &str = "refactor.nested_fn_closure";
pub const KIND_INLINE_TYPE_ALIAS: &str = "refactor.inline_type_alias";
pub const KIND_INLINE_ALL_CALLERS: &str = "refactor.inline_all_callers";

pub fn is_decl_assist_data_kind(kind: &str) -> bool {
    is_string_assist_data_kind(kind)
        || is_extract_lit_data_kind(kind)
        || matches!(
            kind,
            KIND_DOC_TEMPLATE
                | KIND_FN_ALIAS_NAMED
                | KIND_FN_ALIAS_UNNAMED
                | KIND_IMPL_DEFAULT_MEMBERS
                | KIND_GENERATE_IMPL
                | KIND_GENERATE_TRAIT_IMPL
                | KIND_GENERATE_NEW
                | KIND_SINGLE_FIELD_FROM
                | KIND_DEFAULT_FROM_NEW
                | KIND_ENUM_DISCRIMINANTS
                | KIND_CONV_TUPLE_STRUCT
                | KIND_CONV_NAMED_STRUCT
                | KIND_NESTED_FN_CLOSURE
                | KIND_INLINE_TYPE_ALIAS
                | KIND_INLINE_ALL_CALLERS
        )
}

#[allow(clippy::too_many_arguments)]
pub fn collect_decl_assists<F>(
    doc_text: &str,
    uri: &str,
    range: &Value,
    doc_version: Option<i32>,
    ws: &WorkspaceIndex,
    overlay_mgr: &OverlayManager,
    matches_kind: &F,
) -> Vec<Value>
where
    F: Fn(&str) -> bool,
{
    let Some(line0) = range["start"]["line"].as_u64().map(|v| v as u32) else {
        return Vec::new();
    };
    let Some(col_u16) = range["start"]["character"].as_u64().map(|v| v as u32) else {
        return Vec::new();
    };
    let end_line0 = range["end"]["line"].as_u64().map(|v| v as u32).unwrap_or(line0);
    let end_col = range["end"]["character"].as_u64().map(|v| v as u32).unwrap_or(col_u16);
    if end_line0 != line0 {
        return Vec::new();
    }

    let masked_doc = mask_comments_and_strings(doc_text);
    let starts = line_starts_of(doc_text);
    let Some(cursor_byte) = utf16_pos_to_byte(doc_text, &starts, line0, col_u16) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    if end_col == col_u16 {
        out.extend(collect_string_literal_assists(doc_text, &masked_doc, &starts, uri, line0, col_u16, cursor_byte, doc_version, matches_kind));
        out.extend(collect_extract_literal_assists(doc_text, &masked_doc, &starts, uri, line0, col_u16, cursor_byte, doc_version, ws, matches_kind));
        if !out.is_empty() {
            return out;
        }
    }

    let (item, is_enclosing_fn) = if let Some(it) = parse_item_with_masked(doc_text, &masked_doc, &starts, cursor_byte) {
        (it, false)
    } else if let Some(it) = find_enclosing_fn_at_cursor(doc_text, &masked_doc, &starts, cursor_byte) {
        (it, true)
    } else {
        return out;
    };
    let file_path = uri_to_path(uri);

    match &item.kind {
        ItemKindInfo::Fn { .. } => {
            if !is_enclosing_fn {
                if matches_kind("quickfix") {
                    if let Some((ins_line, ins_col, text)) = build_implement_default_members(doc_text, &masked_doc, &starts, &file_path, &item, ws, overlay_mgr) {
                        out.push(make_assist_action(
                            uri,
                            "Implement default members",
                            "quickfix",
                            KIND_IMPL_DEFAULT_MEMBERS,
                            line0,
                            col_u16,
                            ins_line,
                            ins_col,
                            &text,
                            doc_version,
                        ));
                    }
                }
                if matches_kind("refactor.rewrite") {
                    if let Some((ins_line, ins_col, doc_ins)) = build_doc_template(&masked_doc, &item, indexed_item(ws, &file_path, doc_text, &starts, &item)) {
                        out.push(make_assist_action(
                            uri,
                            "Generate a documentation template",
                            "refactor.rewrite",
                            KIND_DOC_TEMPLATE,
                            line0,
                            col_u16,
                            ins_line,
                            ins_col,
                            &doc_ins,
                            doc_version,
                        ));
                    }
                    if let Some(plan) = build_fn_type_aliases(doc_text, &masked_doc, &file_path, &item, ws) {
                        out.push(make_assist_action(
                            uri,
                            "Generate a type alias for function with named params",
                            "refactor.rewrite",
                            KIND_FN_ALIAS_NAMED,
                            line0,
                            col_u16,
                            plan.insert_line0,
                            0,
                            &plan.named_text,
                            doc_version,
                        ));
                        out.push(make_assist_action(
                            uri,
                            "Generate a type alias for function with unnamed params",
                            "refactor.rewrite",
                            KIND_FN_ALIAS_UNNAMED,
                            line0,
                            col_u16,
                            plan.insert_line0,
                            0,
                            &plan.unnamed_text,
                            doc_version,
                        ));
                    }
                    if let Some((ins_line, ins_col, text)) = build_default_from_new(doc_text, &masked_doc, &starts, &item, ws) {
                        out.push(make_assist_action(
                            uri,
                            "Generate a Default impl from a new fn",
                            "refactor.rewrite",
                            KIND_DEFAULT_FROM_NEW,
                            line0,
                            col_u16,
                            ins_line,
                            ins_col,
                            &text,
                            doc_version,
                        ));
                    }
                    if let Some(edits) = build_nested_fn_to_closure(doc_text, &masked_doc, &starts, &item) {
                        out.push(make_multi_edit_assist(uri, "Convert nested function to closure", "refactor.rewrite", KIND_NESTED_FN_CLOSURE, line0, col_u16, edits, doc_version));
                    }
                }
            }
            // On the function's own declaration only, as rust-analyzer offers it: from anywhere in its body Zed listed
            // `Inline into all callers` of the enclosing function on every line.
            if !is_enclosing_fn && matches_kind("refactor.inline") {
                if let Some((edits, other_files)) = build_inline_into_all_callers_workspace(doc_text, &masked_doc, &starts, &file_path, &item, Some(ws)) {
                    let mut action = make_multi_edit_assist(uri, "Inline into all callers", "refactor.inline", KIND_INLINE_ALL_CALLERS, line0, col_u16, edits, doc_version);
                    if let Some(edit_obj) = action.get_mut("edit") {
                        add_other_file_edits(edit_obj, other_files);
                    }
                    out.push(action);
                } else if let Some(edits) = build_inline_into_all_callers(doc_text, &masked_doc, &starts, &item) {
                    out.push(make_multi_edit_assist(uri, "Inline into all callers", "refactor.inline", KIND_INLINE_ALL_CALLERS, line0, col_u16, edits, doc_version));
                }
            }
        }
        ItemKindInfo::AssocOrTypeAlias { .. } => {
            if matches_kind("quickfix") {
                if let Some((ins_line, ins_col, text)) = build_implement_default_members(doc_text, &masked_doc, &starts, &file_path, &item, ws, overlay_mgr) {
                    out.push(make_assist_action(uri, "Implement default members", "quickfix", KIND_IMPL_DEFAULT_MEMBERS, line0, col_u16, ins_line, ins_col, &text, doc_version));
                }
            }
            if matches_kind("refactor.inline") {
                if let Some(edits) = build_inline_type_alias(doc_text, &masked_doc, &starts, &item) {
                    out.push(make_multi_edit_assist(uri, "Inline type alias into all uses", "refactor.inline", KIND_INLINE_TYPE_ALIAS, line0, col_u16, edits, doc_version));
                }
            }
        }
        ItemKindInfo::Adt { .. } => {
            if matches_kind("refactor.rewrite") {
                if let Some((ins_line, ins_col, text)) = build_generate_impl(doc_text, &starts, &item) {
                    out.push(make_assist_action(
                        uri,
                        &format!("Generate impl for `{}`", item.name),
                        "refactor.rewrite",
                        KIND_GENERATE_IMPL,
                        line0,
                        col_u16,
                        ins_line,
                        ins_col,
                        &text,
                        doc_version,
                    ));
                }
                if let Some((ins_line, ins_col, text)) = build_generate_trait_impl(doc_text, &starts, &item) {
                    out.push(make_assist_action(
                        uri,
                        &format!("Generate trait impl for `{}`", item.name),
                        "refactor.rewrite",
                        KIND_GENERATE_TRAIT_IMPL,
                        line0,
                        col_u16,
                        ins_line,
                        ins_col,
                        &text,
                        doc_version,
                    ));
                }
                if let Some((ins_line, ins_col, text)) = build_generate_new(doc_text, &masked_doc, &starts, &file_path, &item, ws) {
                    out.push(make_assist_action(uri, "Generate `new`", "refactor.rewrite", KIND_GENERATE_NEW, line0, col_u16, ins_line, ins_col, &text, doc_version));
                }
                if let Some((ins_line, ins_col, text)) = build_single_field_from(doc_text, &masked_doc, &starts, &file_path, &item) {
                    out.push(make_assist_action(
                        uri,
                        "Generate single field `From`",
                        "refactor.rewrite",
                        KIND_SINGLE_FIELD_FROM,
                        line0,
                        col_u16,
                        ins_line,
                        ins_col,
                        &text,
                        doc_version,
                    ));
                }
                let to_tuple = build_convert_to_tuple_struct(doc_text, &masked_doc, &starts, &item, Some(ws), &file_path);
                let to_named = convert_to_named(doc_text, &masked_doc, &starts, &item, ws, &file_path);
                for (title, kind, conv) in [("Convert to tuple struct", KIND_CONV_TUPLE_STRUCT, to_tuple), ("Convert to named struct", KIND_CONV_NAMED_STRUCT, to_named)] {
                    let Some((edits, other_files)) = conv else { continue };
                    let mut action = make_multi_edit_assist(uri, title, "refactor.rewrite", kind, line0, col_u16, edits, doc_version);
                    add_other_file_edits(&mut action["edit"], other_files);
                    out.push(action);
                }
                if let Some(edits) = build_enum_discriminants(doc_text, &masked_doc, &starts, &item) {
                    out.push(make_multi_edit_assist(uri, "Add explicit enum discriminants", "refactor.rewrite", KIND_ENUM_DISCRIMINANTS, line0, col_u16, edits, doc_version));
                }
            }
        }
    }

    out
}

pub fn resolve_decl_assist(
    data_kind: &str,
    data: &Value,
    doc_text: &str,
    uri: &str,
    doc_version: Option<i32>,
    ws: Option<&WorkspaceIndex>,
    overlay_mgr: &OverlayManager,
) -> LspResult<Value> {
    let ws = ws.ok_or_else(|| LspError::request_failed("Workspace index required to resolve code action"))?;
    let line0 = data["cursor"]["line"].as_u64().ok_or_else(|| LspError::invalid_params("data.cursor.line missing"))? as u32;
    let col_u16 = data["cursor"]["character"].as_u64().ok_or_else(|| LspError::invalid_params("data.cursor.character missing"))? as u32;

    let masked_doc = mask_comments_and_strings(doc_text);
    let starts = line_starts_of(doc_text);
    let cursor_byte = utf16_pos_to_byte(doc_text, &starts, line0, col_u16).ok_or_else(|| LspError::request_failed("Invalid cursor offset"))?;

    if is_string_assist_data_kind(data_kind) {
        return resolve_string_assist(data_kind, doc_text, &masked_doc, &starts, uri, cursor_byte, doc_version)
            .ok_or_else(|| LspError::request_failed("Cannot resolve string literal assist"));
    }
    if is_extract_lit_data_kind(data_kind) {
        return resolve_extract_literal_assist(data_kind, doc_text, &masked_doc, &starts, uri, cursor_byte, doc_version, ws)
            .ok_or_else(|| LspError::request_failed("Cannot resolve extract literal assist"));
    }

    let item = parse_item_with_masked(doc_text, &masked_doc, &starts, cursor_byte)
        .or_else(|| find_enclosing_fn_at_cursor(doc_text, &masked_doc, &starts, cursor_byte))
        .ok_or_else(|| LspError::request_failed("Declaration at cursor could not be parsed"))?;
    let file_path = uri_to_path(uri);

    match data_kind {
        KIND_DOC_TEMPLATE => {
            let (l, c, text) = build_doc_template(&masked_doc, &item, indexed_item(ws, &file_path, doc_text, &starts, &item))
                .ok_or_else(|| LspError::request_failed("Cannot build documentation template"))?;
            Ok(build_insertion_workspace_edit(uri, l, c, &text, doc_version))
        }
        KIND_FN_ALIAS_NAMED => {
            let plan = build_fn_type_aliases(doc_text, &masked_doc, &file_path, &item, ws).ok_or_else(|| LspError::request_failed("Cannot build named function type alias"))?;
            Ok(build_insertion_workspace_edit(uri, plan.insert_line0, 0, &plan.named_text, doc_version))
        }
        KIND_FN_ALIAS_UNNAMED => {
            let plan = build_fn_type_aliases(doc_text, &masked_doc, &file_path, &item, ws).ok_or_else(|| LspError::request_failed("Cannot build unnamed function type alias"))?;
            Ok(build_insertion_workspace_edit(uri, plan.insert_line0, 0, &plan.unnamed_text, doc_version))
        }
        KIND_IMPL_DEFAULT_MEMBERS => {
            let (l, c, t) = build_implement_default_members(doc_text, &masked_doc, &starts, &file_path, &item, ws, overlay_mgr)
                .ok_or_else(|| LspError::request_failed("Cannot resolve default trait members"))?;
            Ok(build_insertion_workspace_edit(uri, l, c, &t, doc_version))
        }
        KIND_GENERATE_IMPL => {
            let (l, c, t) = build_generate_impl(doc_text, &starts, &item).ok_or_else(|| LspError::request_failed("Cannot generate impl block"))?;
            Ok(build_insertion_workspace_edit(uri, l, c, &t, doc_version))
        }
        KIND_GENERATE_TRAIT_IMPL => {
            let (l, c, t) = build_generate_trait_impl(doc_text, &starts, &item).ok_or_else(|| LspError::request_failed("Cannot generate trait impl block"))?;
            Ok(build_insertion_workspace_edit(uri, l, c, &t, doc_version))
        }
        KIND_GENERATE_NEW => {
            let (l, c, t) =
                build_generate_new(doc_text, &masked_doc, &starts, &file_path, &item, ws).ok_or_else(|| LspError::request_failed("Cannot generate `new` constructor"))?;
            Ok(build_insertion_workspace_edit(uri, l, c, &t, doc_version))
        }
        KIND_SINGLE_FIELD_FROM => {
            let (l, c, t) =
                build_single_field_from(doc_text, &masked_doc, &starts, &file_path, &item).ok_or_else(|| LspError::request_failed("Cannot generate single field `From`"))?;
            Ok(build_insertion_workspace_edit(uri, l, c, &t, doc_version))
        }
        KIND_DEFAULT_FROM_NEW => {
            let (l, c, t) = build_default_from_new(doc_text, &masked_doc, &starts, &item, ws).ok_or_else(|| LspError::request_failed("Cannot generate `Default` impl"))?;
            Ok(build_insertion_workspace_edit(uri, l, c, &t, doc_version))
        }
        KIND_ENUM_DISCRIMINANTS => {
            let edits = build_enum_discriminants(doc_text, &masked_doc, &starts, &item).ok_or_else(|| LspError::request_failed("Cannot add explicit enum discriminants"))?;
            Ok(build_multi_edits_workspace_edit(uri, edits, doc_version))
        }
        KIND_CONV_TUPLE_STRUCT => {
            let (edits, other_files) = build_convert_to_tuple_struct(doc_text, &masked_doc, &starts, &item, Some(ws), &file_path)
                .ok_or_else(|| LspError::request_failed("Cannot convert to tuple struct"))?;
            let mut edit = build_multi_edits_workspace_edit(uri, edits, doc_version);
            add_other_file_edits(&mut edit, other_files);
            Ok(edit)
        }
        KIND_CONV_NAMED_STRUCT => {
            let (edits, other_files) =
                convert_to_named(doc_text, &masked_doc, &starts, &item, ws, &file_path).ok_or_else(|| LspError::request_failed("Cannot convert to named struct"))?;
            let mut edit = build_multi_edits_workspace_edit(uri, edits, doc_version);
            add_other_file_edits(&mut edit, other_files);
            Ok(edit)
        }
        KIND_NESTED_FN_CLOSURE => {
            let edits = build_nested_fn_to_closure(doc_text, &masked_doc, &starts, &item).ok_or_else(|| LspError::request_failed("Cannot convert nested function to closure"))?;
            Ok(build_multi_edits_workspace_edit(uri, edits, doc_version))
        }
        KIND_INLINE_TYPE_ALIAS => {
            let edits = build_inline_type_alias(doc_text, &masked_doc, &starts, &item).ok_or_else(|| LspError::request_failed("Cannot inline type alias"))?;
            Ok(build_multi_edits_workspace_edit(uri, edits, doc_version))
        }
        KIND_INLINE_ALL_CALLERS => {
            if let Some((edits, other_files)) = build_inline_into_all_callers_workspace(doc_text, &masked_doc, &starts, &file_path, &item, Some(ws)) {
                let mut edit = build_multi_edits_workspace_edit(uri, edits, doc_version);
                add_other_file_edits(&mut edit, other_files);
                Ok(edit)
            } else {
                let edits = build_inline_into_all_callers(doc_text, &masked_doc, &starts, &item).ok_or_else(|| LspError::request_failed("Cannot inline function into callers"))?;
                Ok(build_multi_edits_workspace_edit(uri, edits, doc_version))
            }
        }
        _ => Err(LspError::invalid_params(format!("Unknown declaration assist kind: {data_kind}"))),
    }
}
