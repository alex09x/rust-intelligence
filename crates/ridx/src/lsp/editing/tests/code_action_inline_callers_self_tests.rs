/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Unit tests for inlining inherent methods containing `Self` type references.

use crate::lsp::editing::actions::{handle_code_action_resolve, handle_code_action_with_options};
use crate::lsp::editing::tests::code_action_assists_tests::assert_rustc_compiles;
use crate::overlay::OverlayManager;
use ridx_format::{DefKind, WorkspaceIndex};
use ridx_test_support::{apply_text_edits, FixtureIndexBuilder};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};

static INLINE_SELF_TEST_ID: AtomicU64 = AtomicU64::new(0);

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
fn test_inline_inherent_method_with_self_constructor() {
    let id = INLINE_SELF_TEST_ID.fetch_add(1, Ordering::Relaxed);
    let temp_dir = std::env::temp_dir().join(format!("ridx_test_inline_self_ctor_{}_{}", std::process::id(), id));
    let _ = fs::create_dir_all(temp_dir.join("src"));
    let src_file = temp_dir.join("src/lib.rs");
    let src_file_str = src_file.to_str().unwrap().to_string();

    let mut builder = FixtureIndexBuilder::new("test_self_ctor", "0.1.0");
    let code = concat!(
        "pub struct Buffer {\n",
        "    pub capacity: usize,\n",
        "}\n",
        "impl Buffer {\n",
        "    fn «with_default_cap»() -> Self {\n",
        "        Self { capacity: 64 }\n",
        "    }\n",
        "    pub fn create() -> Buffer {\n",
        "        Buffer::with_default_cap()\n",
        "    }\n",
        "}\n",
    );
    let (code_cleaned, def_ids) = builder.add_defs_with_ids(code, &src_file_str, "test_self_ctor", DefKind::Fn);
    let _ = fs::write(&src_file, &code_cleaned);

    builder.add_ref("test_self_ctor::with_default_cap", &src_file_str, 5, 8, 5, 24, Some(def_ids[0]), Some(&src_file_str), 5, 8, DefKind::Fn);
    builder.add_ref("test_self_ctor::with_default_cap", &src_file_str, 9, 17, 9, 33, Some(def_ids[0]), Some(&src_file_str), 5, 8, DefKind::Fn);

    let ridx_file = temp_dir.join("index.ridx");
    builder.write_to_file(&ridx_file);
    let mut ws = WorkspaceIndex::open(&ridx_file).unwrap();
    ws.set_workspace_root(Some(temp_dir.clone()));
    let mut ov = OverlayManager::new();
    let uri = format!("file://{}", src_file_str);
    ov.open_document(&uri, &src_file_str, 1, code_cleaned.clone());

    let actions = query_actions_with_multi_file_resolve(&uri, 4, 8, &ws, &ov);
    let inline_action = actions.iter().find(|(t, _)| t == "Inline into all callers");
    assert!(inline_action.is_some(), "Expected 'Inline into all callers' for method constructing Self");

    let (_, changes) = inline_action.unwrap();
    let edits = changes.get(&uri).expect("lib.rs edits missing");
    let updated = apply_text_edits(&code_cleaned, edits);

    assert!(!updated.contains("fn with_default_cap"), "Declaration must be deleted");
    assert!(!updated.contains("Buffer::with_default_cap()"), "Call site must be inlined");
    assert!(updated.contains("Buffer { capacity: 64 }"), "Self must be rewritten to Buffer");
    assert_rustc_compiles(&updated, "Inline into all callers: Self rewritten to Buffer");
}

