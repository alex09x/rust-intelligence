/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/semanticTokens/full (Domain 7, Part 3).
//! Tests struct, enum, trait, type alias, const, static, and macro token encoding.

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

static ST_P3_COUNTER: AtomicU64 = AtomicU64::new(1);

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
            msg.clone(), &mut self.writer, &mut self.lifecycle,
            &mut self.multi_root, &mut self.overlay_mgr, &self.cancelled, None,
        ).unwrap()
    }

    fn parse_response(&mut self) -> Value {
        let text = String::from_utf8(self.writer.clone()).unwrap();
        self.writer.clear();
        let body = text.split("\r\n\r\n").nth(1).unwrap();
        serde_json::from_str(body).unwrap()
    }
}

#[test]
fn test_semantic_tokens_full_struct_and_fields() {
    let id = ST_P3_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_semtok_p3_struct_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("struct_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Struct, visibility: 1, is_macro_expansion: false,
        name: "Point".to_string(), path: "struct_crate::Point".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 1, start_col: 8, end_line: 1, end_col: 13,
    });
    writer.add_definition(RawDefinition {
        def_id: 2, kind: DefKind::Field, visibility: 1, is_macro_expansion: false,
        name: "x".to_string(), path: "struct_crate::Point::x".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 2, start_col: 5, end_line: 2, end_col: 6,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "struct Point {\n    x: i32,\n}\n").unwrap();

    let mut dsp = TestDispatcher::new(shard);
    let req = json!({
        "jsonrpc": "2.0", "id": 101, "method": "textDocument/semanticTokens/full",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 101);
    assert_eq!(resp["result"]["data"], json!([0, 7, 5, 5, 0, 1, 4, 1, 9, 0]));
}

#[test]
fn test_semantic_tokens_full_enum_and_variants() {
    let id = ST_P3_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_semtok_p3_enum_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("enum_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Enum, visibility: 1, is_macro_expansion: false,
        name: "Color".to_string(), path: "enum_crate::Color".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 1, start_col: 6, end_line: 1, end_col: 11,
    });
    writer.add_definition(RawDefinition {
        def_id: 2, kind: DefKind::Variant, visibility: 1, is_macro_expansion: false,
        name: "Red".to_string(), path: "enum_crate::Color::Red".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 2, start_col: 5, end_line: 2, end_col: 8,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "enum Color {\n    Red,\n}\n").unwrap();

    let mut dsp = TestDispatcher::new(shard);
    let req = json!({
        "jsonrpc": "2.0", "id": 102, "method": "textDocument/semanticTokens/full",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 102);
    assert_eq!(resp["result"]["data"], json!([0, 5, 5, 3, 0, 1, 4, 3, 10, 0]));
}

#[test]
fn test_semantic_tokens_full_trait_and_associated_items() {
    let id = ST_P3_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_semtok_p3_trait_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("trait_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Trait, visibility: 1, is_macro_expansion: false,
        name: "Greet".to_string(), path: "trait_crate::Greet".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 1, start_col: 7, end_line: 1, end_col: 12,
    });
    writer.add_definition(RawDefinition {
        def_id: 2, kind: DefKind::AssocFn, visibility: 1, is_macro_expansion: false,
        name: "hello".to_string(), path: "trait_crate::Greet::hello".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 2, start_col: 8, end_line: 2, end_col: 13,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "trait Greet {\n    fn hello();\n}\n").unwrap();

    let mut dsp = TestDispatcher::new(shard);
    let req = json!({
        "jsonrpc": "2.0", "id": 103, "method": "textDocument/semanticTokens/full",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 103);
    assert_eq!(resp["result"]["data"], json!([0, 6, 5, 4, 0, 1, 7, 5, 12, 2]));
}

#[test]
fn test_semantic_tokens_full_type_alias_and_type_param() {
    let id = ST_P3_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_semtok_p3_tyalias_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("alias_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::TyAlias, visibility: 1, is_macro_expansion: false,
        name: "Number".to_string(), path: "alias_crate::Number".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 1, start_col: 6, end_line: 1, end_col: 12,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "type Number = i64;\n").unwrap();

    let mut dsp = TestDispatcher::new(shard);
    let req = json!({
        "jsonrpc": "2.0", "id": 104, "method": "textDocument/semanticTokens/full",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 104);
    assert_eq!(resp["result"]["data"], json!([0, 5, 6, 1, 0]));
}

#[test]
fn test_semantic_tokens_full_constants_and_statics() {
    let id = ST_P3_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_semtok_p3_const_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("const_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Const, visibility: 1, is_macro_expansion: false,
        name: "MAX".to_string(), path: "const_crate::MAX".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 1, start_col: 7, end_line: 1, end_col: 10,
    });
    writer.add_definition(RawDefinition {
        def_id: 2, kind: DefKind::Static, visibility: 1, is_macro_expansion: false,
        name: "GLOBAL".to_string(), path: "const_crate::GLOBAL".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 2, start_col: 8, end_line: 2, end_col: 14,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "const MAX: u32 = 100;\nstatic GLOBAL: u32 = 200;\n").unwrap();

    let mut dsp = TestDispatcher::new(shard);
    let req = json!({
        "jsonrpc": "2.0", "id": 105, "method": "textDocument/semanticTokens/full",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 105);
    assert_eq!(resp["result"]["data"], json!([0, 6, 3, 8, 12, 1, 7, 6, 8, 8]));
}

#[test]
fn test_semantic_tokens_full_macro_definition() {
    let id = ST_P3_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_semtok_p3_macro_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("macro_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Macro, visibility: 1, is_macro_expansion: false,
        name: "my_macro".to_string(), path: "macro_crate::my_macro".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 1, start_col: 14, end_line: 1, end_col: 22,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "macro_rules! my_macro { () => {}; }\n").unwrap();

    let mut dsp = TestDispatcher::new(shard);
    let req = json!({
        "jsonrpc": "2.0", "id": 106, "method": "textDocument/semanticTokens/full",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 106);
    assert_eq!(resp["result"]["data"], json!([0, 13, 8, 13, 0]));
}
