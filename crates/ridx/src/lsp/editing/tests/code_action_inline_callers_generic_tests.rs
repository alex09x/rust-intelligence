/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Unit tests for inlining inherent methods on generic types and compound Self return types.

use crate::lsp::editing::actions::{handle_code_action_resolve, handle_code_action_with_options};
use crate::lsp::editing::tests::code_action_assists_tests::assert_rustc_compiles;
use crate::overlay::OverlayManager;
use ridx_format::{DefKind, WorkspaceIndex};
use ridx_test_support::{apply_text_edits, FixtureIndexBuilder};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};

static INLINE_GEN_TEST_ID: AtomicU64 = AtomicU64::new(0);

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
fn test_inline_inherent_method_on_generic_struct_compiles() {
    let id = INLINE_GEN_TEST_ID.fetch_add(1, Ordering::Relaxed);
    let temp_dir = std::env::temp_dir().join(format!("ridx_test_inline_gen_struct_{}_{}", std::process::id(), id));
    let _ = fs::create_dir_all(temp_dir.join("src"));
    let src_file = temp_dir.join("src/lib.rs");
    let src_file_str = src_file.to_str().unwrap().to_string();

    let mut builder = FixtureIndexBuilder::new("test_gen_struct", "0.1.0");
    let code = concat!(
        "pub struct Filter<St, F> {\n",
        "    pub stream: St,\n",
        "    pub f: F,\n",
        "}\n",
        "impl<St, F> Filter<St, F> {\n",
        "    pub(crate) fn «new»(stream: St, f: F) -> Self {\n",
        "        Self { stream, f }\n",
        "    }\n",
        "}\n",
        "pub fn run() -> Filter<u32, bool> {\n",
        "    Filter::new(42u32, true)\n",
        "}\n",
    );
    let (code_cleaned, def_ids) = builder.add_defs_with_ids(code, &src_file_str, "test_gen_struct", DefKind::Fn);
    let _ = fs::write(&src_file, &code_cleaned);

    builder.add_ref("test_gen_struct::new", &src_file_str, 6, 19, 6, 22, Some(def_ids[0]), Some(&src_file_str), 6, 19, DefKind::Fn);
    builder.add_ref("test_gen_struct::new", &src_file_str, 11, 13, 11, 16, Some(def_ids[0]), Some(&src_file_str), 6, 19, DefKind::Fn);

    let ridx_file = temp_dir.join("index.ridx");
    builder.write_to_file(&ridx_file);
    let mut ws = WorkspaceIndex::open(&ridx_file).unwrap();
    ws.set_workspace_root(Some(temp_dir.clone()));
    let mut ov = OverlayManager::new();
    let uri = format!("file://{}", src_file_str);
    ov.open_document(&uri, &src_file_str, 1, code_cleaned.clone());

    let actions = query_actions_with_multi_file_resolve(&uri, 5, 19, &ws, &ov);
    let inline_action = actions.iter().find(|(t, _)| t == "Inline into all callers");
    assert!(inline_action.is_some(), "Expected 'Inline into all callers' for generic struct constructor");

    let (_, changes) = inline_action.unwrap();
    let edits = changes.get(&uri).expect("lib.rs edits missing");
    let updated = apply_text_edits(&code_cleaned, edits);

    assert!(updated.contains("Filter { stream: 42u32, f: true }") || updated.contains("Filter { stream, f }"), "Call site must be inlined with Filter constructor");
    assert_rustc_compiles(&updated, "Inline into all callers: generic struct constructor");
}

#[test]
fn test_inline_method_with_compound_self_return_type_compiles() {
    let id = INLINE_GEN_TEST_ID.fetch_add(1, Ordering::Relaxed);
    let temp_dir = std::env::temp_dir().join(format!("ridx_test_inline_compound_self_{}_{}", std::process::id(), id));
    let _ = fs::create_dir_all(temp_dir.join("src"));
    let src_file = temp_dir.join("src/lib.rs");
    let src_file_str = src_file.to_str().unwrap().to_string();

    let mut builder = FixtureIndexBuilder::new("test_compound_self", "0.1.0");
    let code = concat!(
        "pub struct Dir(pub f32);\n",
        "impl Dir {\n",
        "    pub fn «new_and_length»(val: f32) -> Result<(Self, f32), &'static str> {\n",
        "        if val <= 0.0 {\n",
        "            Err(\"invalid\")\n",
        "        } else {\n",
        "            Ok((Self(val), val))\n",
        "        }\n",
        "    }\n",
        "}\n",
        "pub fn run() -> Result<(Dir, f32), &'static str> {\n",
        "    Dir::new_and_length(5.0)\n",
        "}\n",
    );
    let (code_cleaned, def_ids) = builder.add_defs_with_ids(code, &src_file_str, "test_compound_self", DefKind::Fn);
    let _ = fs::write(&src_file, &code_cleaned);

    builder.add_ref("test_compound_self::new_and_length", &src_file_str, 3, 12, 3, 26, Some(def_ids[0]), Some(&src_file_str), 3, 12, DefKind::Fn);
    builder.add_ref("test_compound_self::new_and_length", &src_file_str, 12, 10, 12, 24, Some(def_ids[0]), Some(&src_file_str), 3, 12, DefKind::Fn);

    let ridx_file = temp_dir.join("index.ridx");
    builder.write_to_file(&ridx_file);
    let mut ws = WorkspaceIndex::open(&ridx_file).unwrap();
    ws.set_workspace_root(Some(temp_dir.clone()));
    let mut ov = OverlayManager::new();
    let uri = format!("file://{}", src_file_str);
    ov.open_document(&uri, &src_file_str, 1, code_cleaned.clone());

    let actions = query_actions_with_multi_file_resolve(&uri, 2, 12, &ws, &ov);
    let inline_action = actions.iter().find(|(t, _)| t == "Inline into all callers");
    assert!(inline_action.is_some(), "Expected 'Inline into all callers' for compound Self return type");

    let (_, changes) = inline_action.unwrap();
    let edits = changes.get(&uri).expect("lib.rs edits missing");
    let updated = apply_text_edits(&code_cleaned, edits);

    assert!(updated.contains("Dir(val)") || updated.contains("Dir("), "Self must be rewritten to Dir");
    assert_rustc_compiles(&updated, "Inline into all callers: compound Self return type rewritten to Dir");
}

