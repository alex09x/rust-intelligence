/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! incomingCalls resolves its item by position, not by the item's definition path alone: a
//! crate-root function of a test target has a one-segment path (`test`) that another shard's item
//! of the same name also carries, and the path lookup used to pick that other item.

use crate::lsp::hierarchy::call::handle_call_hierarchy_incoming_calls;
use crate::lsp::protocol::{path_to_uri, PositionConverter};
use crate::overlay::OverlayManager;
use ridx_format::{DefKind, IndexWriter, RawDefinition, RawReference, WorkspaceIndex};
use serde_json::json;
use std::fs;
use std::path::PathBuf;

static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The integration test file: `test` on line 1, `test_u8` on line 2 calling it on line 3.
const TEST_CODE: &str = "fn test() {}
fn test_u8() {
    test();
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
        signature: Some("fn()".to_string()),
    }
}

/// A workspace of two shards with their own unit keys: `a_other.ridx` (crate `other`, sorted
/// first) holding an unrelated `fn test` with the path `test`, and `b_test_de.ridx` (crate
/// `test_de`, an integration test target) holding the `fn test` of `tests/test_de.rs`, also with
/// the bare path `test`, and `test_u8` calling it.
fn colliding_workspace() -> (PathBuf, WorkspaceIndex) {
    let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_item_pos_{}_{}", std::process::id(), id));
    let _ = fs::create_dir_all(dir.join("other/src"));
    let _ = fs::create_dir_all(dir.join("tests"));
    let other_file = dir.join("other/src/lib.rs");
    fs::write(&other_file, "pub fn test() {}\n").unwrap();
    let test_file = dir.join("tests/test_de.rs");
    fs::write(&test_file, TEST_CODE).unwrap();
    let (o, t) = (other_file.to_str().unwrap(), test_file.to_str().unwrap());

    let mut other = IndexWriter::new("other", "0.1.0");
    other.set_unit_key("other-lib");
    other.add_definition(def(1, "test", "test", 1, 8, o));
    other
        .write_to(&mut fs::File::create(dir.join("a_other.ridx")).unwrap())
        .unwrap();

    let mut test_de = IndexWriter::new("test_de", "0.0.0");
    test_de.set_unit_key("test_de-test");
    test_de.add_definition(def(1, "test", "test", 1, 4, t));
    test_de.add_definition(def(2, "test_u8", "test_u8", 2, 4, t));
    test_de.add_reference(RawReference {
        file: t.to_string(),
        start_line: 3,
        start_col: 5,
        end_line: 3,
        end_col: 9,
        target_def_id: Some(1),
        target_path: "test".to_string(),
        target_unit: None,
        target_file: Some(t.to_string()),
        target_line: 1,
        target_col: 4,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    test_de
        .write_to(&mut fs::File::create(dir.join("b_test_de.ridx")).unwrap())
        .unwrap();

    let ws = WorkspaceIndex::open(&dir).unwrap();
    assert_eq!(ws.readers().len(), 2);
    (dir, ws)
}

pub(super) fn incoming_callers(
    ws: &WorkspaceIndex,
    file: &PathBuf,
    line0: u64,
    char0: u64,
) -> Vec<(String, Vec<u64>)> {
    let uri = path_to_uri(file.to_str().unwrap());
    let pos = json!({ "line": line0, "character": char0 });
    let req = json!({ "params": { "item": { "name": "test", "kind": 12, "uri": uri,
        "range": { "start": pos, "end": pos }, "selectionRange": { "start": pos, "end": pos } } } });
    let (overlay_mgr, mut conv) = (OverlayManager::new(), PositionConverter::new());
    let res = handle_call_hierarchy_incoming_calls(&req, Some(ws), &overlay_mgr, &mut conv);
    let mut callers: Vec<(String, Vec<u64>)> = res
        .as_array()
        .unwrap()
        .iter()
        .map(|it| {
            let lines = it["fromRanges"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| r["start"]["line"].as_u64().unwrap())
                .collect();
            (it["from"]["name"].as_str().unwrap().to_string(), lines)
        })
        .collect();
    callers.sort();
    callers
}

#[test]
fn the_item_of_a_test_target_with_a_bare_path_finds_its_callers() {
    let (dir, ws) = colliding_workspace();
    let callers = incoming_callers(&ws, &dir.join("tests/test_de.rs"), 0, 3);
    assert_eq!(callers, vec![("test_u8".to_string(), vec![2])]);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn the_same_named_item_of_the_other_shard_has_no_callers() {
    let (dir, ws) = colliding_workspace();
    assert_eq!(
        incoming_callers(&ws, &dir.join("other/src/lib.rs"), 0, 7),
        vec![]
    );
    let _ = fs::remove_dir_all(dir);
}
