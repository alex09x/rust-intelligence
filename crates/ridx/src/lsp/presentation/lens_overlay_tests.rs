/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! `textDocument/codeLens` on a file open in the editor: the same lenses the index gives the
//! closed file, moved onto the live buffer line by line. One shard with one `src/lib.rs` that
//! records the hash of the indexed text, opened like a single-root server; documents are opened
//! the way `didOpen` opens them, with the hash the index recorded.

use super::handle_code_lens;
use crate::lsp::protocol::{path_to_uri, PositionConverter};
use crate::overlay::OverlayManager;
use ridx_format::{normalized_source_hash, DefKind, IndexWriter, RawDefinition, WorkspaceIndex};
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

static DIR_COUNTER: AtomicU64 = AtomicU64::new(0);
static DEF_ID: AtomicU32 = AtomicU32::new(1);

struct Shard {
    dir: PathBuf,
    ws: WorkspaceIndex,
}

impl Drop for Shard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

impl Shard {
    fn file(&self) -> String {
        self.dir.join("src/lib.rs").to_str().unwrap().to_string()
    }
}

/// A crate whose `src/lib.rs` holds `text`, indexed with `defs` and the hash of `text`.
fn shard(text: &str, defs: Vec<RawDefinition>) -> Shard {
    let id = DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_lens_overlay_{}_{}", std::process::id(), id));
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(dir.join("src/lib.rs"), text).unwrap();
    let mut writer = IndexWriter::new("lens_crate", "0.1.0");
    for d in defs {
        writer.add_definition(d);
    }
    writer.add_file_hash("src/lib.rs", normalized_source_hash(text.as_bytes()));
    let shard = dir.join("index.ridx");
    writer.write_to(&mut fs::File::create(&shard).unwrap()).unwrap();
    let ws = WorkspaceIndex::open(&shard).expect("open the shard");
    Shard { dir, ws }
}

