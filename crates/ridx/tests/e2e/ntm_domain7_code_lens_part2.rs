/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for codeLens/resolve (Domain 7, Part 2).
//! Tests reference counting, singular/plural title formatting, command arguments, and roundtrip resolution.

use ridx::lsp::dispatch::dispatch_lsp_message;
use ridx::lsp::lifecycle::LifecycleManager;
use ridx::lsp::protocol::path_to_uri;
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::OverlayManager;
use ridx_format::{DefKind, IndexWriter, RawDefinition, RawReference};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

static LENS_P2_COUNTER: AtomicU64 = AtomicU64::new(1);

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
    let id = LENS_P2_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_lens_p2_{}_{}_{}", name, std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let mut writer = IndexWriter::new(name, "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "target_fn".to_string(),
        path: format!("{}::target_fn", name),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: 1,
        start_col: 4,
        end_line: 1,
        end_col: 13,
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
        dispatch_lsp_message(msg.clone(), &mut self.writer, &mut self.lifecycle, &mut self.multi_root, &mut self.overlay_mgr, &self.cancelled, None).unwrap()
    }
    fn parse_response(&mut self) -> Value {
        let text = String::from_utf8(self.writer.clone()).unwrap();
        self.writer.clear();
        let body = text.split("\r\n\r\n").nth(1).unwrap();
        serde_json::from_str(body).unwrap()
    }
}

#[test]
fn test_code_lens_resolve_with_zero_references() {
    let ts = create_temp_shard("zero_ref");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn target_fn() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let req = json!({
        "jsonrpc": "2.0", "id": 771, "method": "codeLens/resolve",
        "params": {
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 9 } },
            "data": { "uri": to_uri(&file), "line": 1, "character": 1, "name": "target_fn" }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 771);
    assert_eq!(resp["result"]["command"]["title"], "References: target_fn");
    assert_eq!(resp["result"]["command"]["command"], "ridx.showReferences");
}

