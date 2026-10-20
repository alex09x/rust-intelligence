/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/selectionRange (Domain 7, Part 10).
//! Tests moved from Part 2 to keep that file under the size limit: the gap between two items
//! and a method inside an impl block, both on the index-backed dispatcher.

use ridx::lsp::dispatch::dispatch_lsp_message;
use ridx::lsp::lifecycle::LifecycleManager;
use ridx::lsp::protocol::path_to_uri;
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::OverlayManager;
use ridx_format::{DefKind, IndexWriter, RawDefinition};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

static SEL_COUNTER10: AtomicU64 = AtomicU64::new(1000);

struct TempShard {
    dir: PathBuf,
    shard: PathBuf,
}

impl Drop for TempShard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn create_temp_shard(name: &str) -> TempShard {
    let id = SEL_COUNTER10.fetch_add(1, Ordering::Relaxed);
    let dir =
        std::env::temp_dir().join(format!("ridx_sel10_{}_{}_{}", name, std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let mut writer = IndexWriter::new(name, "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "test_fn".to_string(),
        path: format!("{}::test_fn", name),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 10,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    TempShard { dir, shard }
}

fn to_uri(p: &Path) -> String {
    path_to_uri(p.to_str().unwrap())
}

struct TestDispatcher {
    lifecycle: LifecycleManager,
    multi_root: MultiRootWorkspace,
    overlay_mgr: OverlayManager,
    cancelled: Arc<Mutex<HashSet<Value>>>,
    writer: Vec<u8>,
}

impl TestDispatcher {
    fn new(shard: PathBuf) -> Self {
        let mut d = Self {
            lifecycle: LifecycleManager::new(),
            multi_root: MultiRootWorkspace::new(),
            overlay_mgr: OverlayManager::new(),
            cancelled: Arc::new(Mutex::new(HashSet::new())),
            writer: Vec::new(),
        };
        assert!(d.multi_root.add_root(shard));
        let init_msg = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "capabilities": {} } });
        assert!(d.dispatch(&init_msg));
        d.writer.clear();
        d
    }

    fn dispatch(&mut self, msg: &Value) -> bool {
        dispatch_lsp_message(
            msg.clone(),
            &mut self.writer,
            &mut self.lifecycle,
            &mut self.multi_root,
            &mut self.overlay_mgr,
            &self.cancelled,
            None,
        )
        .unwrap()
    }

    fn parse_response(&mut self) -> Value {
        let text = String::from_utf8(self.writer.clone()).unwrap();
        self.writer.clear();
        let body = text.split("\r\n\r\n").nth(1).unwrap();
        serde_json::from_str(body).unwrap()
    }
}

/// Expected chain, innermost first, from `(start_line, start_char, end_line, end_char)`.
fn chain(ranges: &[(u32, u32, u32, u32)]) -> Value {
    let mut current = Value::Null;
    for &(sl, sc, el, ec) in ranges.iter().rev() {
        current = json!({
            "range": { "start": { "line": sl, "character": sc }, "end": { "line": el, "character": ec } },
            "parent": current
        });
    }
    current
}

fn open_doc(dsp: &mut TestDispatcher, file: &Path, text: &str) {
    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": { "textDocument": { "uri": to_uri(file), "languageId": "rust", "version": 1, "text": text } }
    });
    assert!(dsp.dispatch(&open_msg));
}

fn selection_ranges(
    dsp: &mut TestDispatcher,
    id: Value,
    file: &Path,
    positions: &[(u32, u32)],
) -> Value {
    let positions: Vec<Value> = positions
        .iter()
        .map(|&(l, c)| json!({ "line": l, "character": c }))
        .collect();
    let req = json!({
        "jsonrpc": "2.0", "id": id, "method": "textDocument/selectionRange",
        "params": { "textDocument": { "uri": to_uri(file) }, "positions": positions }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], id);
    resp["result"].clone()
}

#[test]
fn test_selection_range_cursor_between_functions() {
    let ts = create_temp_shard("between_fns");
    let file = ts.dir.join("src/lib.rs");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    open_doc(
        &mut dsp,
        &file,
        "fn f1() {\n    let a = 1;\n}\n\nfn f2() {\n    let b = 2;\n}\n",
    );

    // The blank line between the items belongs to the file only.
    let result = selection_ranges(&mut dsp, json!(209), &file, &[(3, 0)]);
    assert_eq!(result, json!([chain(&[(0, 0, 7, 0)])]));
    assert_eq!(result[0]["parent"], Value::Null);
}

#[test]
fn test_selection_range_nested_struct_impl_method() {
    let ts = create_temp_shard("struct_impl");
    let file = ts.dir.join("src/lib.rs");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    open_doc(
        &mut dsp, &file,
        "struct Container {\n    val: i32,\n}\nimpl Container {\n    fn read(&self) -> i32 {\n        self.val\n    }\n}\n",
    );

    // Cursor between `self` and `.`: the receiver token, method body, method item, impl block, file.
    let result = selection_ranges(&mut dsp, json!(210), &file, &[(5, 12)]);
    assert_eq!(
        result,
        json!([chain(&[
            (5, 8, 5, 12),
            (4, 26, 6, 5),
            (4, 4, 6, 5),
            (3, 15, 7, 1),
            (0, 0, 8, 0)
        ])])
    );
}
