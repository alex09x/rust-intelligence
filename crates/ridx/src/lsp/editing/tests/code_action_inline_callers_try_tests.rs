/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use crate::lsp::editing::actions::{handle_code_action_resolve, handle_code_action_with_options};
use crate::overlay::OverlayManager;
use ridx_format::{DefKind, WorkspaceIndex};
use ridx_test_support::fixture::FixtureIndexBuilder;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static INLINE_TRY_TEST_ID: AtomicU64 = AtomicU64::new(0);

fn apply_text_edits(original: &str, edits: &[Value]) -> String {
    let mut lines: Vec<String> = original.split('\n').map(|s| s.to_string()).collect();
    let mut sorted_edits = edits.to_vec();
    sorted_edits.sort_by(|a, b| {
        let la = a["range"]["start"]["line"].as_u64().unwrap();
        let ca = a["range"]["start"]["character"].as_u64().unwrap();
        let lb = b["range"]["start"]["line"].as_u64().unwrap();
        let cb = b["range"]["start"]["character"].as_u64().unwrap();
        (lb, cb).cmp(&(la, ca))
    });

    for edit in sorted_edits {
        let s_line = edit["range"]["start"]["line"].as_u64().unwrap() as usize;
        let s_col = edit["range"]["start"]["character"].as_u64().unwrap() as usize;
        let e_line = edit["range"]["end"]["line"].as_u64().unwrap() as usize;
        let e_col = edit["range"]["end"]["character"].as_u64().unwrap() as usize;
        let new_text = edit["newText"].as_str().unwrap();

        let prefix: String = lines[s_line].chars().take(s_col).collect();
        let suffix: String = lines[e_line].chars().skip(e_col).collect();
        let combined = format!("{prefix}{new_text}{suffix}");
        let new_sub_lines: Vec<String> = combined.split('\n').map(|s| s.to_string()).collect();

        lines.splice(s_line..=e_line, new_sub_lines);
    }
    lines.join("\n")
}

fn assert_rustc_compiles(source: &str, label: &str) {
    let id = INLINE_TRY_TEST_ID.fetch_add(1, Ordering::Relaxed);
    let tmp_dir = std::env::temp_dir().join(format!("ridx_try_c_{}_{}", std::process::id(), id));
    let _ = fs::create_dir_all(&tmp_dir);
    let src_file = tmp_dir.join("lib.rs");
    fs::write(&src_file, source).expect("Failed to write test source for rustc check");

    let status = Command::new("rustc").arg("--crate-type=lib").arg("--emit=metadata").arg(&src_file).arg("--out-dir").arg(&tmp_dir).output();

    let _ = fs::remove_dir_all(&tmp_dir);
    if let Ok(out) = status {
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            panic!("{label}: rustc compilation failed:\n{stderr}\n\nSource:\n{source}");
        }
    }
}

fn query_actions_with_multi_file_resolve(uri: &str, line: u32, character: u32, ws: &WorkspaceIndex, ov: &OverlayManager) -> Vec<(String, BTreeMap<String, Vec<Value>>)> {
    let req = json!({
        "params": {
            "textDocument": { "uri": uri },
            "range": {
                "start": { "line": line, "character": character },
                "end": { "line": line, "character": character }
            },
            "context": { "diagnostics": [] }
        }
    });
    let list = handle_code_action_with_options(&req, Some(ws), ov, true).unwrap();
    let arr = list.as_array().unwrap();
    let mut resolved_out = Vec::new();
    for item in arr {
        let title = item.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let resolve_req = json!({ "params": item });
        let resolved = handle_code_action_resolve(&resolve_req, Some(ws), ov).unwrap();
        let mut changes_map = BTreeMap::new();
        if let Some(changes) = resolved.get("edit").and_then(|e| e.get("changes")).and_then(|c| c.as_object()) {
            for (f_uri, edits_val) in changes {
                let edits: Vec<Value> = edits_val.as_array().cloned().unwrap_or_default();
                changes_map.insert(f_uri.clone(), edits);
            }
        }
        resolved_out.push((title, changes_map));
    }
    resolved_out
}

