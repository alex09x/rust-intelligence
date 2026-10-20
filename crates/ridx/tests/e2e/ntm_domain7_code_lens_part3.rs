/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Code lenses of open buffers for struct, enum, trait, const, static and type alias definitions:
//! the buffer holds the text the index describes, the lenses are the index's.

use ridx::lsp::dispatch::dispatch_lsp_message;
use ridx::lsp::lifecycle::LifecycleManager;
use ridx::lsp::protocol::path_to_uri;
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::OverlayManager;
use ridx_format::{normalized_source_hash, DefKind, IndexWriter, RawDefinition};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

static LENS_P3_COUNTER: AtomicU64 = AtomicU64::new(1);

fn to_uri(p: &Path) -> String {
    path_to_uri(p.to_str().unwrap())
}

/// A crate whose `src/lib.rs` holds `text`, indexed into `shard` with one definition per
/// `(kind, name)` at the first occurrence of `name` and with the hash of `text`: a buffer opened
/// with `text` is the text the index describes.
struct Indexed {
    dir: PathBuf,
    shard: PathBuf,
}

impl Drop for Indexed {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Indexed {
    fn file(&self) -> PathBuf {
        self.dir.join("src/lib.rs")
    }
}

fn indexed(name: &str, text: &str, defs: &[(DefKind, &str)]) -> Indexed {
    let id = LENS_P3_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_lens_{}_{}_{}", name, std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/lib.rs"), text).unwrap();
    let mut writer = IndexWriter::new(name, "0.1.0");
    for (i, (kind, def_name)) in defs.iter().enumerate() {
        let (line, start_col, end_col) = at(text, def_name);
        writer.add_definition(RawDefinition {
            def_id: i as u32 + 1, kind: *kind, visibility: 1, is_macro_expansion: false,
            name: def_name.to_string(), path: format!("{name}::{def_name}"),
            signature: None, file: "src/lib.rs".to_string(),
            start_line: line, start_col, end_line: line, end_col,
        });
    }
    writer.add_file_hash("src/lib.rs", normalized_source_hash(text.as_bytes()));
    let shard = dir.join("index.ridx");
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    Indexed { dir, shard }
}

/// Line and 1-based char columns of the first `needle` in `text`.
fn at(text: &str, needle: &str) -> (u32, u32, u32) {
    for (i, line) in text.split('\n').enumerate() {
        if let Some(byte) = line.find(needle) {
            let col = line[..byte].chars().count() as u32 + 1;
            return (i as u32 + 1, col, col + needle.chars().count() as u32);
        }
    }
    panic!("{needle:?} does not occur in {text:?}");
}

struct TestDispatcher {
    lifecycle: LifecycleManager,
    multi_root: MultiRootWorkspace,
    overlay_mgr: OverlayManager,
    cancelled: Arc<Mutex<HashSet<Value>>>,
    writer: Vec<u8>,
}

