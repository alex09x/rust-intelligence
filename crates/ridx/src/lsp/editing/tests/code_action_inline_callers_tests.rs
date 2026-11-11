/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Unit tests for compile-safe `Inline into all callers` assist across inherent methods and callers.

use crate::lsp::editing::actions::{handle_code_action_resolve, handle_code_action_with_options};
use crate::lsp::editing::tests::code_action_assists_tests::assert_rustc_compiles;
use crate::lsp::protocol::PositionConverter;
use crate::overlay::OverlayManager;
use ridx_format::{DefKind, WorkspaceIndex};
use ridx_test_support::{apply_text_edits, FixtureIndexBuilder};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};

static INLINE_TEST_ID: AtomicU64 = AtomicU64::new(0);

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
        let res_req = json!({ "params": item });
        let resolved = handle_code_action_resolve(&res_req, Some(ws), ov).unwrap();
        let title = resolved["title"].as_str().unwrap().to_string();
        let mut changes_map = BTreeMap::new();
        if let Some(changes) = resolved.get("edit").and_then(|e| e.get("changes")).and_then(|c| c.as_object()) {
            for (file_uri, edits_val) in changes {
                if let Some(ed_arr) = edits_val.as_array() {
                    changes_map.insert(file_uri.clone(), ed_arr.clone());
                }
            }
        }
        resolved_out.push((title, changes_map));
    }
    resolved_out
}

#[test]
fn test_inline_private_method_into_child_module_caller() {
    let id = INLINE_TEST_ID.fetch_add(1, Ordering::Relaxed);
    let temp_dir = std::env::temp_dir().join(format!("ridx_test_inline_multi_{}_{}", std::process::id(), id));
    let _ = fs::create_dir_all(temp_dir.join("src"));
    let src_file = temp_dir.join("src/lib.rs");
    let caller_file = temp_dir.join("src/caller.rs");

    let src_file_str = src_file.to_str().unwrap().to_string();
    let caller_file_str = caller_file.to_str().unwrap().to_string();

    let mut builder = FixtureIndexBuilder::new("test_inline", "0.1.0");

    let lib_code = concat!(
        "pub mod caller;\n",
        "pub struct Counter {\n",
        "    pub val: i32,\n",
        "}\n",
        "impl Counter {\n",
        "    fn «add_offset»(&self, offset: i32) -> i32 {\n",
        "        self.val + offset\n",
        "    }\n",
        "    pub fn run_local(&self) -> i32 {\n",
        "        self.add_offset(10)\n",
        "    }\n",
        "}\n",
    );
    let (code_cleaned, def_ids) = builder.add_defs_with_ids(lib_code, &src_file_str, "test_inline", DefKind::Fn);
    let _ = fs::write(&src_file, &code_cleaned);

    // Self ref at declaration: line 6 (1-based), col 8
    builder.add_ref("test_inline::add_offset", &src_file_str, 6, 8, 6, 18, Some(def_ids[0]), Some(&src_file_str), 6, 8, DefKind::Fn);

    // Local caller ref in lib.rs: line 10 (1-based), character 14
    builder.add_ref("test_inline::add_offset", &src_file_str, 10, 14, 10, 24, Some(def_ids[0]), Some(&src_file_str), 6, 8, DefKind::Fn);

    // Cross-file caller in caller.rs
    let caller_code = concat!("use crate::Counter;\n", "pub fn run_remote(c: &Counter) -> i32 {\n", "    c.add_offset(20)\n", "}\n",);
    let _ = fs::write(&caller_file, caller_code);
    // Remote caller ref in caller.rs: line 3 (1-based), character 7
    builder.add_ref("test_inline::add_offset", &caller_file_str, 3, 7, 3, 17, Some(def_ids[0]), Some(&src_file_str), 6, 8, DefKind::Fn);

    let ridx_file = temp_dir.join("index.ridx");
    builder.write_to_file(&ridx_file);

    let mut ws = WorkspaceIndex::open(&ridx_file).unwrap();
    ws.set_workspace_root(Some(temp_dir.clone()));
    let mut ov = OverlayManager::new();
    let _conv = PositionConverter::new();

    let lib_uri = format!("file://{}", src_file_str);
    let caller_uri = format!("file://{}", caller_file_str);
    ov.open_document(&lib_uri, &src_file_str, 1, code_cleaned.clone());
    ov.open_document(&caller_uri, &caller_file_str, 1, caller_code.to_string());

    let actions = query_actions_with_multi_file_resolve(&lib_uri, 5, 8, &ws, &ov);
    // A private method is visible in the modules inside its own (`crate::caller`), so the action is offered. The
    // fixture records no field table, so the remote caller's `.val` cannot be proven visible: that call is left as it
    // is and the method is kept (one edit in lib.rs, the local call).
    let (_, edits) = actions.iter().find(|(t, _)| t == "Inline into all callers").expect("offered for the local caller");
    assert!(edits.keys().all(|uri| !uri.ends_with("caller.rs")), "{edits:?}");
    let lib = edits.iter().find(|(uri, _)| uri.ends_with("lib.rs")).map(|(_, e)| e.len());
    assert_eq!(lib, Some(1), "{edits:?}");
}