#[test]
fn test_inline_fn_with_question_mark_into_result_caller_compiles() {
    let id = INLINE_TRY_TEST_ID.fetch_add(1, Ordering::Relaxed);
    let temp_dir = std::env::temp_dir().join(format!("ridx_inline_try_{}_{}", std::process::id(), id));
    let _ = fs::create_dir_all(&temp_dir);
    let lib_file = temp_dir.join("lib.rs");
    let lib_str = lib_file.to_str().unwrap().to_string();

    let mut builder = FixtureIndexBuilder::new("test_try", "0.1.0");

    let code = concat!(
        "fn parse_num(s: &str) -> Result<i32, std::num::ParseIntError> {\n",
        "    s.parse::<i32>()\n",
        "}\n",
        "fn «add_one»(s: &str) -> Result<i32, std::num::ParseIntError> {\n",
        "    let val = parse_num(s)?;\n",
        "    Ok(val + 1)\n",
        "}\n",
        "pub fn caller(s: &str) -> Result<i32, std::num::ParseIntError> {\n",
        "    let res = add_one(s)?;\n",
        "    Ok(res * 2)\n",
        "}\n",
    );

    let (cleaned, def_ids) = builder.add_defs_with_ids(code, &lib_str, "test_try", DefKind::Fn);
    let _ = fs::write(&lib_file, &cleaned);

    builder.add_ref("test_try::add_one", &lib_str, 4, 4, 4, 11, Some(def_ids[0]), Some(&lib_str), 4, 4, DefKind::Fn);
    builder.add_ref("test_try::add_one", &lib_str, 9, 15, 9, 22, Some(def_ids[0]), Some(&lib_str), 4, 4, DefKind::Fn);

    let ridx_file = temp_dir.join("index.ridx");
    builder.write_to_file(&ridx_file);
    let mut ws = WorkspaceIndex::open(&ridx_file).unwrap();
    ws.set_workspace_root(Some(temp_dir.clone()));
    let mut ov = OverlayManager::new();
    let uri = format!("file://{}", lib_str);
    ov.open_document(&uri, &lib_str, 1, cleaned.clone());

    let actions = query_actions_with_multi_file_resolve(&uri, 3, 4, &ws, &ov);
    let inline_action = actions.iter().find(|(t, _)| t == "Inline into all callers");
    assert!(inline_action.is_some(), "Expected 'Inline into all callers' for try-operator fn with Result caller");

    let (_, changes) = inline_action.unwrap();
    let edits = changes.get(&uri).expect("File edits missing");
    let updated = apply_text_edits(&cleaned, edits);

    assert!(!updated.contains("fn add_one("), "Declaration should be deleted");
    assert!(!updated.contains("add_one(s)"), "Call site should be inlined");
    assert_rustc_compiles(&updated, "Inline into all callers: fn with question mark into Result caller compiles");
}

