/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/prepareCallHierarchy (Domain 6, Part 4).
//! Tests const, async, unsafe, extern C, generic functions, and trait methods.

use ridx::lsp::dispatch::dispatch_lsp_message;
use ridx::lsp::lifecycle::LifecycleManager;
use ridx::lsp::protocol::path_to_uri;
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::OverlayManager;
use ridx_format::{DefKind, IndexWriter, RawDefinition};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::fs::File;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

static CALL_COUNTER_P4: AtomicU64 = AtomicU64::new(100);

struct TempShard {
    dir: PathBuf,
    shard: PathBuf,
}

impl Drop for TempShard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[allow(clippy::too_many_arguments, dead_code)]
fn rdef(
    id: u32,
    kind: DefKind,
    vis: u8,
    mac: bool,
    name: impl Into<String>,
    path: impl Into<String>,
    sig: Option<String>,
    file: impl Into<String>,
    sl: u32,
    sc: u32,
    el: u32,
    ec: u32,
) -> RawDefinition {
    RawDefinition {
        def_id: id,
        kind,
        visibility: vis,
        is_macro_expansion: mac,
        name: name.into(),
        path: path.into(),
        signature: sig,
        file: file.into(),
        start_line: sl,
        start_col: sc,
        end_line: el,
        end_col: ec,
    }
}

fn create_custom_shard(name: &str, defs: Vec<RawDefinition>) -> TempShard {
    let id = CALL_COUNTER_P4.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_prepcall_p4_{}_{}_{}", name, std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let mut writer = IndexWriter::new(name, "0.1.0");
    for def in defs {
        writer.add_definition(def);
    }
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    TempShard { dir, shard }
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
fn test_prepare_call_hierarchy_const_fn_declaration() {
    let lib_str = "/mock/prepcall/const_fn/src/lib.rs";
    let ts = create_custom_shard(
        "const_fn",
        vec![rdef(1, DefKind::Fn, 1, false, "max_capacity", "const_fn::max_capacity", Some("pub const fn max_capacity() -> usize".to_string()), lib_str, 1, 1, 1, 13)],
    );
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let req = json!({
        "jsonrpc": "2.0", "id": 621, "method": "textDocument/prepareCallHierarchy",
        "params": {
            "textDocument": { "uri": path_to_uri(lib_str) },
            "position": { "line": 0, "character": 4 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 621);
    assert_eq!(
        resp["result"],
        json!([{
            "name": "max_capacity",
            "kind": 12,
            "detail": "pub const fn max_capacity() -> usize",
            "uri": path_to_uri(lib_str),
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 12 } },
            "selectionRange": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 12 } }
        }])
    );
}

