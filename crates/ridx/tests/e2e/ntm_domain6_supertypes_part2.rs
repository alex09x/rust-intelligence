/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for typeHierarchy/supertypes (Domain 6, Part 2).
//! Tests enum impls, deduplication across blocks, references, fallback paths, and numeric IDs.

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

static SUPER_P2_COUNTER: AtomicU64 = AtomicU64::new(100);

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

#[allow(clippy::too_many_arguments, dead_code)]
fn rref(
    file: impl Into<String>,
    sl: u32,
    sc: u32,
    el: u32,
    ec: u32,
    td: Option<u32>,
    tp: impl Into<String>,
    tf: Option<String>,
    tu: Option<String>,
    tl: u32,
    tc: u32,
    kind: DefKind,
    w: bool,
    m: bool,
) -> RawReference {
    RawReference {
        file: file.into(),
        start_line: sl,
        start_col: sc,
        end_line: el,
        end_col: ec,
        target_def_id: td,
        target_path: tp.into(),
        target_file: tf,
        target_unit: tu,
        target_line: tl,
        target_col: tc,
        kind,
        is_write: w,
        is_macro_expansion: m,
    }
}

fn create_temp_shard(name: &str) -> TempShard {
    let id = SUPER_P2_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_super_p2_{}_{}_{}", name, std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let lib_file = dir.join("src/lib.rs");
    let lib_str = lib_file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new(name, "0.1.0");
    writer.add_definition(rdef(1, DefKind::Trait, 1, false, "Greeter", format!("{}::Greeter", name), Some("pub trait Greeter".to_string()), &lib_str, 1, 1, 1, 18));
    writer.add_definition(rdef(2, DefKind::Struct, 1, false, "Robot", format!("{}::Robot", name), Some("pub struct Robot".to_string()), &lib_str, 3, 1, 3, 17));
    writer.add_definition(rdef(3, DefKind::Impl, 1, false, "<Robot as Greeter>", "<Robot as Greeter>", Some("impl Greeter for Robot".to_string()), lib_str, 5, 1, 7, 2));
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
fn test_type_hierarchy_supertypes_enum_implementation() {
    let id = SUPER_P2_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_super_enum_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("enum_crate", "0.1.0");
    writer.add_definition(rdef(1, DefKind::Trait, 1, false, "Describable", "enum_crate::Describable", None, &lib_str, 1, 1, 1, 22));
    writer.add_definition(rdef(2, DefKind::Enum, 1, false, "Status", "enum_crate::Status", None, &lib_str, 3, 1, 3, 18));
    writer.add_definition(rdef(3, DefKind::Impl, 1, false, "<Status as Describable>", "<Status as Describable>", None, lib_str, 5, 1, 5, 35));
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "trait Describable {}\n\nenum Status { Ok, Err }\n\nimpl Describable for Status {}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);
    let req = json!({
        "jsonrpc": "2.0", "id": 706, "method": "typeHierarchy/supertypes",
        "params": {
            "item": { "name": "Status", "kind": 10, "uri": to_uri(&file) }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "Describable");
}

#[test]
fn test_type_hierarchy_supertypes_deduplication_across_blocks() {
    let id = SUPER_P2_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_super_dedup_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("dedup_crate", "0.1.0");
    writer.add_definition(rdef(1, DefKind::Trait, 1, false, "Greeter", "dedup_crate::Greeter", None, &lib_str, 1, 1, 1, 18));
    writer.add_definition(rdef(2, DefKind::Struct, 1, false, "Robot", "dedup_crate::Robot", None, &lib_str, 3, 1, 3, 17));
    writer.add_definition(rdef(3, DefKind::Impl, 1, false, "<Robot as Greeter>", "<Robot as Greeter>", None, &lib_str, 5, 1, 5, 25));
    writer.add_definition(rdef(4, DefKind::Impl, 1, false, "<Robot as Greeter>", "<Robot as Greeter>", None, lib_str, 7, 1, 7, 25));
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "trait Greeter {}\n\nstruct Robot;\n\nimpl Greeter for Robot {}\n\nimpl Greeter for Robot {}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);
    let req = json!({
        "jsonrpc": "2.0", "id": 707, "method": "typeHierarchy/supertypes",
        "params": {
            "item": { "name": "Robot", "kind": 23, "uri": to_uri(&file) }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "Greeter");
}

#[test]
fn test_type_hierarchy_supertypes_reference_based_impl_resolution() {
    let id = SUPER_P2_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_super_ref_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("ref_crate", "0.1.0");
    writer.add_definition(rdef(1, DefKind::Trait, 1, false, "Greeter", "ref_crate::Greeter", None, &lib_str, 1, 1, 1, 18));
    writer.add_definition(rdef(2, DefKind::Struct, 1, false, "Robot", "ref_crate::Robot", None, &lib_str, 3, 1, 3, 17));
    // Add reference on Robot at line 5 (in `impl Greeter for Robot`)
    writer.add_reference(rref(&lib_str, 5, 18, 5, 23, Some(2), "ref_crate::Robot", Some(lib_str.clone()), None, 3, 1, DefKind::Struct, false, false));
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "trait Greeter {}\n\nstruct Robot;\n\nimpl Greeter for Robot {}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);
    let req = json!({
        "jsonrpc": "2.0", "id": 708, "method": "typeHierarchy/supertypes",
        "params": {
            "item": { "name": "Robot", "kind": 23, "uri": to_uri(&file) }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "Greeter");
}

#[test]
fn test_type_hierarchy_supertypes_fallback_path_when_file_read_fails() {
    let id = SUPER_P2_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_super_fb_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let nonexistent_file = dir.join("src/nonexistent.rs");
    let lib_str = nonexistent_file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("fb_crate", "0.1.0");
    writer.add_definition(rdef(1, DefKind::Trait, 1, false, "Greeter", "fb_crate::Greeter", None, &lib_str, 1, 1, 1, 18));
    writer.add_definition(rdef(2, DefKind::Struct, 1, false, "Robot", "fb_crate::Robot", None, &lib_str, 3, 1, 3, 17));
    writer.add_definition(rdef(3, DefKind::Impl, 1, false, "<Robot as Greeter>", "<Robot as Greeter>", None, lib_str, 5, 1, 7, 2));
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    // Do NOT create nonexistent.rs on disk — fallback parsing extracts Greeter from path
    let mut dsp = TestDispatcher::new(shard);
    let req = json!({
        "jsonrpc": "2.0", "id": 709, "method": "typeHierarchy/supertypes",
        "params": {
            "item": { "name": "Robot", "kind": 23, "uri": to_uri(&nonexistent_file) }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "Greeter");
}

#[test]
fn test_type_hierarchy_supertypes_dispatcher_echoes_numeric_id() {
    let ts = create_temp_shard("num_id");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "trait Greeter {}\n\nstruct Robot;\n\nimpl Greeter for Robot {}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let req = json!({
        "jsonrpc": "2.0", "id": 654321, "method": "typeHierarchy/supertypes",
        "params": {
            "item": { "name": "Robot", "kind": 23, "uri": to_uri(&file) }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 654321);
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "Greeter");
}
