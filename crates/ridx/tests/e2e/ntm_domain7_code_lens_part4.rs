/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Code lenses of open buffers: ordering, line shifts after didChange, symbol deletion, CRLF
//! lines, Unicode and multi-document isolation. Each buffer holds the text its shard indexed.

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

static LENS_P4_COUNTER: AtomicU64 = AtomicU64::new(1);

fn to_uri(p: &Path) -> String {
    path_to_uri(p.to_str().unwrap())
}

/// A crate whose `src/lib.rs` holds `text`, indexed into `shard` with a function per name at its
/// first occurrence in `text` and with the hash of `text`: a buffer opened with `text` is the text
/// the index describes.
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

fn indexed(name: &str, text: &str, fns: &[&str]) -> Indexed {
    let id = LENS_P4_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_lens_{}_{}_{}", name, std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/lib.rs"), text).unwrap();
    let mut writer = IndexWriter::new(name, "0.1.0");
    for (i, def_name) in fns.iter().enumerate() {
        let (line, start_col, end_col) = at(text, def_name);
        writer.add_definition(RawDefinition {
            def_id: i as u32 + 1, kind: DefKind::Fn, visibility: 1, is_macro_expansion: false,
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
    fn with_roots(shards: &[&Path]) -> Self {
        let mut d = Self {
            lifecycle: LifecycleManager::new(),
            multi_root: MultiRootWorkspace::new(),
            overlay_mgr: OverlayManager::new(),
            cancelled: Arc::new(Mutex::new(HashSet::new())),
            writer: Vec::new(),
        };
        for shard in shards {
            assert!(d.multi_root.add_root(shard.to_path_buf()));
        }
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

    fn open(&mut self, file: &Path, text: &str) {
        let open_msg = json!({
            "jsonrpc": "2.0", "method": "textDocument/didOpen",
            "params": {
                "textDocument": {
                    "uri": to_uri(file), "languageId": "rust", "version": 1, "text": text
                }
            }
        });
        assert!(self.dispatch(&open_msg));
    }

    fn change(&mut self, file: &Path, version: i64, text: &str) {
        let change_msg = json!({
            "jsonrpc": "2.0", "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": to_uri(file), "version": version },
                "contentChanges": [{ "text": text }]
            }
        });
        assert!(self.dispatch(&change_msg));
    }

    fn lenses(&mut self, file: &Path, id: u64) -> Value {
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
fn test_code_lens_multiple_functions_order_preserved() {
    let text = "fn alpha() {}\nfn beta() {}\nfn gamma() {}\n";
    let ix = indexed("p4_order", text, &["alpha", "beta", "gamma"]);
    let mut dsp = TestDispatcher::with_roots(&[&ix.shard]);
    dsp.open(&ix.file(), text);
    let result = dsp.lenses(&ix.file(), 401);
    let lenses = result.as_array().unwrap();
    assert_eq!(lenses.len(), 3);
    assert_eq!(lenses[0]["range"]["start"]["line"], 0);
    assert_eq!(lenses[0]["data"]["name"], "alpha");
    assert_eq!(lenses[1]["range"]["start"]["line"], 1);
    assert_eq!(lenses[1]["data"]["name"], "beta");
    assert_eq!(lenses[2]["range"]["start"]["line"], 2);
    assert_eq!(lenses[2]["data"]["name"], "gamma");
}

#[test]
fn test_code_lens_did_change_shifts_lenses_downward() {
    let text = "fn compute() {}\n";
    let ix = indexed("p4_shift", text, &["compute"]);
    let mut dsp = TestDispatcher::with_roots(&[&ix.shard]);
    dsp.open(&ix.file(), text);
    dsp.change(
        &ix.file(),
        2,
        "// Header line 1\n// Header line 2\n// Header line 3\nfn compute() {}\n",
    );
    let result = dsp.lenses(&ix.file(), 402);
    let lenses = result.as_array().unwrap();
    assert_eq!(lenses.len(), 1);
    assert_eq!(lenses[0]["range"]["start"]["line"], 3);
    assert_eq!(lenses[0]["range"]["start"]["character"], 3);
    assert_eq!(lenses[0]["range"]["end"]["character"], 10);
    assert_eq!(lenses[0]["data"]["name"], "compute");
    assert_eq!(lenses[0]["data"]["line"], 4);
    assert_eq!(lenses[0]["data"]["character"], 4);
}

#[test]
fn test_code_lens_did_change_deletes_symbol_removes_lens() {
    let text = "fn to_delete() {}\n";
    let ix = indexed("p4_delete", text, &["to_delete"]);
    let mut dsp = TestDispatcher::with_roots(&[&ix.shard]);
    dsp.open(&ix.file(), text);
    dsp.change(&ix.file(), 2, "// replaced with comment\n");
    let result = dsp.lenses(&ix.file(), 403);
    assert_eq!(result.as_array().unwrap().len(), 0);
}

#[test]
fn test_code_lens_crlf_line_offsets_matching() {
    let text = "// Header\r\nfn crlf_func() {}\r\n";
    let ix = indexed("p4_crlf", text, &["crlf_func"]);
    let mut dsp = TestDispatcher::with_roots(&[&ix.shard]);
    dsp.open(&ix.file(), text);
    let result = dsp.lenses(&ix.file(), 404);
    let lenses = result.as_array().unwrap();
    assert_eq!(lenses.len(), 1);
    assert_eq!(lenses[0]["range"]["start"]["line"], 1);
    assert_eq!(lenses[0]["range"]["start"]["character"], 3);
    assert_eq!(lenses[0]["range"]["end"]["line"], 1);
    assert_eq!(lenses[0]["range"]["end"]["character"], 12);
    assert_eq!(lenses[0]["data"]["name"], "crlf_func");
}

#[test]
fn test_code_lens_unicode_identifiers_exact_offsets() {
    let text = "// Σχολιασμός!\nfn calculate() {}\n";
    let ix = indexed("p4_uni", text, &["calculate"]);
    let mut dsp = TestDispatcher::with_roots(&[&ix.shard]);
    dsp.open(&ix.file(), text);
    let result = dsp.lenses(&ix.file(), 405);
    let lenses = result.as_array().unwrap();
    assert_eq!(lenses.len(), 1);
    assert_eq!(lenses[0]["range"]["start"]["line"], 1);
    assert_eq!(lenses[0]["range"]["start"]["character"], 3);
    assert_eq!(lenses[0]["range"]["end"]["line"], 1);
    assert_eq!(lenses[0]["range"]["end"]["character"], 12);
    assert_eq!(lenses[0]["data"]["name"], "calculate");
}

#[test]
fn test_code_lens_two_documents_independent_lenses() {
    let text_a = "fn single_fn() {}\n";
    let text_b = "fn first_fn() {}\nfn second_fn() {}\n";
    let ix_a = indexed("p4_doca", text_a, &["single_fn"]);
    let ix_b = indexed("p4_docb", text_b, &["first_fn", "second_fn"]);
    let mut dsp = TestDispatcher::with_roots(&[&ix_a.shard, &ix_b.shard]);
    dsp.open(&ix_a.file(), text_a);
    dsp.open(&ix_b.file(), text_b);

    let result_a = dsp.lenses(&ix_a.file(), 406);
    let lenses_a = result_a.as_array().unwrap();
    assert_eq!(lenses_a.len(), 1);
    assert_eq!(lenses_a[0]["data"]["name"], "single_fn");

    let result_b = dsp.lenses(&ix_b.file(), 407);
    let lenses_b = result_b.as_array().unwrap();
    assert_eq!(lenses_b.len(), 2);
    assert_eq!(lenses_b[0]["data"]["name"], "first_fn");
    assert_eq!(lenses_b[1]["data"]["name"], "second_fn");
}