#[test]
fn test_inline_method_on_generic_struct_with_concrete_ret_ty_compiles() {
    let temp_dir = std::env::temp_dir().join(format!("test_inline_generic_concrete_ret_{}", std::process::id()));
    let _ = fs::create_dir_all(&temp_dir);
    let src_file = temp_dir.join("lib.rs");
    let src_file_str = src_file.to_str().unwrap().to_string();

    let mut builder = FixtureIndexBuilder::new("test_generic_concrete_ret", "0.1.0");
    let code = concat!(
        "use std::io;\n",
        "pub struct Sink<W>(pub W);\n",
        "impl<W: io::Write> Sink<W> {\n",
        "    pub(crate) fn «write_bytes»(&mut self, buf: &[u8]) -> io::Result<()> {\n",
        "        self.0.write_all(buf)?;\n",
        "        Ok(())\n",
        "    }\n",
        "}\n",
        "pub fn caller<W: io::Write>(mut s: Sink<W>) -> io::Result<()> {\n",
        "    s.write_bytes(b\"hello\")\n",
        "}\n",
    );
    let (code_cleaned, def_ids) = builder.add_defs_with_ids(code, &src_file_str, "test_generic_concrete_ret", DefKind::Fn);
    let _ = fs::write(&src_file, &code_cleaned);

    builder.add_ref("test_generic_concrete_ret::write_bytes", &src_file_str, 4, 19, 4, 30, Some(def_ids[0]), Some(&src_file_str), 4, 19, DefKind::Fn);
    builder.add_ref("test_generic_concrete_ret::write_bytes", &src_file_str, 10, 7, 10, 18, Some(def_ids[0]), Some(&src_file_str), 4, 19, DefKind::Fn);

    let ridx_file = temp_dir.join("index.ridx");
    builder.write_to_file(&ridx_file);
    let mut ws = WorkspaceIndex::open(&ridx_file).unwrap();
    ws.set_workspace_root(Some(temp_dir.clone()));
    let mut ov = OverlayManager::new();
    let uri = format!("file://{}", src_file_str);
    ov.open_document(&uri, &src_file_str, 1, code_cleaned.clone());

    let actions = query_actions_with_multi_file_resolve(&uri, 3, 19, &ws, &ov);
    let inline_action = actions.iter().find(|(t, _)| t == "Inline into all callers");
    assert!(inline_action.is_some(), "Expected 'Inline into all callers' for concrete Result return type on generic struct");

    let (_, changes) = inline_action.unwrap();
    let edits = changes.get(&uri).expect("lib.rs edits missing");
    let updated = apply_text_edits(&code_cleaned, edits);

    assert_rustc_compiles(&updated, "Inline into all callers: concrete Result return type on generic struct compiles");
}

#[test]
fn test_inline_fn_with_early_return_compiles() {
    let temp_dir = tempfile::tempdir().unwrap().into_path();
    let src_file = temp_dir.join("lib.rs");
    let src_file_str = src_file.to_str().unwrap().to_string();

    let mut builder = RidxTestWorkspaceBuilder::new(&temp_dir);
    let code = concat!(
        "pub struct Item {\n",
        "    val: i32,\n",
        "}\n",
        "impl Item {\n",
        "    fn check(&self, limit: i32) -> Option<i32> {\n",
        "        if self.val > limit {\n",
        "            return None;\n",
        "        }\n",
        "        Some(self.val)\n",
        "    }\n",
        "}\n",
        "pub fn caller(item: &Item) -> Option<i32> {\n",
        "    item.check(10)\n",
        "}\n",
    );
    let (code_cleaned, def_ids) = builder.add_defs_with_ids(code, &src_file_str, "test_early_return", DefKind::Fn);
    let _ = fs::write(&src_file, &code_cleaned);

    builder.add_ref("test_early_return::check", &src_file_str, 5, 8, 5, 13, Some(def_ids[0]), Some(&src_file_str), 5, 8, DefKind::Fn);
    builder.add_ref("test_early_return::check", &src_file_str, 13, 10, 13, 15, Some(def_ids[0]), Some(&src_file_str), 5, 8, DefKind::Fn);

    let ridx_file = temp_dir.join("index.ridx");
    builder.write_to_file(&ridx_file);
    let mut ws = WorkspaceIndex::open(&ridx_file).unwrap();
    ws.set_workspace_root(Some(temp_dir.clone()));
    let mut ov = OverlayManager::new();
    let uri = format!("file://{}", src_file_str);
    ov.open_document(&uri, &src_file_str, 1, code_cleaned.clone());

    let actions = query_actions_with_multi_file_resolve(&uri, 4, 8, &ws, &ov);
    let inline_action = actions.iter().find(|(t, _)| t == "Inline into all callers");
    assert!(inline_action.is_some(), "Expected 'Inline into all callers' for early return fn");

    let (_, changes) = inline_action.unwrap();
    let edits = changes.get(&uri).expect("lib.rs edits missing");
    let updated = apply_text_edits(&code_cleaned, edits);

    assert_rustc_compiles(&updated, "Inline into all callers: early return compiles");
}
