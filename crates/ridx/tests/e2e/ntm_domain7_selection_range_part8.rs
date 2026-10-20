/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/selectionRange (Domain 7, Part 8).
//! Tests the token level of the chain: identifier boundaries, raw identifiers, lifetimes,
//! keywords, comments and literals as whole nodes, and the `fn` item start with attributes.

use ridx::lsp::dispatch::dispatch_lsp_message;
use ridx::lsp::lifecycle::LifecycleManager;
use ridx::lsp::protocol::path_to_uri;
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::OverlayManager;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

static SEL_P8_COUNTER: AtomicU64 = AtomicU64::new(800);

fn to_uri(p: &Path) -> String {
    path_to_uri(p.to_str().unwrap())
}

fn temp_file(tag: &str) -> PathBuf {
    let id = SEL_P8_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "ridx_sel_p8_{}_{}_{}.rs",
        tag,
        std::process::id(),
        id
    ))
}

struct TestDispatcher {
    lifecycle: LifecycleManager,
    multi_root: MultiRootWorkspace,
    overlay_mgr: OverlayManager,
    cancelled: Arc<Mutex<HashSet<Value>>>,
    writer: Vec<u8>,
}

impl TestDispatcher {
    fn new() -> Self {
        let mut d = Self {
            lifecycle: LifecycleManager::new(),
            multi_root: MultiRootWorkspace::new(),
            overlay_mgr: OverlayManager::new(),
            cancelled: Arc::new(Mutex::new(HashSet::new())),
            writer: Vec::new(),
        };
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
    id: u64,
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

const TOKENS_TEXT: &str = "fn tokens() {\n    let value = other + 1;\n}\n";

#[test]
fn test_selection_range_token_start_middle_and_end_boundary_share_one_range() {
    let file = temp_file("tokbounds");
    let mut dsp = TestDispatcher::new();
    open_doc(&mut dsp, &file, TOKENS_TEXT);

    // `value` spans columns 8..13: its start, middle and end boundary all select it.
    let result = selection_ranges(
        &mut dsp,
        801,
        &file,
        &[(1, 8), (1, 10), (1, 13), (1, 16), (1, 21)],
    );
    let value = chain(&[(1, 8, 1, 13), (0, 12, 2, 1), (0, 0, 2, 1), (0, 0, 3, 0)]);
    let other = chain(&[(1, 16, 1, 21), (0, 12, 2, 1), (0, 0, 2, 1), (0, 0, 3, 0)]);
    assert_eq!(result, json!([value, value, value, other, other]));
}

#[test]
fn test_selection_range_whitespace_punctuation_and_number_have_no_token() {
    let file = temp_file("notoken");
    let mut dsp = TestDispatcher::new();
    open_doc(&mut dsp, &file, TOKENS_TEXT);

    // Indentation, `=`, `+`, the digit `1` and `;`: the chain starts at the fn body.
    let result = selection_ranges(
        &mut dsp,
        802,
        &file,
        &[(1, 2), (1, 14), (1, 22), (1, 24), (1, 25)],
    );
    let body_only = chain(&[(0, 12, 2, 1), (0, 0, 2, 1), (0, 0, 3, 0)]);
    assert_eq!(
        result,
        json!([body_only, body_only, body_only, body_only, body_only])
    );
}

#[test]
fn test_selection_range_raw_identifier_and_lifetime_tokens() {
    let file = temp_file("rawlife");
    let mut dsp = TestDispatcher::new();
    open_doc(
        &mut dsp,
        &file,
        "fn demo<'a>(r#type: &'a str) -> &'a str {\n    r#type\n}\n",
    );

    // `'a` at 8..10 (on `'`, on `a`, at its end before `>`); `r#type` at 12..18 (on `r`, `#`,
    // inside). Signature positions sit in the fn item but before its body.
    let result = selection_ranges(
        &mut dsp,
        803,
        &file,
        &[(0, 8), (0, 9), (0, 10), (0, 12), (0, 13), (0, 15), (1, 6)],
    );
    let lifetime = chain(&[(0, 8, 0, 10), (0, 0, 2, 1), (0, 0, 3, 0)]);
    let raw_param = chain(&[(0, 12, 0, 18), (0, 0, 2, 1), (0, 0, 3, 0)]);
    let raw_in_body = chain(&[(1, 4, 1, 10), (0, 40, 2, 1), (0, 0, 2, 1), (0, 0, 3, 0)]);
    assert_eq!(
        result,
        json!([
            lifetime,
            lifetime,
            lifetime,
            raw_param,
            raw_param,
            raw_param,
            raw_in_body
        ])
    );
}

#[test]
fn test_selection_range_keyword_tokens_fn_and_let() {
    let file = temp_file("keywords");
    let mut dsp = TestDispatcher::new();
    open_doc(&mut dsp, &file, "fn kw() {\n    let x = 1;\n}\n");

    let result = selection_ranges(&mut dsp, 804, &file, &[(0, 0), (0, 1), (0, 3), (1, 4)]);
    let kw_fn = chain(&[(0, 0, 0, 2), (0, 0, 2, 1), (0, 0, 3, 0)]);
    let name = chain(&[(0, 3, 0, 5), (0, 0, 2, 1), (0, 0, 3, 0)]);
    let kw_let = chain(&[(1, 4, 1, 7), (0, 8, 2, 1), (0, 0, 2, 1), (0, 0, 3, 0)]);
    assert_eq!(result, json!([kw_fn, kw_fn, name, kw_let]));
}

#[test]
fn test_selection_range_comment_and_literal_select_whole_node() {
    let file = temp_file("literals");
    let mut dsp = TestDispatcher::new();
    open_doc(
        &mut dsp,
        &file,
        concat!(
            "fn lits() {\n",
            "    // note: value\n",
            "    let s = \"hello world\";\n",
            "    let c = 'x';\n",
            "    /* multi\n",
            "       line */\n",
            "    let r = r#\"raw { brace\"#;\n",
            "}\n",
        ),
    );

    // Inside a comment, string, char or raw string the whole node is selected; the `{` in the
    // raw string opens no block; on the opening quote itself there is no token yet.
    let result = selection_ranges(
        &mut dsp,
        805,
        &file,
        &[
            (1, 10),
            (2, 15),
            (2, 25),
            (3, 13),
            (5, 9),
            (6, 17),
            (6, 22),
            (2, 12),
        ],
    );
    let rest = [(0, 10, 7, 1), (0, 0, 7, 1), (0, 0, 8, 0)];
    let with = |first: (u32, u32, u32, u32)| chain(&[&[first][..], &rest[..]].concat());
    assert_eq!(
        result,
        json!([
            with((1, 4, 1, 18)),
            with((2, 12, 2, 25)),
            with((2, 12, 2, 25)),
            with((3, 12, 3, 15)),
            with((4, 4, 5, 14)),
            with((6, 12, 6, 28)),
            with((6, 12, 6, 28)),
            chain(&rest),
        ])
    );
}

#[test]
fn test_selection_range_fn_item_includes_doc_comment_and_attributes() {
    let file = temp_file("fnattrs");
    let mut dsp = TestDispatcher::new();
    open_doc(
        &mut dsp,
        &file,
        concat!(
            "//! Module docs.\n",
            "\n",
            "/// Adds one.\n",
            "#[inline]\n",
            "#[must_use] // keep\n",
            "pub fn add_one(x: i32) -> i32 {\n",
            "    x + 1\n",
            "}\n",
        ),
    );

    // The item starts at `/// Adds one.`; the module doc comment above the blank line is not
    // part of it. Attribute words and doc comments are nodes inside the item.
    let result = selection_ranges(&mut dsp, 806, &file, &[(6, 4), (3, 3), (2, 5), (0, 5)]);
    assert_eq!(
        result,
        json!([
            chain(&[(6, 4, 6, 5), (5, 30, 7, 1), (2, 0, 7, 1), (0, 0, 8, 0)]),
            chain(&[(3, 2, 3, 8), (2, 0, 7, 1), (0, 0, 8, 0)]),
            chain(&[(2, 0, 2, 13), (2, 0, 7, 1), (0, 0, 8, 0)]),
            chain(&[(0, 0, 0, 16), (0, 0, 8, 0)]),
        ])
    );
}

#[test]
fn test_selection_range_trait_method_without_body_and_nested_fn_item() {
    let file = temp_file("traitnested");
    let mut dsp = TestDispatcher::new();
    open_doc(
        &mut dsp,
        &file,
        concat!(
            "trait Shape {\n",
            "    fn area(&self) -> f64;\n",
            "}\n",
            "fn outer() {\n",
            "    fn inner() {\n",
            "        1\n",
            "    }\n",
            "}\n",
        ),
    );

    // A bodiless trait method is an item ending at `;` with no body node; a nested fn item
    // sits inside the outer fn body.
    let result = selection_ranges(&mut dsp, 807, &file, &[(1, 14), (5, 8)]);
    assert_eq!(
        result,
        json!([
            chain(&[(1, 13, 1, 17), (1, 4, 1, 26), (0, 12, 2, 1), (0, 0, 8, 0)]),
            chain(&[
                (4, 15, 6, 5),
                (4, 4, 6, 5),
                (3, 11, 7, 1),
                (3, 0, 7, 1),
                (0, 0, 8, 0)
            ]),
        ])
    );
}
