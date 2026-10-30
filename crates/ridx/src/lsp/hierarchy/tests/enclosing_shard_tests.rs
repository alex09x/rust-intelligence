/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! incomingCalls when the call site's file is indexed by two shards of one crate: the lib target's
//! shard holds no `#[cfg(test)]` function, the test target's shard holds them. The caller of a call
//! written in a test function exists only in the test shard, and when a test-only function is
//! nested in a function both shards hold, the nested one is the caller whichever shard sorts first.

use super::super::scan::find_enclosing_function;
use crate::lsp::hierarchy::call::handle_call_hierarchy_incoming_calls;
use crate::lsp::protocol::{path_to_uri, PositionConverter};
use crate::overlay::OverlayManager;
use ridx_format::{DefKind, IndexWriter, RawDefinition, RawReference, WorkspaceIndex};
use serde_json::json;
use std::fs;
use std::path::PathBuf;

static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// `helper` on line 1, `lib_caller` calling it on line 3, a test module whose `calls_helper`
/// (line 8) calls it on line 9.
const MODULE_CODE: &str = "pub fn helper() -> u32 { 1 }
pub fn lib_caller() -> u32 {
    helper()
}
#[cfg(test)]
mod tests {
    #[test]
    fn calls_helper() {
        assert_eq!(super::helper(), 1);
    }
}
";

/// `helper` on line 1, `outer` (lines 2-8) holding a test-only nested `inner` (lines 4-6) that
/// calls `helper` on line 5, and a call from `outer` itself on line 7.
const NESTED_CODE: &str = "pub fn helper() -> u32 { 1 }
pub fn outer() -> u32 {
    #[cfg(test)]
    fn inner() -> u32 {
        helper()
    }
    helper()
}
";

fn def(id: u32, name: &str, path: &str, line: u32, start_col: u32, file: &str) -> RawDefinition {
    RawDefinition {
        def_id: id,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        file: file.to_string(),
        name: name.to_string(),
        start_line: line,
        start_col,
        end_line: line,
        end_col: start_col + name.len() as u32,
        path: path.to_string(),
        signature: Some("fn() -> u32".to_string()),
    }
}

/// A call of `helper` (definition 1 on line 1) at `(line, start_col)`.
fn call_of_helper(line: u32, start_col: u32, file: &str) -> RawReference {
    RawReference {
        file: file.to_string(),
        start_line: line,
        start_col,
        end_line: line,
        end_col: start_col + 6,
        target_def_id: Some(1),
        target_path: "unit_crate::helper".to_string(),
        target_unit: None,
        target_file: Some(file.to_string()),
        target_line: 1,
        target_col: 8,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    }
}