impl TestDispatcher {
    fn with_root(shard: &Path) -> Self {
        let mut d = Self {
            lifecycle: LifecycleManager::new(),
            multi_root: MultiRootWorkspace::new(),
            overlay_mgr: OverlayManager::new(),
            cancelled: Arc::new(Mutex::new(HashSet::new())),
            writer: Vec::new(),
        };
        assert!(d.multi_root.add_root(shard.to_path_buf()));
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

    /// Opens `file` with `text` and returns the `textDocument/codeLens` result for it.
    fn open_and_lenses(&mut self, file: &Path, text: &str, id: u64) -> Value {
        let open_msg = json!({
            "jsonrpc": "2.0", "method": "textDocument/didOpen",
            "params": {
                "textDocument": {
                    "uri": to_uri(file), "languageId": "rust", "version": 1, "text": text
                }
            }
        });
        assert!(self.dispatch(&open_msg));
        let req = json!({
            "jsonrpc": "2.0", "id": id, "method": "textDocument/codeLens",
            "params": { "textDocument": { "uri": to_uri(file) } }
        });
        assert!(self.dispatch(&req));
        let resp = self.parse_response();
        assert_eq!(resp["id"], id);
        resp["result"].clone()
    }
}

#[test]
fn test_code_lens_struct_definition_lens() {
    let text = "struct Point {\n    x: i32,\n    y: i32,\n}\n";
    let ix = indexed("p3_struct", text, &[(DefKind::Struct, "Point")]);
    let mut dsp = TestDispatcher::with_root(&ix.shard);
    let result = dsp.open_and_lenses(&ix.file(), text, 301);
    let lenses = result.as_array().unwrap();
    assert_eq!(lenses.len(), 1);
    assert_eq!(lenses[0]["range"]["start"]["line"], 0);
    assert_eq!(lenses[0]["range"]["start"]["character"], 7);
    assert_eq!(lenses[0]["range"]["end"]["line"], 0);
    assert_eq!(lenses[0]["range"]["end"]["character"], 12);
    assert_eq!(lenses[0]["data"]["name"], "Point");
}

#[test]
fn test_code_lens_enum_definition_lens() {
    // The index names the variants as well as the enum, so each variant has a lens.
    let text = "enum Color {\n    Red,\n    Green,\n    Blue,\n}\n";
    let ix = indexed(
        "p3_enum",
        text,
        &[
            (DefKind::Enum, "Color"),
            (DefKind::Variant, "Red"),
            (DefKind::Variant, "Green"),
            (DefKind::Variant, "Blue"),
        ],
    );
    let mut dsp = TestDispatcher::with_root(&ix.shard);
    let result = dsp.open_and_lenses(&ix.file(), text, 302);
    let lenses = result.as_array().unwrap();
    assert_eq!(lenses.len(), 4);
    assert_eq!(lenses[0]["range"]["start"]["line"], 0);
    assert_eq!(lenses[0]["range"]["start"]["character"], 5);
    assert_eq!(lenses[0]["range"]["end"]["line"], 0);
    assert_eq!(lenses[0]["range"]["end"]["character"], 10);
    assert_eq!(lenses[0]["data"]["name"], "Color");
    assert_eq!(lenses[1]["range"]["start"]["line"], 1);
    assert_eq!(lenses[1]["range"]["start"]["character"], 4);
    assert_eq!(lenses[1]["range"]["end"]["character"], 7);
    assert_eq!(lenses[1]["data"]["name"], "Red");
    assert_eq!(lenses[2]["range"]["start"]["line"], 2);
    assert_eq!(lenses[2]["data"]["name"], "Green");
    assert_eq!(lenses[3]["range"]["start"]["line"], 3);
    assert_eq!(lenses[3]["data"]["name"], "Blue");
}

#[test]
fn test_code_lens_trait_definition_lens() {
    let text = "trait Greeter {\n    fn greet(&self);\n}\n";
    let ix = indexed(
        "p3_trait",
        text,
        &[(DefKind::Trait, "Greeter"), (DefKind::AssocFn, "greet")],
    );
    let mut dsp = TestDispatcher::with_root(&ix.shard);
    let result = dsp.open_and_lenses(&ix.file(), text, 303);
    let lenses = result.as_array().unwrap();
    assert_eq!(lenses.len(), 2);
    assert_eq!(lenses[0]["data"]["name"], "Greeter");
    assert_eq!(lenses[0]["range"]["start"]["character"], 6);
    assert_eq!(lenses[1]["data"]["name"], "greet");
    assert_eq!(lenses[1]["range"]["start"]["line"], 1);
    assert_eq!(lenses[1]["range"]["start"]["character"], 7);
}

#[test]
fn test_code_lens_const_definition_lens() {
    let text = "const MAX_SIZE: usize = 1024;\n";
    let ix = indexed("p3_const", text, &[(DefKind::Const, "MAX_SIZE")]);
    let mut dsp = TestDispatcher::with_root(&ix.shard);
    let result = dsp.open_and_lenses(&ix.file(), text, 304);
    let lenses = result.as_array().unwrap();
    assert_eq!(lenses.len(), 1);
    assert_eq!(lenses[0]["range"]["start"]["line"], 0);
    assert_eq!(lenses[0]["range"]["start"]["character"], 6);
    assert_eq!(lenses[0]["range"]["end"]["line"], 0);
    assert_eq!(lenses[0]["range"]["end"]["character"], 14);
    assert_eq!(lenses[0]["data"]["name"], "MAX_SIZE");
}

#[test]
fn test_code_lens_static_definition_lens() {
    let text = "static COUNTER: u64 = 0;\n";
    let ix = indexed("p3_static", text, &[(DefKind::Static, "COUNTER")]);
    let mut dsp = TestDispatcher::with_root(&ix.shard);
    let result = dsp.open_and_lenses(&ix.file(), text, 305);
    let lenses = result.as_array().unwrap();
    assert_eq!(lenses.len(), 1);
    assert_eq!(lenses[0]["range"]["start"]["line"], 0);
    assert_eq!(lenses[0]["range"]["start"]["character"], 7);
    assert_eq!(lenses[0]["range"]["end"]["line"], 0);
    assert_eq!(lenses[0]["range"]["end"]["character"], 14);
    assert_eq!(lenses[0]["data"]["name"], "COUNTER");
}

#[test]
fn test_code_lens_type_alias_definition_lens() {
    let text = "type IntMap = std::collections::HashMap<i32, i32>;\n";
    let ix = indexed("p3_type", text, &[(DefKind::TyAlias, "IntMap")]);
    let mut dsp = TestDispatcher::with_root(&ix.shard);
    let result = dsp.open_and_lenses(&ix.file(), text, 306);
    let lenses = result.as_array().unwrap();
    assert_eq!(lenses.len(), 1);
    assert_eq!(lenses[0]["range"]["start"]["line"], 0);
    assert_eq!(lenses[0]["range"]["start"]["character"], 5);
    assert_eq!(lenses[0]["range"]["end"]["line"], 0);
    assert_eq!(lenses[0]["range"]["end"]["character"], 11);
    assert_eq!(lenses[0]["data"]["name"], "IntMap");
}