#[test]
fn test_inline_inherent_method_with_tail_return() {
    let id = INLINE_SELF_TEST_ID.fetch_add(1, Ordering::Relaxed);
    let temp_dir = std::env::temp_dir().join(format!("ridx_test_inline_tail_ret_{}_{}", std::process::id(), id));
    let _ = fs::create_dir_all(temp_dir.join("src"));
    let src_file = temp_dir.join("src/lib.rs");
    let src_file_str = src_file.to_str().unwrap().to_string();

    let mut builder = FixtureIndexBuilder::new("test_tail_ret", "0.1.0");
    let code = concat!(
        "pub struct Calc {\n",
        "    pub base: i32,\n",
        "}\n",
        "impl Calc {\n",
        "    fn «compute»(&self, factor: i32) -> i32 {\n",
        "        let bonus = 10;\n",
        "        return (self.base + bonus) * factor;\n",
        "    }\n",
        "    pub fn run(c: &Calc) -> i32 {\n",
        "        c.compute(3)\n",
        "    }\n",
        "}\n",
    );
    let (code_cleaned, def_ids) = builder.add_defs_with_ids(code, &src_file_str, "test_tail_ret", DefKind::Fn);
    let _ = fs::write(&src_file, &code_cleaned);

    builder.add_ref("test_tail_ret::compute", &src_file_str, 5, 8, 5, 15, Some(def_ids[0]), Some(&src_file_str), 5, 8, DefKind::Fn);
    builder.add_ref("test_tail_ret::compute", &src_file_str, 10, 11, 10, 18, Some(def_ids[0]), Some(&src_file_str), 5, 8, DefKind::Fn);

    let ridx_file = temp_dir.join("index.ridx");
    builder.write_to_file(&ridx_file);
    let mut ws = WorkspaceIndex::open(&ridx_file).unwrap();
    ws.set_workspace_root(Some(temp_dir.clone()));
    let mut ov = OverlayManager::new();
    let uri = format!("file://{}", src_file_str);
    ov.open_document(&uri, &src_file_str, 1, code_cleaned.clone());

    let actions = query_actions_with_multi_file_resolve(&uri, 4, 8, &ws, &ov);
    let inline_action = actions.iter().find(|(t, _)| t == "Inline into all callers");
    assert!(inline_action.is_some(), "Expected 'Inline into all callers' for method with tail return");

    let (_, changes) = inline_action.unwrap();
    let edits = changes.get(&uri).expect("lib.rs edits missing");
    let updated = apply_text_edits(&code_cleaned, edits);

    assert!(!updated.contains("fn compute"), "Declaration must be deleted");
    assert!(!updated.contains("c.compute(3)"), "Call site must be inlined");
    assert!(!updated.contains("return "), "return keyword must be stripped");
    assert_rustc_compiles(&updated, "Inline into all callers: tail return unwrapped");
}

#[test]
fn test_inline_method_cross_file() {
    let id = INLINE_SELF_TEST_ID.fetch_add(1, Ordering::Relaxed);
    let temp_dir = std::env::temp_dir().join(format!("ridx_test_inline_cross_{}_{}", std::process::id(), id));
    let _ = fs::create_dir_all(temp_dir.join("src"));
    let worker_file = temp_dir.join("src/worker.rs");
    let main_file = temp_dir.join("src/main.rs");
    let worker_str = worker_file.to_str().unwrap().to_string();
    let main_str = main_file.to_str().unwrap().to_string();

    let mut builder = FixtureIndexBuilder::new("test_cross", "0.1.0");
    let worker_code = concat!("pub struct Worker;\n", "impl Worker {\n", "    pub(crate) fn «work»(&self) -> i32 {\n", "        42\n", "    }\n", "}\n",);
    let main_code = concat!("mod worker;\n", "use worker::Worker;\n", "fn main() {\n", "    let w = Worker;\n", "    let _ = w.work();\n", "}\n",);

    let (worker_cleaned, def_ids) = builder.add_defs_with_ids(worker_code, &worker_str, "test_cross", DefKind::Fn);
    let _ = fs::write(&worker_file, &worker_cleaned);
    let _ = fs::write(&main_file, main_code);

    builder.add_ref("test_cross::work", &worker_str, 3, 19, 3, 23, Some(def_ids[0]), Some(&worker_str), 3, 19, DefKind::Fn);
    builder.add_ref("test_cross::work", &main_str, 5, 15, 5, 19, Some(def_ids[0]), Some(&worker_str), 3, 19, DefKind::Fn);

    let ridx_file = temp_dir.join("index.ridx");
    builder.write_to_file(&ridx_file);
    let mut ws = WorkspaceIndex::open(&ridx_file).unwrap();
    ws.set_workspace_root(Some(temp_dir.clone()));
    let mut ov = OverlayManager::new();
    let worker_uri = format!("file://{}", worker_str);
    let main_uri = format!("file://{}", main_str);
    ov.open_document(&worker_uri, &worker_str, 1, worker_cleaned.clone());

    let actions = query_actions_with_multi_file_resolve(&worker_uri, 2, 18, &ws, &ov);
    let inline_action = actions.iter().find(|(t, _)| t == "Inline into all callers");
    assert!(inline_action.is_some(), "Expected 'Inline into all callers' to support multi-file inlining");

    let (_, changes) = inline_action.unwrap();
    let main_edits = changes.get(&main_uri).expect("main.rs edits missing");
    let updated_main = apply_text_edits(main_code, main_edits);
    assert!(!updated_main.contains("w.work()"), "Call site in main.rs must be inlined");
}

