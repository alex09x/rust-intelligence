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