#[test]
fn test_inline_inherent_method_with_mut_self() {
    let id = INLINE_TEST_ID.fetch_add(1, Ordering::Relaxed);
    let temp_dir = std::env::temp_dir().join(format!("ridx_test_inline_mut_{}_{}", std::process::id(), id));
    let _ = fs::create_dir_all(temp_dir.join("src"));
    let src_file = temp_dir.join("src/lib.rs");
    let src_file_str = src_file.to_str().unwrap().to_string();

    let mut builder = FixtureIndexBuilder::new("test_mut_inline", "0.1.0");

    let code = concat!(
        "pub struct Counter {\n",
        "    pub val: i32,\n",
        "}\n",
        "impl Counter {\n",
        "    fn «bump»(&mut self, step: i32) {\n",
        "        self.val += step;\n",
        "    }\n",
        "    pub fn test(&mut self) {\n",
        "        self.bump(5);\n",
        "    }\n",
        "}\n",
    );
    let (code_cleaned, def_ids) = builder.add_defs_with_ids(code, &src_file_str, "test_mut_inline", DefKind::Fn);
    let _ = fs::write(&src_file, &code_cleaned);

    builder.add_ref("test_mut_inline::bump", &src_file_str, 5, 8, 5, 12, Some(def_ids[0]), Some(&src_file_str), 5, 8, DefKind::Fn);
    builder.add_ref("test_mut_inline::bump", &src_file_str, 9, 14, 9, 18, Some(def_ids[0]), Some(&src_file_str), 5, 8, DefKind::Fn);

    let ridx_file = temp_dir.join("index.ridx");
    builder.write_to_file(&ridx_file);

    let mut ws = WorkspaceIndex::open(&ridx_file).unwrap();
    ws.set_workspace_root(Some(temp_dir.clone()));
    let mut ov = OverlayManager::new();

    let uri = format!("file://{}", src_file_str);
    ov.open_document(&uri, &src_file_str, 1, code_cleaned.clone());

    let actions = query_actions_with_multi_file_resolve(&uri, 4, 8, &ws, &ov);
    let inline_action = actions.iter().find(|(t, _)| t == "Inline into all callers");
    assert!(inline_action.is_some(), "Expected 'Inline into all callers' assist for &mut self");

    let (_, changes) = inline_action.unwrap();
    let edits = changes.get(&uri).expect("lib.rs edits missing");
    let updated = apply_text_edits(&code_cleaned, edits);

    assert!(!updated.contains("fn bump"), "Declaration must be deleted");
    assert!(!updated.contains("self.bump(5)"), "Call site must be inlined");
    assert_rustc_compiles(&updated, "Inline into all callers: &mut self");
}