#[test]
fn test_inline_method_relative_path_references() {
    let id = INLINE_SELF_TEST_ID.fetch_add(1, Ordering::Relaxed);
    let temp_dir = std::env::temp_dir().join(format!("ridx_test_inline_rel_{}_{}", std::process::id(), id));
    let _ = fs::create_dir_all(temp_dir.join("src"));
    let worker_file = temp_dir.join("src/worker.rs");
    let main_file = temp_dir.join("src/main.rs");
    let worker_str = worker_file.to_str().unwrap().to_string();
    let _main_str = main_file.to_str().unwrap().to_string();

    let mut builder = FixtureIndexBuilder::new("test_rel", "0.1.0");
    let worker_code = concat!(
        "pub struct Worker {\n",
        "    pub val: i32,\n",
        "}\n",
        "impl Worker {\n",
        "    fn «get»(&self) -> i32 {\n",
        "        self.val\n",
        "    }\n",
        "    pub fn run(&self) -> i32 {\n",
        "        self.get()\n",
        "    }\n",
        "}\n",
    );
    let main_code = concat!("mod worker;\n", "use worker::Worker;\n", "fn main() {\n", "    let w = Worker { val: 10 };\n", "    let _ = w.run();\n", "}\n",);

    let (worker_cleaned, def_ids) = builder.add_defs_with_ids(worker_code, &worker_str, "test_rel", DefKind::Fn);
    let _ = fs::write(&worker_file, &worker_cleaned);
    let _ = fs::write(&main_file, main_code);

    // References indexed with relative paths matching real workspace metadata
    builder.add_ref("test_rel::get", "src/worker.rs", 5, 8, 5, 11, Some(def_ids[0]), Some("src/worker.rs"), 5, 8, DefKind::Fn);
    builder.add_ref("test_rel::get", "src/worker.rs", 9, 14, 9, 17, Some(def_ids[0]), Some("src/worker.rs"), 5, 8, DefKind::Fn);

    let ridx_file = temp_dir.join("index.ridx");
    builder.write_to_file(&ridx_file);
    let mut ws = WorkspaceIndex::open(&ridx_file).unwrap();
    ws.set_workspace_root(Some(temp_dir.clone()));
    let mut ov = OverlayManager::new();
    let worker_uri = format!("file://{}", worker_str);
    ov.open_document(&worker_uri, &worker_str, 1, worker_cleaned.clone());

    let actions = query_actions_with_multi_file_resolve(&worker_uri, 4, 8, &ws, &ov);
    let inline_action = actions.iter().find(|(t, _)| t == "Inline into all callers");
    assert!(inline_action.is_some(), "Expected 'Inline into all callers' when references use relative paths");

    let (_, changes) = inline_action.unwrap();
    let edits = changes.get(&worker_uri).expect("Worker file edits missing");
    let updated = apply_text_edits(&worker_cleaned, edits);

    assert!(!updated.contains("fn get"), "Declaration must be deleted");
    assert!(!updated.contains("self.get()"), "Internal call site must be inlined");
    assert!(updated.contains("this.val") || updated.contains("self.val"), "Field access preserved in single-file caller");
    assert_rustc_compiles(&updated, "Inline into all callers: relative path reference resolution");
}