type Shard = (&'static str, Vec<RawDefinition>, Vec<RawReference>);

/// A workspace of shards of the crate `unit_crate` over the one file `src/lib.rs` holding `code`;
/// the shards are opened in the order of their names.
fn workspace(code: &str, shards: impl FnOnce(&str) -> Vec<Shard>) -> (PathBuf, WorkspaceIndex) {
    let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_shards_{}_{}", std::process::id(), id));
    let _ = fs::create_dir_all(dir.join("src"));
    let file = dir.join("src/lib.rs");
    fs::write(&file, code).unwrap();
    let shards = shards(file.to_str().unwrap());
    let count = shards.len();
    for (name, defs, refs) in shards {
        let mut writer = IndexWriter::new("unit_crate", "0.1.0");
        for d in defs {
            writer.add_definition(d);
        }
        for r in refs {
            writer.add_reference(r);
        }
        let mut out = fs::File::create(dir.join(name)).unwrap();
        writer.write_to(&mut out).unwrap();
    }
    let ws = WorkspaceIndex::open(&dir).unwrap();
    assert_eq!(ws.readers().len(), count);
    (dir, ws)
}

/// The lib shard (`a_lib.ridx`, first) with `helper` and `lib_caller`; the test shard with those,
/// `tests::calls_helper` and its call.
fn module_workspace() -> (PathBuf, WorkspaceIndex) {
    workspace(MODULE_CODE, |f| {
        let lib = vec![def(1, "helper", "unit_crate::helper", 1, 8, f), def(2, "lib_caller", "unit_crate::lib_caller", 2, 8, f)];
        let mut test = lib.clone();
        test.push(def(3, "calls_helper", "unit_crate::tests::calls_helper", 8, 8, f));
        vec![("a_lib.ridx", lib, vec![call_of_helper(3, 5, f)]), ("b_test.ridx", test, vec![call_of_helper(3, 5, f), call_of_helper(9, 27, f)])]
    })
}

/// The lib shard with `helper` and `outer` and the call on line 7; the test shard with those,
/// the nested `inner` and both calls. `lib_first` chooses which shard name sorts first.
fn nested_workspace(lib_first: bool) -> (PathBuf, WorkspaceIndex) {
    workspace(NESTED_CODE, |f| {
        let lib = vec![def(1, "helper", "unit_crate::helper", 1, 8, f), def(2, "outer", "unit_crate::outer", 2, 8, f)];
        let mut test = lib.clone();
        test.push(def(3, "inner", "unit_crate::outer::inner", 4, 8, f));
        let (lib_name, test_name) = if lib_first { ("a_lib.ridx", "b_test.ridx") } else { ("b_lib.ridx", "a_test.ridx") };
        vec![(lib_name, lib, vec![call_of_helper(7, 5, f)]), (test_name, test, vec![call_of_helper(5, 9, f), call_of_helper(7, 5, f)])]
    })
}

/// Sorted `(caller name, 0-based lines of its call sites)` of incomingCalls on `helper`.
fn incoming_callers_of_helper(dir: &PathBuf, ws: &WorkspaceIndex) -> Vec<(String, Vec<u64>)> {
    let uri = path_to_uri(dir.join("src/lib.rs").to_str().unwrap());
    let req = json!({ "params": { "item": { "name": "helper", "uri": uri,
        "range": { "start": { "line": 0, "character": 7 }, "end": { "line": 0, "character": 13 } } } } });
    let (overlay_mgr, mut conv) = (OverlayManager::new(), PositionConverter::new());
    let res = handle_call_hierarchy_incoming_calls(&req, Some(ws), &overlay_mgr, &mut conv);
    let mut callers: Vec<(String, Vec<u64>)> = res
        .as_array()
        .unwrap()
        .iter()
        .map(|it| {
            let lines = it["fromRanges"].as_array().unwrap().iter().map(|r| r["start"]["line"].as_u64().unwrap()).collect();
            (it["from"]["name"].as_str().unwrap().to_string(), lines)
        })
        .collect();
    callers.sort();
    callers
}

#[test]
fn a_caller_known_only_to_the_test_shard_of_the_file_is_listed() {
    let (dir, ws) = module_workspace();
    let callers = incoming_callers_of_helper(&dir, &ws);
    assert_eq!(callers, vec![("calls_helper".to_string(), vec![8]), ("lib_caller".to_string(), vec![2])]);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn the_enclosing_function_is_looked_up_in_every_shard_of_the_file() {
    let (dir, ws) = module_workspace();
    let file = dir.join("src/lib.rs");
    let lines: Vec<&str> = MODULE_CODE.lines().collect();
    let (in_test, end) = find_enclosing_function(&ws, file.to_str().unwrap(), 9, &lines).unwrap();
    assert_eq!((in_test.name, in_test.path, end), ("calls_helper", "unit_crate::tests::calls_helper", 10));
    let (in_lib, end) = find_enclosing_function(&ws, file.to_str().unwrap(), 3, &lines).unwrap();
    assert_eq!((in_lib.name, end), ("lib_caller", 4));
    assert!(find_enclosing_function(&ws, file.to_str().unwrap(), 6, &lines).is_none());
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn the_innermost_function_wins_across_shards_in_either_order() {
    for lib_first in [true, false] {
        let (dir, ws) = nested_workspace(lib_first);
        assert_eq!(incoming_callers_of_helper(&dir, &ws), vec![("inner".to_string(), vec![4]), ("outer".to_string(), vec![6])], "lib shard first: {lib_first}");
        let file = dir.join("src/lib.rs");
        let lines: Vec<&str> = NESTED_CODE.lines().collect();
        let (inner, end) = find_enclosing_function(&ws, file.to_str().unwrap(), 5, &lines).unwrap();
        assert_eq!((inner.name, end), ("inner", 6), "lib shard first: {lib_first}");
        let (outer, end) = find_enclosing_function(&ws, file.to_str().unwrap(), 7, &lines).unwrap();
        assert_eq!((outer.name, end), ("outer", 8), "lib shard first: {lib_first}");
        let _ = fs::remove_dir_all(dir);
    }
}