#[test]
fn test_inline_rejected_on_return_and_question_mark() {
    let id = INLINE_TEST_ID.fetch_add(1, Ordering::Relaxed);
    let temp_dir = std::env::temp_dir().join(format!("ridx_test_inline_rej_{}_{}", std::process::id(), id));
    let _ = fs::create_dir_all(temp_dir.join("src"));
    let src_file = temp_dir.join("src/lib.rs");
    let src_file_str = src_file.to_str().unwrap().to_string();

    let builder = FixtureIndexBuilder::new("test_inline_rej", "0.1.0");

    let code = concat!(
        "pub struct S(pub i32);\n",
        "impl S {\n",
        "    fn with_return(&self) -> i32 {\n",
        "        if self.0 > 0 { return 1; }\n",
        "        0\n",
        "    }\n",
        "    fn with_qmark(&self, opt: Option<i32>) -> Option<i32> {\n",
        "        let v = opt?;\n",
        "        Some(v + self.0)\n",
        "    }\n",
        "}\n",
    );
    let _ = fs::write(&src_file, code);
    let ridx_file = temp_dir.join("index.ridx");
    builder.write_to_file(&ridx_file);

    let mut ws = WorkspaceIndex::open(&ridx_file).unwrap();
    ws.set_workspace_root(Some(temp_dir.clone()));
    let mut ov = OverlayManager::new();

    let uri = format!("file://{}", src_file_str);
    ov.open_document(&uri, &src_file_str, 1, code.to_string());

    // Line 2: with_return
    let actions_ret = query_actions_with_multi_file_resolve(&uri, 2, 8, &ws, &ov);
    assert!(actions_ret.iter().all(|(t, _)| t != "Inline into all callers"), "Must reject function containing early return");

    // Line 6: with_qmark
    let actions_qm = query_actions_with_multi_file_resolve(&uri, 6, 8, &ws, &ov);
    assert!(actions_qm.iter().all(|(t, _)| t != "Inline into all callers"), "Must reject function containing '?' operator");
}

#[test]
fn test_inline_rejected_on_generic_function() {
    let id = INLINE_TEST_ID.fetch_add(1, Ordering::Relaxed);
    let temp_dir = std::env::temp_dir().join(format!("ridx_test_inline_gen_{}_{}", std::process::id(), id));
    let _ = fs::create_dir_all(temp_dir.join("src"));
    let src_file = temp_dir.join("src/lib.rs");
    let src_file_str = src_file.to_str().unwrap().to_string();

    let builder = FixtureIndexBuilder::new("test_inline_gen", "0.1.0");

    let code = concat!("pub struct S(pub i32);\n", "impl S {\n", "    fn with_gen<T>(&self, _t: T) -> i32 {\n", "        self.0\n", "    }\n", "}\n",);
    let _ = fs::write(&src_file, code);
    let ridx_file = temp_dir.join("index.ridx");
    builder.write_to_file(&ridx_file);

    let mut ws = WorkspaceIndex::open(&ridx_file).unwrap();
    ws.set_workspace_root(Some(temp_dir.clone()));
    let mut ov = OverlayManager::new();

    let uri = format!("file://{}", src_file_str);
    ov.open_document(&uri, &src_file_str, 1, code.to_string());

    // Line 2: with_gen<T>
    let actions = query_actions_with_multi_file_resolve(&uri, 2, 8, &ws, &ov);
    assert!(actions.iter().all(|(t, _)| t != "Inline into all callers"), "Must reject function with generic type parameters");
}