#[test]
fn test_inline_pub_fn_preserves_declaration() {
    let id = INLINE_SELF_TEST_ID.fetch_add(1, Ordering::Relaxed);
    let temp_dir = std::env::temp_dir().join(format!("ridx_test_inline_pub_{}_{}", std::process::id(), id));
    let _ = fs::create_dir_all(temp_dir.join("src"));
    let lib_file = temp_dir.join("src/lib.rs");
    let lib_str = lib_file.to_str().unwrap().to_string();

    let mut builder = FixtureIndexBuilder::new("test_pub", "0.1.0");
    let code = concat!(
        "pub struct Counter {\n",
        "    pub count: i32,\n",
        "}\n",
        "impl Counter {\n",
        "    pub fn «is_positive»(&self) -> bool {\n",
        "        self.count > 0\n",
        "    }\n",
        "    pub fn check(&self) -> bool {\n",
        "        self.is_positive()\n",
        "    }\n",
        "}\n",
    );

    let (cleaned, def_ids) = builder.add_defs_with_ids(code, &lib_str, "test_pub", DefKind::Fn);
    let _ = fs::write(&lib_file, &cleaned);

    builder.add_ref("test_pub::is_positive", &lib_str, 5, 12, 5, 23, Some(def_ids[0]), Some(&lib_str), 5, 12, DefKind::Fn);
    builder.add_ref("test_pub::is_positive", &lib_str, 9, 14, 9, 25, Some(def_ids[0]), Some(&lib_str), 5, 12, DefKind::Fn);

    let ridx_file = temp_dir.join("index.ridx");
    builder.write_to_file(&ridx_file);
    let mut ws = WorkspaceIndex::open(&ridx_file).unwrap();
    ws.set_workspace_root(Some(temp_dir.clone()));
    let mut ov = OverlayManager::new();
    let uri = format!("file://{}", lib_str);
    ov.open_document(&uri, &lib_str, 1, cleaned.clone());

    let actions = query_actions_with_multi_file_resolve(&uri, 4, 12, &ws, &ov);
    let inline_action = actions.iter().find(|(t, _)| t == "Inline into all callers");
    assert!(inline_action.is_some(), "Expected 'Inline into all callers' for pub fn with internal callers");

    let (_, changes) = inline_action.unwrap();
    let edits = changes.get(&uri).expect("File edits missing");
    let updated = apply_text_edits(&cleaned, edits);

    assert!(updated.contains("pub fn is_positive"), "pub fn declaration must be preserved for external consumers");
    assert!(!updated.contains("self.is_positive()"), "Internal call site must be inlined");
    assert_rustc_compiles(&updated, "Inline into all callers: pub fn preserves declaration");
}