/// A `Fn` named `name` on `line`, spanning `[start_col, end_col)` in 1-based char columns.
fn func(name: &str, (line, start_col, end_col): (u32, u32, u32)) -> RawDefinition {
    RawDefinition {
        def_id: DEF_ID.fetch_add(1, Ordering::Relaxed),
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: name.to_string(),
        path: format!("lens_crate::{name}"),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: line,
        start_col,
        end_line: line,
        end_col,
    }
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

/// Opens `file` in `mgr` as `didOpen` does: with the hash the index recorded for it.
fn open(mgr: &mut OverlayManager, sh: &Shard, file: &str, text: &str) {
    let uri = path_to_uri(file);
    let hash = sh.ws.source_hash_for_file(file);
    mgr.open_with_source_hash(&uri, file, 1, text.to_string(), hash);
}

fn lenses(ws: Option<&WorkspaceIndex>, mgr: &OverlayManager, file: &str) -> Value {
    let uri = path_to_uri(file);
    let msg = json!({ "params": { "textDocument": { "uri": &uri } } });
    let mut conv = PositionConverter::new();
    handle_code_lens(&msg, ws, mgr, &mut conv)
}

fn closed(sh: &Shard) -> Value {
    lenses(Some(&sh.ws), &OverlayManager::new(), &sh.file())
}

/// `(line, start, end, data.line, data.character, name)` of every lens, LSP coordinates for the
/// range and the 1-based `data` coordinates `codeLens/resolve` receives.
fn rows(result: &Value) -> Vec<(u64, u64, u64, u64, u64, String)> {
    result
        .as_array()
        .unwrap()
        .iter()
        .map(|l| {
            (
                l["range"]["start"]["line"].as_u64().unwrap(),
                l["range"]["start"]["character"].as_u64().unwrap(),
                l["range"]["end"]["character"].as_u64().unwrap(),
                l["data"]["line"].as_u64().unwrap(),
                l["data"]["character"].as_u64().unwrap(),
                l["data"]["name"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

const TEXT: &str = "pub fn alpha(x: u32) -> u32 {\n    let y = x;\n    y\n}\n\npub fn beta() {}\n";

fn two_fns() -> Shard {
    shard(TEXT, vec![func("alpha", at(TEXT, "alpha")), func("beta", at(TEXT, "beta"))])
}

#[test]
fn test_code_lens_open_unmodified_buffer_answers_like_the_closed_file() {
    let sh = two_fns();
    let mut mgr = OverlayManager::new();
    open(&mut mgr, &sh, &sh.file(), TEXT);
    assert!(!mgr.get(&sh.file()).unwrap().is_dirty());
    let open_answer = lenses(Some(&sh.ws), &mgr, &sh.file());
    // The buffer also declares the parameter `x` and the local `y`; neither is a lens.
    assert_eq!(rows(&open_answer), vec![(0, 7, 12, 1, 8, "alpha".to_string()), (5, 7, 11, 6, 8, "beta".to_string())]);
    assert_eq!(open_answer, closed(&sh));
}

/// The overlay maps lines by the common prefix and suffix of the two texts (`LineMap`): lines
/// inserted above an item shift its lens, an edited line has no counterpart and loses its lens.
#[test]
fn test_code_lens_open_dirty_buffer_moves_lenses_and_drops_the_edited_line() {
    let sh = two_fns();
    let mut mgr = OverlayManager::new();
    open(&mut mgr, &sh, &sh.file(), TEXT);
    // Two lines inserted at the top: both lenses move down by two lines, range and `data` alike.
    assert!(mgr.update(&sh.file(), 2, format!("// one\n// two\n{TEXT}")));
    assert!(mgr.get(&sh.file()).unwrap().is_dirty());
    assert_eq!(rows(&lenses(Some(&sh.ws), &mgr, &sh.file())), vec![(2, 7, 12, 3, 8, "alpha".to_string()), (7, 7, 11, 8, 8, "beta".to_string())]);
    // `beta` renamed in the buffer: its line has no counterpart in the indexed text, so it has
    // no lens; `alpha` keeps its own.
    assert!(mgr.update(&sh.file(), 3, TEXT.replace("fn beta()", "fn beta2()")));
    assert_eq!(rows(&lenses(Some(&sh.ws), &mgr, &sh.file())), vec![(0, 7, 12, 1, 8, "alpha".to_string())]);
}

#[test]
fn test_code_lens_open_buffer_without_compiled_baseline_falls_back_to_the_disk_text() {
    let indexed = "pub fn alpha() {}\npub fn beta() {}\n";
    let sh = shard(indexed, vec![func("alpha", at(indexed, "alpha")), func("beta", at(indexed, "beta"))]);
    // The file changed on disk after indexing and the buffer differs from both: no compiled
    // baseline. The disk text is the baseline, as for a closed file: `beta` no longer spells
    // its name there, `alpha` does and follows the inserted line.
    let on_disk = "pub fn alpha() {}\npub fn gamma() {}\n";
    fs::write(sh.dir.join("src/lib.rs"), on_disk).unwrap();
    let mut mgr = OverlayManager::new();
    open(&mut mgr, &sh, &sh.file(), &format!("// top\n{on_disk}"));
    let overlay = mgr.get(&sh.file()).unwrap();
    assert_eq!(overlay.compiled_text, None);
    assert_eq!(overlay.disk_text.as_deref(), Some(on_disk));
    assert_eq!(rows(&lenses(Some(&sh.ws), &mgr, &sh.file())), vec![(1, 7, 12, 2, 8, "alpha".to_string())]);
    assert_eq!(rows(&closed(&sh)), vec![(0, 7, 12, 1, 8, "alpha".to_string())]);
}

#[test]
fn test_code_lens_open_file_the_index_does_not_know_has_no_lenses() {
    let sh = two_fns();
    let other = sh.dir.join("src/other.rs").to_str().unwrap().to_string();
    let mut mgr = OverlayManager::new();
    open(&mut mgr, &sh, &other, "fn main() {}\n");
    assert_eq!(lenses(Some(&sh.ws), &mgr, &other), json!([]));
    assert_eq!(lenses(None, &mgr, &other), json!([]));
}

#[test]
fn test_code_lens_open_buffer_keeps_utf16_columns_on_non_ascii_lines_through_a_shift() {
    let text = "/* 🦀 */ pub fn alpha() {}\n";
    let sh = shard(text, vec![func("alpha", at(text, "alpha"))]);
    let mut mgr = OverlayManager::new();
    open(&mut mgr, &sh, &sh.file(), text);
    // 15 chars precede `alpha`, the crab takes two UTF-16 units: columns 16..21 in both answers.
    let unmodified = rows(&lenses(Some(&sh.ws), &mgr, &sh.file()));
    assert_eq!(unmodified, vec![(0, 16, 21, 1, 16, "alpha".to_string())]);
    assert_eq!(unmodified, rows(&closed(&sh)));
    assert!(mgr.update(&sh.file(), 2, format!("// top\n{text}")));
    assert_eq!(rows(&lenses(Some(&sh.ws), &mgr, &sh.file())), vec![(1, 16, 21, 2, 16, "alpha".to_string())]);
}

#[test]
fn test_code_lens_open_buffer_with_crlf_line_endings() {
    let text = "// Header\r\nfn crlf_func() {}\r\n";
    let sh = shard(text, vec![func("crlf_func", at(text, "crlf_func"))]);
    let mut mgr = OverlayManager::new();
    open(&mut mgr, &sh, &sh.file(), text);
    let unmodified = rows(&lenses(Some(&sh.ws), &mgr, &sh.file()));
    assert_eq!(unmodified, vec![(1, 3, 12, 2, 4, "crlf_func".to_string())]);
    assert_eq!(unmodified, rows(&closed(&sh)));
    assert!(mgr.update(&sh.file(), 2, format!("// x\r\n{text}")));
    assert_eq!(rows(&lenses(Some(&sh.ws), &mgr, &sh.file())), vec![(2, 3, 12, 3, 4, "crlf_func".to_string())]);
}