#[test]
fn test_code_lens_resolve_with_single_reference() {
    let id = LENS_P2_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_lens_p2_1ref_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let mut writer = IndexWriter::new("one_ref_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "target_fn".to_string(),
        path: "one_ref_crate::target_fn".to_string(),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: 1,
        start_col: 4,
        end_line: 1,
        end_col: 13,
    });
    writer.add_reference(RawReference {
        file: "src/lib.rs".to_string(),
        start_line: 4,
        start_col: 5,
        end_line: 4,
        end_col: 14,
        target_def_id: Some(1),
        target_path: "one_ref_crate::target_fn".to_string(),
        target_file: Some("src/lib.rs".to_string()),
        target_unit: None,
        target_line: 1,
        target_col: 1,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    let file = dir.join("src/lib.rs");
    std::fs::write(&file, "fn target_fn() {}\nfn caller() {\n    target_fn();\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);
    let req = json!({
        "jsonrpc": "2.0", "id": 772, "method": "codeLens/resolve",
        "params": {
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 9 } },
            "data": { "uri": to_uri(&file), "line": 1, "character": 4, "name": "target_fn" }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["result"]["command"]["title"], "1 reference");
}

#[test]
fn test_code_lens_resolve_with_multiple_references() {
    let id = LENS_P2_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_lens_p2_2ref_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let mut writer = IndexWriter::new("two_ref_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "target_fn".to_string(),
        path: "two_ref_crate::target_fn".to_string(),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: 1,
        start_col: 4,
        end_line: 1,
        end_col: 13,
    });
    writer.add_reference(RawReference {
        file: "src/lib.rs".to_string(),
        start_line: 3,
        start_col: 5,
        end_line: 3,
        end_col: 14,
        target_def_id: Some(1),
        target_path: "two_ref_crate::target_fn".to_string(),
        target_file: Some("src/lib.rs".to_string()),
        target_unit: None,
        target_line: 1,
        target_col: 1,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.add_reference(RawReference {
        file: "src/lib.rs".to_string(),
        start_line: 4,
        start_col: 5,
        end_line: 4,
        end_col: 14,
        target_def_id: Some(1),
        target_path: "two_ref_crate::target_fn".to_string(),
        target_file: Some("src/lib.rs".to_string()),
        target_unit: None,
        target_line: 1,
        target_col: 1,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    let file = dir.join("src/lib.rs");
    std::fs::write(&file, "fn target_fn() {}\nfn c() {\n    target_fn();\n    target_fn();\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);
    let req = json!({
        "jsonrpc": "2.0", "id": 773, "method": "codeLens/resolve",
        "params": {
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 9 } },
            "data": { "uri": to_uri(&file), "line": 1, "character": 4, "name": "target_fn" }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["result"]["command"]["title"], "2 references");
}

#[test]
fn test_code_lens_resolve_plural_title_formatting() {
    let ts = create_temp_shard("plural");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn target_fn() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let req = json!({
        "jsonrpc": "2.0", "id": 774, "method": "codeLens/resolve",
        "params": {
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 9 } },
            "data": { "uri": to_uri(&file), "line": 1, "character": 1, "name": "target_fn" }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let args = resp["result"]["command"]["arguments"].as_array().unwrap();
    assert_eq!(args.len(), 3);
    assert_eq!(args[0], to_uri(&file));
    assert_eq!(args[1]["line"], 0);
    assert_eq!(args[1]["character"], 0);
}

#[test]
fn test_code_lens_resolve_singular_title_formatting() {
    let ts = create_temp_shard("singular");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn target_fn() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let req = json!({
        "jsonrpc": "2.0", "id": 775, "method": "codeLens/resolve",
        "params": {
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 9 } },
            "data": { "uri": to_uri(&file), "line": 1, "character": 1, "name": "target_fn" }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["result"]["command"]["command"], "ridx.showReferences");
}

#[test]
fn test_code_lens_resolve_overlay_dirty_mapped_coords() {
    let ts = create_temp_shard("overlay_dirty");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn target_fn() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": { "uri": to_uri(&file), "languageId": "rust", "version": 1, "text": "// comment\nfn target_fn() {}\n" }
        }
    });
    assert!(dsp.dispatch(&open_msg));
    let req = json!({
        "jsonrpc": "2.0", "id": 776, "method": "codeLens/resolve",
        "params": {
            "range": { "start": { "line": 1, "character": 0 }, "end": { "line": 1, "character": 9 } },
            "data": { "uri": to_uri(&file), "line": 2, "character": 1, "name": "target_fn" }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 776);
    assert_eq!(resp["result"]["command"]["title"], "References: target_fn");
}

#[test]
fn test_code_lens_resolve_missing_data_returns_unchanged() {
    let ts = create_temp_shard("missing_data");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let req = json!({
        "jsonrpc": "2.0", "id": 777, "method": "codeLens/resolve",
        "params": {
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 5 } }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 777);
    assert_eq!(resp["result"]["range"]["start"]["line"], 0);
    // Positive control query by resolving lens with valid data
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn target_fn() {}\n").unwrap();
    let ctrl_req = json!({
        "jsonrpc": "2.0", "id": 7770, "method": "codeLens/resolve",
        "params": {
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 9 } },
            "data": { "uri": to_uri(&file), "line": 1, "character": 1, "name": "target_fn" }
        }
    });
    assert!(dsp.dispatch(&ctrl_req));
    let ctrl_resp = dsp.parse_response();
    assert_eq!(ctrl_resp["result"]["command"]["title"], "References: target_fn");
}

#[test]
fn test_code_lens_resolve_echoes_numeric_id() {
    let ts = create_temp_shard("num_id");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn target_fn() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let req = json!({
        "jsonrpc": "2.0", "id": 888111, "method": "codeLens/resolve",
        "params": {
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 9 } },
            "data": { "uri": to_uri(&file), "line": 1, "character": 1, "name": "target_fn" }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 888111);
    assert_eq!(resp["result"]["command"]["title"], "References: target_fn");
    assert_eq!(resp["result"]["command"]["command"], "ridx.showReferences");
}

#[test]
fn test_code_lens_resolve_echoes_string_id() {
    let ts = create_temp_shard("str_id");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn target_fn() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let req = json!({
        "jsonrpc": "2.0", "id": "lens-resolve-string-id", "method": "codeLens/resolve",
        "params": {
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 9 } },
            "data": { "uri": to_uri(&file), "line": 1, "character": 1, "name": "target_fn" }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], "lens-resolve-string-id");
    assert_eq!(resp["result"]["command"]["title"], "References: target_fn");
    assert_eq!(resp["result"]["command"]["command"], "ridx.showReferences");
}

#[test]
fn test_code_lens_resolve_roundtrip_from_code_lens_request() {
    let ts = create_temp_shard("roundtrip");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn target_fn() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let lens_req = json!({
        "jsonrpc": "2.0", "id": 780, "method": "textDocument/codeLens",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&lens_req));
    let lens_resp = dsp.parse_response();
    let raw_lens = &lens_resp["result"][0];
    let resolve_req = json!({
        "jsonrpc": "2.0", "id": 781, "method": "codeLens/resolve",
        "params": raw_lens
    });
    assert!(dsp.dispatch(&resolve_req));
    let resolve_resp = dsp.parse_response();
    assert_eq!(resolve_resp["id"], 781);
    assert_eq!(resolve_resp["result"]["command"]["title"], "References: target_fn");
}