#[test]
fn test_inline_result_fn_compiles() {
    let id = INLINE_SELF_TEST_ID.fetch_add(1, Ordering::Relaxed);
    let temp_dir = std::env::temp_dir().join(format!("ridx_inline_result_{}_{}", std::process::id(), id));
    let _ = fs::create_dir_all(&temp_dir);
    let lib_file = temp_dir.join("lib.rs");
    let lib_str = lib_file.to_str().unwrap().to_string();

    let mut builder = FixtureIndexBuilder::new("test_result", "0.1.0");

    let code = concat!(
        "fn «compute»(x: i32) -> Result<i32, String> {\n",
        "    if x > 0 { Ok(x * 2) } else { Err(String::from(\"negative\")) }\n",
        "}\n",
        "pub fn caller(val: i32) -> Result<i32, String> {\n",
        "    let res = compute(val)?;\n",
        "    Ok(res + 1)\n",
        "}\n",
    );

    let (cleaned, def_ids) = builder.add_defs_with_ids(code, &lib_str, "test_result", DefKind::Fn);
    let _ = fs::write(&lib_file, &cleaned);

    builder.add_ref("test_result::compute", &lib_str, 0, 3, 0, 10, Some(def_ids[0]), Some(&lib_str), 0, 3, DefKind::Fn);
    builder.add_ref("test_result::compute", &lib_str, 4, 14, 4, 21, Some(def_ids[0]), Some(&lib_str), 0, 3, DefKind::Fn);

    let ridx_file = temp_dir.join("index.ridx");
    builder.write_to_file(&ridx_file);
    let mut ws = WorkspaceIndex::open(&ridx_file).unwrap();
    ws.set_workspace_root(Some(temp_dir.clone()));
    let mut ov = OverlayManager::new();
    let uri = format!("file://{}", lib_str);
    ov.open_document(&uri, &lib_str, 1, cleaned.clone());

    let actions = query_actions_with_multi_file_resolve(&uri, 0, 4, &ws, &ov);
    let inline_action = actions.iter().find(|(t, _)| t == "Inline into all callers");
    assert!(inline_action.is_some(), "Expected 'Inline into all callers' for Result-returning fn");

    let (_, changes) = inline_action.unwrap();
    let edits = changes.get(&uri).expect("File edits missing");
    let updated = apply_text_edits(&cleaned, edits);

    assert!(!updated.contains("fn compute("), "Non-pub fn declaration should be deleted");
    assert!(!updated.contains("compute(val)"), "Call site should be inlined");
    assert_rustc_compiles(&updated, "Inline into all callers: Result-returning fn compiles");
}

#[test]
fn test_inline_generic_fn_unused_in_body_compiles() {
    let id = INLINE_SELF_TEST_ID.fetch_add(1, Ordering::Relaxed);
    let temp_dir = std::env::temp_dir().join(format!("ridx_inline_gen_{}_{}", std::process::id(), id));
    let _ = fs::create_dir_all(&temp_dir);
    let lib_file = temp_dir.join("lib.rs");
    let lib_str = lib_file.to_str().unwrap().to_string();

    let mut builder = FixtureIndexBuilder::new("test_gen", "0.1.0");

    let code = concat!("fn «wrap_val»<E>(x: i32) -> Result<i32, E> {\n", "    Ok(x * 2)\n", "}\n", "pub fn caller() -> Result<i32, ()> {\n", "    wrap_val(21)\n", "}\n",);

    let (cleaned, def_ids) = builder.add_defs_with_ids(code, &lib_str, "test_gen", DefKind::Fn);
    let _ = fs::write(&lib_file, &cleaned);

    builder.add_ref("test_gen::wrap_val", &lib_str, 1, 4, 1, 12, Some(def_ids[0]), Some(&lib_str), 1, 4, DefKind::Fn);
    builder.add_ref("test_gen::wrap_val", &lib_str, 5, 5, 5, 13, Some(def_ids[0]), Some(&lib_str), 1, 4, DefKind::Fn);

    let ridx_file = temp_dir.join("index.ridx");
    builder.write_to_file(&ridx_file);
    let mut ws = WorkspaceIndex::open(&ridx_file).unwrap();
    ws.set_workspace_root(Some(temp_dir.clone()));
    let mut ov = OverlayManager::new();
    let uri = format!("file://{}", lib_str);
    ov.open_document(&uri, &lib_str, 1, cleaned.clone());

    let actions = query_actions_with_multi_file_resolve(&uri, 0, 4, &ws, &ov);
    let inline_action = actions.iter().find(|(t, _)| t == "Inline into all callers");
    assert!(inline_action.is_some(), "Expected 'Inline into all callers' for generic fn unused in body");

    let (_, changes) = inline_action.unwrap();
    let edits = changes.get(&uri).expect("File edits missing");
    let updated = apply_text_edits(&cleaned, edits);

    assert!(!updated.contains("fn wrap_val"), "Declaration should be deleted");
    assert!(!updated.contains("wrap_val(21)"), "Call site should be inlined");
    assert_rustc_compiles(&updated, "Inline into all callers: generic fn unused in body compiles");
}