#[test]
fn test_inline_inherent_method_cursor_inside_body() {
    let id = INLINE_TEST_ID.fetch_add(1, Ordering::Relaxed);
    let temp_dir = std::env::temp_dir().join(format!("ridx_test_inline_body_{}_{}", std::process::id(), id));
    let _ = fs::create_dir_all(temp_dir.join("src"));
    let src_file = temp_dir.join("src/lib.rs");
    let src_file_str = src_file.to_str().unwrap().to_string();

    let mut builder = FixtureIndexBuilder::new("test_inline_body", "0.1.0");

    let code = concat!(
        "pub struct Counter {\n",
        "    pub val: i32,\n",
        "}\n",
        "impl Counter {\n",
        "    fn «add_ten»(&self) -> i32 {\n",
        "        let bonus = 10;\n",
        "        self.val + bonus\n",
        "    }\n",
        "    pub fn run(&self) -> i32 {\n",
        "        self.add_ten()\n",
        "    }\n",
        "}\n",
    );
    let (code_cleaned, def_ids) = builder.add_defs_with_ids(code, &src_file_str, "test_inline_body", DefKind::Fn);
    let _ = fs::write(&src_file, &code_cleaned);

    builder.add_ref("test_inline_body::add_ten", &src_file_str, 5, 8, 5, 15, Some(def_ids[0]), Some(&src_file_str), 5, 8, DefKind::Fn);
    builder.add_ref("test_inline_body::add_ten", &src_file_str, 10, 14, 10, 21, Some(def_ids[0]), Some(&src_file_str), 5, 8, DefKind::Fn);

    let ridx_file = temp_dir.join("index.ridx");
    builder.write_to_file(&ridx_file);

    let mut ws = WorkspaceIndex::open(&ridx_file).unwrap();
    ws.set_workspace_root(Some(temp_dir.clone()));
    let mut ov = OverlayManager::new();

    let uri = format!("file://{}", src_file_str);
    ov.open_document(&uri, &src_file_str, 1, code_cleaned.clone());

    // Line 5 (0-based): inside the body on `let bonus = 10;`!
    let actions = query_actions_with_multi_file_resolve(&uri, 5, 12, &ws, &ov);
    let inline_action = actions.iter().find(|(t, _)| t == "Inline into all callers");
    assert!(inline_action.is_some(), "Expected 'Inline into all callers' when cursor is inside body");

    let (_, changes) = inline_action.unwrap();
    let edits = changes.get(&uri).expect("lib.rs edits missing");
    let updated = apply_text_edits(&code_cleaned, edits);

    assert!(!updated.contains("fn add_ten"), "Declaration must be deleted");
    assert!(!updated.contains("self.add_ten()"), "Call site must be inlined");
    assert_rustc_compiles(&updated, "Inline into all callers: cursor in body");
}

#[test]
fn test_inline_delegating_method_and_lifetime_generics() {
    let id = INLINE_TEST_ID.fetch_add(1, Ordering::Relaxed);
    let temp_dir = std::env::temp_dir().join(format!("ridx_test_inline_del_{}_{}", std::process::id(), id));
    let _ = fs::create_dir_all(temp_dir.join("src"));
    let src_file = temp_dir.join("src/lib.rs");
    let src_file_str = src_file.to_str().unwrap().to_string();

    let mut builder = FixtureIndexBuilder::new("test_inline_del", "0.1.0");
    let code = concat!(
        "pub struct Inner;\n",
        "impl Inner {\n",
        "    pub fn execute(&self, x: i32) -> i32 { x + 1 }\n",
        "}\n",
        "pub struct Wrapper {\n",
        "    pub inner: Inner,\n",
        "}\n",
        "impl Wrapper {\n",
        "    fn «execute»<'a>(&'a self, x: i32) -> i32 {\n",
        "        self.inner.execute(x)\n",
        "    }\n",
        "}\n",
        "pub fn run(w: &Wrapper) -> i32 {\n",
        "    w.execute(5)\n",
        "}\n",
    );
    let (code_cleaned, def_ids) = builder.add_defs_with_ids(code, &src_file_str, "test_inline_del", DefKind::Fn);
    let _ = fs::write(&src_file, &code_cleaned);
    builder.add_ref("test_inline_del::execute", &src_file_str, 9, 8, 9, 15, Some(def_ids[0]), Some(&src_file_str), 9, 8, DefKind::Fn);
    builder.add_ref("test_inline_del::execute", &src_file_str, 14, 7, 14, 14, Some(def_ids[0]), Some(&src_file_str), 9, 8, DefKind::Fn);

    let ridx_file = temp_dir.join("index.ridx");
    builder.write_to_file(&ridx_file);
    let mut ws = WorkspaceIndex::open(&ridx_file).unwrap();
    ws.set_workspace_root(Some(temp_dir.clone()));
    let mut ov = OverlayManager::new();
    let uri = format!("file://{}", src_file_str);
    ov.open_document(&uri, &src_file_str, 1, code_cleaned.clone());

    let actions = query_actions_with_multi_file_resolve(&uri, 8, 7, &ws, &ov);
    let inline_action = actions.iter().find(|(t, _)| t == "Inline into all callers");
    assert!(inline_action.is_some(), "Expected 'Inline into all callers' on delegating method with lifetime generic");

    let (_, changes) = inline_action.unwrap();
    let edits = changes.get(&uri).expect("lib.rs edits missing");
    let updated = apply_text_edits(&code_cleaned, edits);

    assert!(!updated.contains("fn execute<'a>"), "Declaration must be deleted");
    assert!(!updated.contains("w.execute(5)"), "Call site must be inlined");
    assert_rustc_compiles(&updated, "Inline into all callers: delegating method with lifetime generics");
}

