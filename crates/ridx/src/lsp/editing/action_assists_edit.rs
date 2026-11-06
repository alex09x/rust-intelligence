/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Code action and workspace edit objects for the declaration assists.

use super::action_raw_string::build_multi_edits_workspace_edit;
use serde_json::{json, Value};

#[allow(clippy::too_many_arguments)]
pub(super) fn make_assist_action(
    uri: &str,
    title: &str,
    lsp_kind: &str,
    data_kind: &str,
    cursor_line0: u32,
    cursor_col_u16: u32,
    ins_line0: u32,
    ins_col_u16: u32,
    new_text: &str,
    doc_version: Option<i32>,
) -> Value {
    let edit_obj = build_insertion_workspace_edit(uri, ins_line0, ins_col_u16, new_text, doc_version);
    json!({
        "title": title,
        "kind": lsp_kind,
        "data": {
            "uri": uri,
            "version": doc_version,
            "kind": data_kind,
            "cursor": { "line": cursor_line0, "character": cursor_col_u16 }
        },
        "edit": edit_obj
    })
}

/// Adds edits for files other than the requested document to a `WorkspaceEdit`, in `changes` and, when the edit is
/// versioned, as unversioned `documentChanges` entries.
pub(super) fn add_other_file_edits(edit: &mut Value, other_files: std::collections::BTreeMap<String, Vec<Value>>) {
    for (uri, edits) in other_files {
        if let Some(doc_changes) = edit.get_mut("documentChanges").and_then(Value::as_array_mut) {
            doc_changes.push(json!({ "textDocument": { "uri": uri, "version": null }, "edits": edits }));
        }
        edit["changes"][uri.as_str()] = json!(edits);
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn make_multi_edit_assist(
    uri: &str,
    title: &str,
    lsp_kind: &str,
    data_kind: &str,
    cursor_line0: u32,
    cursor_col_u16: u32,
    edits: Vec<Value>,
    doc_version: Option<i32>,
) -> Value {
    let edit_obj = build_multi_edits_workspace_edit(uri, edits, doc_version);
    json!({
        "title": title,
        "kind": lsp_kind,
        "data": {
            "uri": uri,
            "version": doc_version,
            "kind": data_kind,
            "cursor": { "line": cursor_line0, "character": cursor_col_u16 }
        },
        "edit": edit_obj
    })
}

pub(super) fn build_insertion_workspace_edit(uri: &str, line0: u32, col_u16: u32, new_text: &str, doc_version: Option<i32>) -> Value {
    let text_edit = json!({
        "range": {
            "start": { "line": line0, "character": col_u16 },
            "end": { "line": line0, "character": col_u16 }
        },
        "newText": new_text
    });
    build_multi_edits_workspace_edit(uri, vec![text_edit], doc_version)
}