#[test]
fn test_inline_fn_with_question_mark_rejected_when_caller_does_not_return_result() {
    let id = INLINE_TRY_TEST_ID.fetch_add(1, Ordering::Relaxed);
    let temp_dir = std::env::temp_dir().join(format!("ridx_inline_try_rej_{}_{}", std::process::id(), id));
    let _ = fs::create_dir_all(&temp_dir);
    let lib_file = temp_dir.join("lib.rs");
    let lib_str = lib_file.to_str().unwrap().to_string();

    let mut builder = FixtureIndexBuilder::new("test_try_rej", "0.1.0");

    let code = concat!(
        "fn parse_num(s: &str) -> Result<i32, std::num::ParseIntError> {\n",
        "    s.parse::<i32>()\n",
        "}\n",
        "fn «add_one»(s: &str) -> Result<i32, std::num::ParseIntError> {\n",
        "    let val = parse_num(s)?;\n",
        "    Ok(val + 1)\n",
        "}\n",
        "pub fn non_result_caller(s: &str) {\n",
        "    let _ = add_one(s);\n",
        "}\n",
    );

    let (cleaned, def_ids) = builder.add_defs_with_ids(code, &lib_str, "test_try_rej", DefKind::Fn);
    let _ = fs::write(&lib_file, &cleaned);

    builder.add_ref("test_try_rej::add_one", &lib_str, 4, 4, 4, 11, Some(def_ids[0]), Some(&lib_str), 4, 4, DefKind::Fn);
    builder.add_ref("test_try_rej::add_one", &lib_str, 9, 13, 9, 20, Some(def_ids[0]), Some(&lib_str), 4, 4, DefKind::Fn);

    let ridx_file = temp_dir.join("index.ridx");
    builder.write_to_file(&ridx_file);
    let mut ws = WorkspaceIndex::open(&ridx_file).unwrap();
    ws.set_workspace_root(Some(temp_dir.clone()));
    let mut ov = OverlayManager::new();
    let uri = format!("file://{}", lib_str);
    ov.open_document(&uri, &lib_str, 1, cleaned.clone());

    let actions = query_actions_with_multi_file_resolve(&uri, 3, 4, &ws, &ov);
    let inline_action = actions.iter().find(|(t, _)| t == "Inline into all callers");
    assert!(inline_action.is_none(), "Should NOT offer 'Inline into all callers' when caller does not return Result");
}

#[test]
fn test_inline_fn_with_underscored_type_identifier() {
    let id = INLINE_TRY_TEST_ID.fetch_add(1, Ordering::Relaxed);
    let temp_dir = std::env::temp_dir().join(format!("ridx_inline_under_{}_{}", std::process::id(), id));
    let _ = fs::create_dir_all(&temp_dir);
    let lib_file = temp_dir.join("lib.rs");
    let lib_str = lib_file.to_str().unwrap().to_string();

    let mut builder = FixtureIndexBuilder::new("test_under", "0.1.0");

    let code = concat!(
        "pub mod my_mod {\n",
        "    pub struct Type_A(pub i32);\n",
        "}\n",
        "fn «process_val»(arg: my_mod::Type_A) -> i32 {\n",
        "    arg.0 * 2\n",
        "}\n",
        "pub fn caller() -> i32 {\n",
        "    let val = my_mod::Type_A(21);\n",
        "    process_val(val)\n",
        "}\n",
    );

    let (cleaned, def_ids) = builder.add_defs_with_ids(code, &lib_str, "test_under", DefKind::Fn);
    let _ = fs::write(&lib_file, &cleaned);

    builder.add_ref("test_under::process_val", &lib_str, 4, 4, 4, 15, Some(def_ids[0]), Some(&lib_str), 4, 4, DefKind::Fn);
    builder.add_ref("test_under::process_val", &lib_str, 9, 5, 9, 16, Some(def_ids[0]), Some(&lib_str), 4, 4, DefKind::Fn);

    let ridx_file = temp_dir.join("index.ridx");
    builder.write_to_file(&ridx_file);
    let mut ws = WorkspaceIndex::open(&ridx_file).unwrap();
    ws.set_workspace_root(Some(temp_dir.clone()));
    let mut ov = OverlayManager::new();
    let uri = format!("file://{}", lib_str);
    ov.open_document(&uri, &lib_str, 1, cleaned.clone());

    let actions = query_actions_with_multi_file_resolve(&uri, 3, 4, &ws, &ov);
    let inline_action = actions.iter().find(|(t, _)| t == "Inline into all callers");
    assert!(inline_action.is_some(), "Expected 'Inline into all callers' when parameter type has underscores in identifier name");

    let (_, changes) = inline_action.unwrap();
    let edits = changes.get(&uri).expect("File edits missing");
    let updated = apply_text_edits(&cleaned, edits);

    assert!(!updated.contains("fn process_val"), "Declaration should be deleted");
    assert!(!updated.contains("process_val(val)"), "Call site should be inlined");
    assert_rustc_compiles(&updated, "Inline into all callers: underscored type identifier compiles");
}
