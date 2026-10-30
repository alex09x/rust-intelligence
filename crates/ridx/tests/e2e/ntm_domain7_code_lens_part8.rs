/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Code lenses of open buffers come from the index, not from the syntax scan of the buffer: an
//! unmodified buffer answers like the closed file, edits move or remove lenses line by line, and a
//! file the index does not know has none.

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

static LENS_P8_COUNTER: AtomicU64 = AtomicU64::new(1);

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
    let id = LENS_P8_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_lens_{}_{}_{}", name, std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/lib.rs"), text).unwrap();
    let mut writer = IndexWriter::new(name, "0.1.0");
    for (i, def_name) in fns.iter().enumerate() {
        let (line, start_col, end_col) = at(text, def_name);
        writer.add_definition(RawDefinition {
            def_id: i as u32 + 1,
            kind: DefKind::Fn,
            visibility: 1,
            is_macro_expansion: false,
            name: def_name.to_string(),
            path: format!("{name}::{def_name}"),
            signature: None,
            file: "src/lib.rs".to_string(),
            start_line: line,
            start_col,
            end_line: line,
            end_col,
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
        dispatch_lsp_message(msg.clone(), &mut self.writer, &mut self.lifecycle, &mut self.multi_root, &mut self.overlay_mgr, &self.cancelled, None).unwrap()
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
fn test_code_lens_open_buffer_answers_from_the_index_not_the_syntax_scan() {
    // The buffer is the indexed text. Its parameter `x` is a symbol of the syntax scan of the
    // buffer but not a definition of the index, so it gets no lens: the open file answers exactly
    // like the closed file does.
    let text = "fn test_fn(x: u32) {}\n";
    let ix = indexed("p8_index", text, &["test_fn"]);
    let mut dsp = TestDispatcher::with_root(&ix.shard);
    dsp.open(&ix.file(), text);
    let result = dsp.lenses(&ix.file(), 801);
    let lenses = result.as_array().unwrap();
    assert_eq!(lenses.len(), 1);
    assert_eq!(lenses[0]["range"]["start"]["line"], 0);
    assert_eq!(lenses[0]["range"]["start"]["character"], 3);
    assert_eq!(lenses[0]["range"]["end"]["character"], 10);
    assert_eq!(lenses[0]["data"]["name"], "test_fn");
    assert_eq!(lenses[0]["data"]["line"], 1);
    assert_eq!(lenses[0]["data"]["character"], 4);
}

#[test]
fn test_code_lens_open_buffer_edit_moves_and_removes_lenses() {
    // The buffer is mapped onto the indexed text line by line (common prefix and suffix): a header
    // inserted at the top moves both lenses down; `other_fn` renamed in the buffer has no line in
    // the indexed text any more and loses its lens while `test_fn` keeps its own.
    let text = "fn test_fn() {}\n\nfn other_fn() {}\n";
    let ix = indexed("p8_edit", text, &["test_fn", "other_fn"]);
    let mut dsp = TestDispatcher::with_root(&ix.shard);
    dsp.open(&ix.file(), text);

    dsp.change(&ix.file(), 2, "// header\nfn test_fn() {}\n\nfn other_fn() {}\n");
    let shifted = dsp.lenses(&ix.file(), 802);
    let lenses = shifted.as_array().unwrap();
    assert_eq!(lenses.len(), 2);
    assert_eq!(lenses[0]["range"]["start"]["line"], 1);
    assert_eq!(lenses[0]["range"]["start"]["character"], 3);
    assert_eq!(lenses[0]["data"]["name"], "test_fn");
    assert_eq!(lenses[0]["data"]["line"], 2);
    assert_eq!(lenses[0]["data"]["character"], 4);
    assert_eq!(lenses[1]["range"]["start"]["line"], 3);
    assert_eq!(lenses[1]["data"]["name"], "other_fn");
    assert_eq!(lenses[1]["data"]["line"], 4);

    dsp.change(&ix.file(), 3, "fn test_fn() {}\n\nfn other_fn2() {}\n");
    let renamed = dsp.lenses(&ix.file(), 803);
    let lenses = renamed.as_array().unwrap();
    assert_eq!(lenses.len(), 1);
    assert_eq!(lenses[0]["range"]["start"]["line"], 0);
    assert_eq!(lenses[0]["data"]["name"], "test_fn");
    assert_eq!(lenses[0]["data"]["line"], 1);
}

#[test]
fn test_code_lens_open_file_without_index_data_is_empty() {
    // The index has no definitions for `src/other.rs`: an open buffer of it gets no lenses, like
    // the closed file.
    let ix = indexed("p8_unknown", "fn test_fn() {}\n", &["test_fn"]);
    let other = ix.dir.join("src/other.rs");
    let mut dsp = TestDispatcher::with_root(&ix.shard);
    dsp.open(&other, "fn main() {}\n");
    assert_eq!(dsp.lenses(&other, 804), json!([]));
}