#[test]
fn test_inline_method_with_path_qualified_self_and_this_reborrow() {
    let id = INLINE_TEST_ID.fetch_add(1, Ordering::Relaxed);
    let temp_dir = std::env::temp_dir().join(format!("ridx_test_inline_self_this_{}_{}", std::process::id(), id));
    let _ = fs::create_dir_all(temp_dir.join("src"));
    let src_file = temp_dir.join("src/lib.rs");
    let src_file_str = src_file.to_str().unwrap().to_string();

    let mut builder = FixtureIndexBuilder::new("test_self_this", "0.1.0");
    let code = concat!(
        "pub mod sub {\n",
        "    pub enum Kind { A, B }\n",
        "}\n",
        "pub struct Worker {\n",
        "    pub count: i32,\n",
        "}\n",
        "impl Worker {\n",
        "    fn «step»(&mut self) -> i32 {\n",
        "        use self::sub::Kind;\n",
        "        let _k = Kind::A;\n",
        "        self.count += 1;\n",
        "        self.count\n",
        "    }\n",
        "    pub fn run(this: &mut Worker) -> i32 {\n",
        "        this.step()\n",
        "    }\n",
        "}\n",
    );
    let (code_cleaned, def_ids) = builder.add_defs_with_ids(code, &src_file_str, "test_self_this", DefKind::Fn);
    let _ = fs::write(&src_file, &code_cleaned);
    builder.add_ref("test_self_this::step", &src_file_str, 8, 8, 8, 12, Some(def_ids[0]), Some(&src_file_str), 8, 8, DefKind::Fn);
    builder.add_ref("test_self_this::step", &src_file_str, 15, 14, 15, 18, Some(def_ids[0]), Some(&src_file_str), 8, 8, DefKind::Fn);

    let ridx_file = temp_dir.join("index.ridx");
    builder.write_to_file(&ridx_file);
    let mut ws = WorkspaceIndex::open(&ridx_file).unwrap();
    ws.set_workspace_root(Some(temp_dir.clone()));
    let mut ov = OverlayManager::new();
    let uri = format!("file://{}", src_file_str);
    ov.open_document(&uri, &src_file_str, 1, code_cleaned.clone());

    let actions = query_actions_with_multi_file_resolve(&uri, 7, 8, &ws, &ov);
    let inline_action = actions.iter().find(|(t, _)| t == "Inline into all callers");
    assert!(inline_action.is_some(), "Expected 'Inline into all callers' to be offered");

    let (_, changes) = inline_action.unwrap();
    let edits = changes.get(&uri).expect("lib.rs edits missing");
    let updated = apply_text_edits(&code_cleaned, edits);

    assert!(!updated.contains("fn step"), "Declaration must be deleted");
    assert!(!updated.contains("this.step()"), "Call site must be inlined");
    assert!(updated.contains("use self::sub::Kind;"), "self:: must not be rewritten to this::");
    assert_rustc_compiles(&updated, "Inline into all callers: self:: path qualifier and this receiver reborrow");
}