#[test]
fn test_prepare_call_hierarchy_async_fn_declaration() {
    let lib_str = "/mock/prepcall/async_fn/src/lib.rs";
    let ts = create_custom_shard(
        "async_fn",
        vec![rdef(2, DefKind::Fn, 1, false, "fetch_async", "async_fn::fetch_async", Some("pub async fn fetch_async() -> Result<(), ()>".to_string()), lib_str, 5, 1, 5, 12)],
    );
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let req = json!({
        "jsonrpc": "2.0", "id": 622, "method": "textDocument/prepareCallHierarchy",
        "params": {
            "textDocument": { "uri": path_to_uri(lib_str) },
            "position": { "line": 4, "character": 6 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 622);
    assert_eq!(
        resp["result"],
        json!([{
            "name": "fetch_async",
            "kind": 12,
            "detail": "pub async fn fetch_async() -> Result<(), ()>",
            "uri": path_to_uri(lib_str),
            "range": { "start": { "line": 4, "character": 0 }, "end": { "line": 4, "character": 11 } },
            "selectionRange": { "start": { "line": 4, "character": 0 }, "end": { "line": 4, "character": 11 } }
        }])
    );
}

#[test]
fn test_prepare_call_hierarchy_unsafe_fn_declaration() {
    let lib_str = "/mock/prepcall/unsafe_fn/src/lib.rs";
    let ts = create_custom_shard(
        "unsafe_fn",
        vec![rdef(3, DefKind::Fn, 1, false, "raw_poke", "unsafe_fn::raw_poke", Some("pub unsafe fn raw_poke(ptr: *mut u8)".to_string()), lib_str, 10, 1, 10, 9)],
    );
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let req = json!({
        "jsonrpc": "2.0", "id": 623, "method": "textDocument/prepareCallHierarchy",
        "params": {
            "textDocument": { "uri": path_to_uri(lib_str) },
            "position": { "line": 9, "character": 2 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 623);
    assert_eq!(
        resp["result"],
        json!([{
            "name": "raw_poke",
            "kind": 12,
            "detail": "pub unsafe fn raw_poke(ptr: *mut u8)",
            "uri": path_to_uri(lib_str),
            "range": { "start": { "line": 9, "character": 0 }, "end": { "line": 9, "character": 8 } },
            "selectionRange": { "start": { "line": 9, "character": 0 }, "end": { "line": 9, "character": 8 } }
        }])
    );
}

#[test]
fn test_prepare_call_hierarchy_extern_c_fn_declaration() {
    let lib_str = "/mock/prepcall/extern_c/src/lib.rs";
    let ts = create_custom_shard(
        "extern_c",
        vec![rdef(4, DefKind::Fn, 1, false, "c_abi_call", "extern_c::c_abi_call", Some("pub extern \"C\" fn c_abi_call() -> i32".to_string()), lib_str, 3, 1, 3, 11)],
    );
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let req = json!({
        "jsonrpc": "2.0", "id": 624, "method": "textDocument/prepareCallHierarchy",
        "params": {
            "textDocument": { "uri": path_to_uri(lib_str) },
            "position": { "line": 2, "character": 5 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 624);
    assert_eq!(
        resp["result"],
        json!([{
            "name": "c_abi_call",
            "kind": 12,
            "detail": "pub extern \"C\" fn c_abi_call() -> i32",
            "uri": path_to_uri(lib_str),
            "range": { "start": { "line": 2, "character": 0 }, "end": { "line": 2, "character": 10 } },
            "selectionRange": { "start": { "line": 2, "character": 0 }, "end": { "line": 2, "character": 10 } }
        }])
    );
}

#[test]
fn test_prepare_call_hierarchy_generic_fn_with_type_params() {
    let lib_str = "/mock/prepcall/generic_fn/src/lib.rs";
    let ts = create_custom_shard(
        "generic_fn",
        vec![rdef(5, DefKind::Fn, 1, false, "identity", "generic_fn::identity", Some("pub fn identity<T: Clone>(val: T) -> T".to_string()), lib_str, 1, 1, 1, 9)],
    );
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let req = json!({
        "jsonrpc": "2.0", "id": 625, "method": "textDocument/prepareCallHierarchy",
        "params": {
            "textDocument": { "uri": path_to_uri(lib_str) },
            "position": { "line": 0, "character": 2 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 625);
    assert_eq!(
        resp["result"],
        json!([{
            "name": "identity",
            "kind": 12,
            "detail": "pub fn identity<T: Clone>(val: T) -> T",
            "uri": path_to_uri(lib_str),
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 8 } },
            "selectionRange": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 8 } }
        }])
    );
}

#[test]
fn test_prepare_call_hierarchy_generic_fn_with_where_clause() {
    let lib_str = "/mock/prepcall/where_fn/src/lib.rs";
    let ts = create_custom_shard(
        "where_fn",
        vec![rdef(6, DefKind::Fn, 1, false, "convert", "where_fn::convert", Some("pub fn convert<A, B>(a: A) -> B where A: Into<B>".to_string()), lib_str, 8, 1, 8, 8)],
    );
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let req = json!({
        "jsonrpc": "2.0", "id": 626, "method": "textDocument/prepareCallHierarchy",
        "params": {
            "textDocument": { "uri": path_to_uri(lib_str) },
            "position": { "line": 7, "character": 3 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 626);
    assert_eq!(
        resp["result"],
        json!([{
            "name": "convert",
            "kind": 12,
            "detail": "pub fn convert<A, B>(a: A) -> B where A: Into<B>",
            "uri": path_to_uri(lib_str),
            "range": { "start": { "line": 7, "character": 0 }, "end": { "line": 7, "character": 7 } },
            "selectionRange": { "start": { "line": 7, "character": 0 }, "end": { "line": 7, "character": 7 } }
        }])
    );
}

#[test]
fn test_prepare_call_hierarchy_trait_default_method() {
    let lib_str = "/mock/prepcall/trait_methods/src/lib.rs";
    let ts = create_custom_shard(
        "trait_methods",
        vec![rdef(
            7,
            DefKind::AssocFn,
            1,
            false,
            "default_action",
            "trait_methods::MyTrait::default_action",
            Some("fn default_action(&self) -> bool".to_string()),
            lib_str,
            12,
            5,
            12,
            19,
        )],
    );
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let req = json!({
        "jsonrpc": "2.0", "id": 627, "method": "textDocument/prepareCallHierarchy",
        "params": {
            "textDocument": { "uri": path_to_uri(lib_str) },
            "position": { "line": 11, "character": 8 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 627);
    assert_eq!(
        resp["result"],
        json!([{
            "name": "default_action",
            "kind": 6,
            "detail": "fn default_action(&self) -> bool",
            "uri": path_to_uri(lib_str),
            "range": { "start": { "line": 11, "character": 4 }, "end": { "line": 11, "character": 18 } },
            "selectionRange": { "start": { "line": 11, "character": 4 }, "end": { "line": 11, "character": 18 } }
        }])
    );
}

#[test]
fn test_prepare_call_hierarchy_trait_required_method() {
    let lib_str = "/mock/prepcall/trait_methods/src/lib.rs";
    let ts = create_custom_shard(
        "trait_methods2",
        vec![rdef(
            8,
            DefKind::AssocFn,
            1,
            false,
            "must_implement",
            "trait_methods2::MyTrait::must_implement",
            Some("fn must_implement(&self)".to_string()),
            lib_str,
            15,
            5,
            15,
            19,
        )],
    );
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let req = json!({
        "jsonrpc": "2.0", "id": 628, "method": "textDocument/prepareCallHierarchy",
        "params": {
            "textDocument": { "uri": path_to_uri(lib_str) },
            "position": { "line": 14, "character": 6 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 628);
    assert_eq!(
        resp["result"],
        json!([{
            "name": "must_implement",
            "kind": 6,
            "detail": "fn must_implement(&self)",
            "uri": path_to_uri(lib_str),
            "range": { "start": { "line": 14, "character": 4 }, "end": { "line": 14, "character": 18 } },
            "selectionRange": { "start": { "line": 14, "character": 4 }, "end": { "line": 14, "character": 18 } }
        }])
    );
}
