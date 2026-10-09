/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Code actions, quickfixes, and deferred action resolution.

use crate::lsp::error::{LspError, LspResult};
use crate::lsp::protocol::uri_to_path;
pub use super::atomic_write::{is_readonly_dependency, validate_workspace_edit};
use crate::overlay::OverlayManager;
use ridx_format::WorkspaceIndex;
use serde_json::{json, Value};
use std::path::Path;

pub fn handle_code_action(
    msg: &Value,
    workspace: Option<&WorkspaceIndex>,
    overlay_mgr: &OverlayManager,
) -> LspResult<Value> {
    let params = msg
        .get("params")
        .ok_or_else(|| LspError::invalid_params("Missing params object"))?;

    let uri = params["textDocument"]["uri"]
        .as_str()
        .ok_or_else(|| LspError::invalid_params("Missing textDocument.uri"))?;
    if uri.is_empty() {
        return Err(LspError::invalid_params("Empty textDocument.uri"));
    }

    let range = &params["range"];
    if !range.is_object() {
        return Err(LspError::invalid_params("Missing range object"));
    }

    let file = uri_to_path(uri);
    if is_readonly_dependency(Path::new(&file), workspace) {
        return Err(LspError::request_failed(format!(
            "Cannot perform code action on external dependency: {file}"
        )));
    }

    let (doc_text, doc_version) = if let Some(overlay) = overlay_mgr.get(&file) {
        (overlay.text.clone(), Some(overlay.version))
    } else {
        (
            std::fs::read_to_string(&file).unwrap_or_default(),
            None,
        )
    };

    let context = &params["context"];
    let only_kinds: Option<Vec<&str>> = context["only"]
        .as_array()
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect());

    let matches_kind = |kind: &str| -> bool {
        match &only_kinds {
            None => true,
            Some(list) => list.iter().any(|k| kind.starts_with(*k) || *k == kind),
        }
    };

    let mut actions = Vec::new();

    if matches_kind("source.organizeImports") {
        if let Some(action) = super::organize::organize_imports_action(&doc_text, uri, doc_version)
        {
            validate_workspace_edit(&action["edit"], workspace)?;
            actions.push(action);
        }
    }

    let is_range_empty = range["start"] == range["end"];
    if !is_range_empty && matches_kind("refactor.extract") {
        if let Some(action) =
            super::extract::try_extract_variable_action(&doc_text, uri, range, doc_version)
        {
            validate_workspace_edit(&action["edit"], workspace)?;
            actions.push(action);
        }
    }

    if matches_kind("quickfix") {
        if let Some(diagnostics) = context["diagnostics"].as_array() {
            for diag in diagnostics {
                let code = diag["code"].as_str().unwrap_or("");
                let message = diag["message"].as_str().unwrap_or("");
                let diag_range = &diag["range"];

                if code == "unused_variable" || message.contains("unused variable") {
                    if let Some(var_name) = message.split('`').nth(1) {
                        let action = make_quickfix_action(
                            uri,
                            &format!("Prefix with underscore: `_{var_name}`"),
                            diag_range,
                            &format!("_{var_name}"),
                            doc_version,
                            diag,
                        );
                        validate_workspace_edit(&action["edit"], workspace)?;
                        actions.push(action);
                    }
                } else if code == "unused_mut" || message.contains("variable does not need to be mutable") {
                    let action = make_quickfix_action(
                        uri,
                        "Remove unnecessary `mut`",
                        diag_range,
                        "",
                        doc_version,
                        diag,
                    );
                    validate_workspace_edit(&action["edit"], workspace)?;
                    actions.push(action);
                } else if (code == "E0425" || code == "E0412" || message.contains("cannot find"))
                    && workspace.is_some()
                {
                    if let Some(ident) = message.split('`').nth(1) {
                        if let Some(ws) = workspace {
                            let mut seen_paths = std::collections::HashSet::new();
                            for s in ws.search_symbols(ident) {
                                if s.name == ident && seen_paths.insert(s.path.to_string()) {
                                    let action = make_auto_import_action(
                                        uri,
                                        s.path,
                                        doc_version,
                                        diag,
                                    );
                                    validate_workspace_edit(&action["edit"], workspace)?;
                                    actions.push(action);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let content_hash = ridx_format::normalized_source_hash(doc_text.as_bytes());
    for action in &mut actions {
        if let Some(data) = action.get_mut("data").and_then(|d| d.as_object_mut()) {
            data.insert("content_hash".to_string(), json!(content_hash));
        }
    }

    Ok(json!(actions))
}

pub fn handle_code_action_resolve(
    msg: &Value,
    workspace: Option<&WorkspaceIndex>,
    overlay_mgr: &OverlayManager,
) -> LspResult<Value> {
    let params = msg
        .get("params")
        .ok_or_else(|| LspError::invalid_params("Missing params object"))?;

    if !params.is_object() {
        return Err(LspError::invalid_params("Params must be a CodeAction object"));
    }

    let _title = params
        .get("title")
        .and_then(|t| t.as_str())
        .ok_or_else(|| LspError::invalid_params("CodeAction must contain a title"))?;

    // Check version conflict if data contains uri and version / content_hash
    if let Some(data) = params.get("data") {
        if let Some(uri) = data.get("uri").and_then(|u| u.as_str()) {
            let file = uri_to_path(uri);
            if let Some(ov) = overlay_mgr.get(&file) {
                if let Some(expected_ver) = data.get("version").and_then(|v| v.as_i64()) {
                    if ov.version as i64 != expected_ver {
                        return Err(LspError::content_modified(format!(
                            "Document '{uri}' was modified since code action was computed (version {} != expected {})",
                            ov.version, expected_ver
                        )));
                    }
                }
            } else if let Some(expected_hash) = data.get("content_hash").and_then(|h| h.as_u64()) {
                let disk_bytes = std::fs::read(&file).unwrap_or_default();
                let current_hash = ridx_format::normalized_source_hash(&disk_bytes);
                if current_hash != expected_hash {
                    return Err(LspError::content_modified(format!(
                        "Document '{uri}' on disk was modified since code action was computed",
                    )));
                }
            }
        }
    }

    // If edit is already attached, validate and return
    if let Some(edit) = params.get("edit") {
        validate_workspace_edit(edit, workspace)?;
        return Ok(params.clone());
    }

    // Resolve deferred edit from data if present
    let Some(data) = params.get("data") else {
        return Ok(params.clone());
    };

    let uri = data
        .get("uri")
        .and_then(|u| u.as_str())
        .ok_or_else(|| LspError::invalid_params("data missing uri"))?;

    let file = uri_to_path(uri);
    if is_readonly_dependency(Path::new(&file), workspace) {
        return Err(LspError::request_failed(format!(
            "Cannot modify read-only external dependency: {file}"
        )));
    }

    let (doc_text, live_ver) = if let Some(ov) = overlay_mgr.get(&file) {
        (ov.text.clone(), Some(ov.version))
    } else {
        (
            std::fs::read_to_string(&file).unwrap_or_default(),
            None,
        )
    };

    let kind = data.get("kind").and_then(|k| k.as_str()).unwrap_or("");
    let mut resolved = params.clone();

    if kind == "source.organizeImports" {
        let action = super::organize::organize_imports_action(&doc_text, uri, live_ver)
            .ok_or_else(|| LspError::request_failed("No imports to organize"))?;
        resolved["edit"] = action["edit"].clone();
    } else if kind == "refactor.extract" {
        let range = data
            .get("range")
            .ok_or_else(|| LspError::invalid_params("data missing range"))?;
        let action = super::extract::try_extract_variable_action(&doc_text, uri, range, live_ver)
            .ok_or_else(|| LspError::request_failed("Cannot extract variable at range"))?;
        resolved["edit"] = action["edit"].clone();
    } else if kind == "quickfix.auto_import" {
        let import_path = data
            .get("import_path")
            .and_then(|p| p.as_str())
            .ok_or_else(|| LspError::invalid_params("data missing import_path"))?;
        let action = make_auto_import_action(uri, import_path, live_ver, &Value::Null);
        resolved["edit"] = action["edit"].clone();
    } else if kind == "quickfix" {
        let range = data
            .get("range")
            .ok_or_else(|| LspError::invalid_params("data missing range"))?;
        let new_text = data
            .get("newText")
            .and_then(|t| t.as_str())
            .unwrap_or("");
        let edit = json!({
            "range": range,
            "newText": new_text
        });
        resolved["edit"] = json!({
            "changes": { uri: [edit] }
        });
    } else {
        return Err(LspError::invalid_params(format!(
            "Cannot resolve CodeAction with unknown data kind: '{kind}'"
        )));
    }

    validate_workspace_edit(&resolved["edit"], workspace)?;
    Ok(resolved)
}

fn wrap_single_edit_action(
    uri: &str,
    title: &str,
    kind: &str,
    data: Value,
    edit: Value,
    doc_version: Option<i32>,
    diag: &Value,
) -> Value {
    let mut edit_obj = json!({ "changes": { uri: [edit.clone()] } });
    if let Some(ver) = doc_version {
        edit_obj["documentChanges"] = json!([{
            "textDocument": { "uri": uri, "version": ver },
            "edits": [edit]
        }]);
    }
    let mut action = json!({
        "title": title,
        "kind": kind,
        "isPreferred": true,
        "data": data,
        "edit": edit_obj,
    });
    if !diag.is_null() {
        action["diagnostics"] = json!([diag]);
    }
    action
}

fn make_quickfix_action(
    uri: &str,
    title: &str,
    range: &Value,
    new_text: &str,
    doc_version: Option<i32>,
    diag: &Value,
) -> Value {
    let edit = json!({ "range": range, "newText": new_text });
    wrap_single_edit_action(
        uri,
        title,
        "quickfix",
        json!({
            "uri": uri,
            "version": doc_version,
            "kind": "quickfix",
            "range": range,
            "newText": new_text
        }),
        edit,
        doc_version,
        diag,
    )
}

fn make_auto_import_action(
    uri: &str,
    import_path: &str,
    doc_version: Option<i32>,
    diag: &Value,
) -> Value {
    let zero_range = json!({
        "start": { "line": 0, "character": 0 },
        "end": { "line": 0, "character": 0 }
    });
    let edit = json!({
        "range": zero_range,
        "newText": format!("use {import_path};\n")
    });
    wrap_single_edit_action(
        uri,
        &format!("Import `{import_path}`"),
        "quickfix",
        json!({
            "uri": uri,
            "version": doc_version,
            "kind": "quickfix.auto_import",
            "import_path": import_path
        }),
        edit,
        doc_version,
        diag,
    )
}
