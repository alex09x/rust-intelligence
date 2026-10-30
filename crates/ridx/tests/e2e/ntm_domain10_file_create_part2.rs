/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for workspace/willCreateFiles and workspace/didCreateFiles (Domain 10, Part 2).
//! Verifies request parameter edge cases, complex URI forms, and schema conformance for willCreateFiles.

use ridx::lsp::dispatch::dispatch_lsp_message;
use ridx::lsp::lifecycle::LifecycleManager;
use ridx::lsp::protocol::path_to_uri;
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::OverlayManager;
use ridx_format::{DefKind, IndexWriter, RawDefinition};
use serde_json::json;
use std::collections::HashSet;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

static CREATE_COUNTER_P2: AtomicU64 = AtomicU64::new(100);

struct TempShard {
    dir: PathBuf,
    shard: PathBuf,
}

impl Drop for TempShard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn create_temp_shard_with_symbol(name: &str, sym_name: &str) -> TempShard {
    let id = CREATE_COUNTER_P2.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_create_p2_{}_{}_{}", name, std::process::id(), id));
    std::fs::create_dir_all(&dir).unwrap();
    let shard = dir.join("index.ridx");
    let mut writer = IndexWriter::new(name, "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: sym_name.to_string(),
        path: format!("{}::{}", name, sym_name),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: 10,
        start_col: 5,
        end_line: 10,
        end_col: 15,
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
    cancelled: Arc<Mutex<HashSet<serde_json::Value>>>,
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

    fn dispatch(&mut self, msg: &serde_json::Value) -> bool {
        dispatch_lsp_message(msg.clone(), &mut self.writer, &mut self.lifecycle, &mut self.multi_root, &mut self.overlay_mgr, &self.cancelled, None).unwrap()
    }

    fn parse_response(&mut self) -> serde_json::Value {
        let text = String::from_utf8(self.writer.clone()).unwrap();
        self.writer.clear();
        let body = text.split("\r\n\r\n").nth(1).unwrap();
        serde_json::from_str(body).unwrap()
    }
}

#[test]
fn test_will_create_files_with_nested_subdirectories() {
    let ts = create_temp_shard_with_symbol("p2_nest", "NestSym");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let subfile = ts.dir.join("a/b/c/d/deep_module.rs");
    let req = json!({
        "jsonrpc": "2.0",
        "id": 201,
        "method": "workspace/willCreateFiles",
        "params": { "files": [{ "uri": to_uri(&subfile) }] }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 201);
    assert_eq!(resp["result"], json!({}));
}

#[test]
fn test_will_create_files_special_characters_in_filename() {
    let ts = create_temp_shard_with_symbol("p2_spec", "SpecSym");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let specfile = ts.dir.join("src/my-custom_module.v2.test.rs");
    let req = json!({
        "jsonrpc": "2.0",
        "id": 202,
        "method": "workspace/willCreateFiles",
        "params": { "files": [{ "uri": to_uri(&specfile) }] }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 202);
    assert_eq!(resp["result"], json!({}));
}

#[test]
fn test_will_create_files_windows_style_uri() {
    let ts = create_temp_shard_with_symbol("p2_win", "WinSym");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let req = json!({
        "jsonrpc": "2.0",
        "id": 203,
        "method": "workspace/willCreateFiles",
        "params": { "files": [{ "uri": "file:///C:/work/tester/project/src/main.rs" }] }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 203);
    assert_eq!(resp["result"], json!({}));
}

#[test]
fn test_will_create_files_with_query_params() {
    let ts = create_temp_shard_with_symbol("p2_query", "QuerySym");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let req = json!({
        "jsonrpc": "2.0",
        "id": 204,
        "method": "workspace/willCreateFiles",
        "params": { "files": [{ "uri": "file:///tmp/new_file.rs?branch=feature-x&commit=abc" }] }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 204);
    assert_eq!(resp["result"], json!({}));
}

#[test]
fn test_will_create_files_with_fragment() {
    let ts = create_temp_shard_with_symbol("p2_frag", "FragSym");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let req = json!({
        "jsonrpc": "2.0",
        "id": 205,
        "method": "workspace/willCreateFiles",
        "params": { "files": [{ "uri": "file:///tmp/new_file.rs#L10-L20" }] }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 205);
    assert_eq!(resp["result"], json!({}));
}

#[test]
fn test_will_create_files_duplicate_files_in_request() {
    let ts = create_temp_shard_with_symbol("p2_dup", "DupSym");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let target = ts.dir.join("src/dup.rs");
    let u = to_uri(&target);
    let req = json!({
        "jsonrpc": "2.0",
        "id": 206,
        "method": "workspace/willCreateFiles",
        "params": { "files": [{ "uri": u }, { "uri": u }] }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 206);
    assert_eq!(resp["result"], json!({}));
}

#[test]
fn test_will_create_files_extra_fields_in_file_object() {
    let ts = create_temp_shard_with_symbol("p2_extra", "ExtraSym");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let req = json!({
        "jsonrpc": "2.0",
        "id": 207,
        "method": "workspace/willCreateFiles",
        "params": {
            "files": [{
                "uri": "file:///tmp/extra_fields.rs",
                "customMetadata": "custom_data_v1",
                "createMode": 0o644
            }]
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 207);
    assert_eq!(resp["result"], json!({}));
}

#[test]
fn test_will_create_files_null_options() {
    let ts = create_temp_shard_with_symbol("p2_opts", "OptsSym");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let req = json!({
        "jsonrpc": "2.0",
        "id": 208,
        "method": "workspace/willCreateFiles",
        "params": {
            "files": [{ "uri": "file:///tmp/null_opts.rs" }],
            "options": serde_json::Value::Null
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 208);
    assert_eq!(resp["result"], json!({}));
}

#[test]
fn test_will_create_files_negative_numeric_id() {
    let ts = create_temp_shard_with_symbol("p2_neg", "NegSym");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let req = json!({
        "jsonrpc": "2.0",
        "id": -99,
        "method": "workspace/willCreateFiles",
        "params": { "files": [{ "uri": "file:///tmp/neg_id.rs" }] }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], -99);
    assert_eq!(resp["result"], json!({}));
}

#[test]
fn test_will_create_files_floating_point_id() {
    let ts = create_temp_shard_with_symbol("p2_float", "FloatSym");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let req = json!({
        "jsonrpc": "2.0",
        "id": 55.5,
        "method": "workspace/willCreateFiles",
        "params": { "files": [{ "uri": "file:///tmp/float_id.rs" }] }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 55.5);
    assert_eq!(resp["result"], json!({}));
}

#[test]
fn test_will_create_files_large_batch_fifty_files() {
    let ts = create_temp_shard_with_symbol("p2_fifty", "FiftySym");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let mut files = Vec::new();
    for i in 0..50 {
        files.push(json!({ "uri": format!("file:///tmp/batch_{}.rs", i) }));
    }
    let req = json!({
        "jsonrpc": "2.0",
        "id": 250,
        "method": "workspace/willCreateFiles",
        "params": { "files": files }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 250);
    assert_eq!(resp["result"], json!({}));
}
