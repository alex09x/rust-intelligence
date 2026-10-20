/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use super::handle_selection_range;
use crate::lsp::protocol::path_to_uri;
use crate::overlay::OverlayManager;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static SEL_TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

fn chain_from_ranges(ranges: &[(u32, u32, u32, u32)]) -> Value {
    let mut current = Value::Null;
    for &(sl, sc, el, ec) in ranges.iter().rev() {
        current = json!({
            "range": {
                "start": { "line": sl, "character": sc },
                "end": { "line": el, "character": ec }
            },
            "parent": current
        });
    }
    current
}

struct TempTestDir {
    path: PathBuf,
}

impl TempTestDir {
    fn new(prefix: &str) -> Self {
        let id = SEL_TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "ridx_sel_closed_{}_{}_{}",
            prefix,
            std::process::id(),
            id
        ));
        fs::create_dir_all(path.join("src")).unwrap();
        Self { path }
    }
}

impl Drop for TempTestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn test_selection_range_closed_file_lifecycle_open_change_close() {
    let dir = TempTestDir::new("lifecycle");
    let file_path = dir.path.join("src/lib.rs");
    let file_str = file_path.to_str().unwrap();
    let uri = path_to_uri(file_str);

    let disk_text = concat!(
        "pub mod outer {\n",
        "    pub fn compute(x: i32) -> i32 {\n",
        "        if x > 0 {\n",
        "            let y = x + 1;\n",
        "            y\n",
        "        } else {\n",
        "            0\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    fs::write(&file_path, disk_text).unwrap();

    let mut overlay_mgr = OverlayManager::new();
    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "positions": [{ "line": 3, "character": 16 }]
        }
    });

    // 1. Closed file on disk returns exact nested SelectionRange chain.
    let closed_res = handle_selection_range(&req, &overlay_mgr);
    let expected_disk_chain = json!([chain_from_ranges(&[
        (3, 16, 3, 17),
        (2, 17, 5, 9),
        (1, 34, 8, 5),
        (1, 4, 8, 5),
        (0, 14, 9, 1),
        (0, 0, 10, 0),
    ])]);
    assert_eq!(closed_res, expected_disk_chain);

    // 2. Opening the document with identical text produces the exact same response.
    overlay_mgr.open_document(&uri, file_str, 1, disk_text.to_string());
    let open_res = handle_selection_range(&req, &overlay_mgr);
    assert_eq!(open_res, closed_res);

    // 3. Updating the live buffer via didChange overrides the on-disk file.
    let live_text = "fn compute() { let z = 1; }\n";
    overlay_mgr.update_document(file_str, 2, live_text.to_string());
    let live_req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "positions": [{ "line": 0, "character": 19 }]
        }
    });
    let live_res = handle_selection_range(&live_req, &overlay_mgr);
    assert_eq!(
        live_res,
        json!([chain_from_ranges(&[
            (0, 19, 0, 20),
            (0, 13, 0, 27),
            (0, 0, 0, 27),
            (0, 0, 1, 0),
        ])])
    );

    // 4. Closing the buffer via didClose falls back to the on-disk file again.
    overlay_mgr.close_document(file_str);
    let after_close_res = handle_selection_range(&req, &overlay_mgr);
    assert_eq!(after_close_res, expected_disk_chain);
}

#[test]
fn test_selection_range_closed_file_non_utf8_and_unreadable() {
    let dir = TempTestDir::new("invalid_io");
    let invalid_utf8_path = dir.path.join("src/invalid_utf8.rs");
    fs::write(&invalid_utf8_path, [0xff, 0xfe, 0x00, 0x80]).unwrap();

    let missing_path = dir.path.join("src/does_not_exist.rs");
    let dir_as_file_path = dir.path.join("src");

    let overlay_mgr = OverlayManager::new();

    for path in [&invalid_utf8_path, &missing_path, &dir_as_file_path] {
        let uri = path_to_uri(path.to_str().unwrap());
        let req = json!({
            "params": {
                "textDocument": { "uri": uri },
                "positions": [{ "line": 0, "character": 0 }]
            }
        });
        let res = handle_selection_range(&req, &overlay_mgr);
        assert_eq!(res, json!([]), "expected empty array for {:?}", path);
    }
}

#[test]
fn test_selection_range_closed_file_past_eol_and_past_eof() {
    let dir = TempTestDir::new("bounds");
    let file_path = dir.path.join("src/bounds.rs");
    let file_str = file_path.to_str().unwrap();
    let uri = path_to_uri(file_str);

    let text = "pub fn a() {\n    let x = 1;\n}\n";
    fs::write(&file_path, text).unwrap();

    let mut overlay_mgr = OverlayManager::new();
    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "positions": [
                { "line": 1, "character": 999 },
                { "line": 999, "character": 999 }
            ]
        }
    });

    let closed_res = handle_selection_range(&req, &overlay_mgr);
    let expected = json!([
        chain_from_ranges(&[(0, 11, 2, 1), (0, 0, 2, 1), (0, 0, 3, 0)]),
        chain_from_ranges(&[(0, 0, 3, 0)]),
    ]);
    assert_eq!(closed_res, expected);

    overlay_mgr.open_document(&uri, file_str, 1, text.to_string());
    let open_res = handle_selection_range(&req, &overlay_mgr);
    assert_eq!(closed_res, open_res);
}

#[test]
fn test_selection_range_closed_file_crlf_and_non_ascii_utf16() {
    let dir = TempTestDir::new("crlf_utf16");
    let file_path = dir.path.join("src/utf16.rs");
    let file_str = file_path.to_str().unwrap();
    let uri = path_to_uri(file_str);

    // Line 0: "pub fn γειά() {\r\n" -> `pub` at 0, `{` at UTF-16 col 14
    // Line 1: "    let s = \"🎉\"; { let inner = 1; }\r\n"
    //   `inner` spans UTF-16 cols 24..29; `{ let inner = 1; }` spans UTF-16 cols 18..36
    let text = "pub fn γειά() {\r\n    let s = \"🎉\"; { let inner = 1; }\r\n}\r\n";
    fs::write(&file_path, text).unwrap();

    let mut overlay_mgr = OverlayManager::new();
    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "positions": [{ "line": 1, "character": 25 }]
        }
    });

    let closed_res = handle_selection_range(&req, &overlay_mgr);
    let expected = json!([chain_from_ranges(&[
        (1, 24, 1, 29),
        (1, 18, 1, 36),
        (0, 14, 2, 1),
        (0, 0, 2, 1),
        (0, 0, 3, 0),
    ])]);
    assert_eq!(closed_res, expected);

    overlay_mgr.open_document(&uri, file_str, 1, text.to_string());
    let open_res = handle_selection_range(&req, &overlay_mgr);
    assert_eq!(closed_res, open_res);
}

#[test]
fn test_selection_range_closed_file_lifetimes_and_raw_strings() {
    let dir = TempTestDir::new("lifetimes");
    let file_path = dir.path.join("src/lifetimes.rs");
    let file_str = file_path.to_str().unwrap();
    let uri = path_to_uri(file_str);

    let text = concat!(
        "pub struct Holder<'a> { pub s: &'a str }\n",
        "impl<'a> Holder<'a> {\n",
        "    pub fn get(&'a self) -> &'a str {\n",
        "        let raw = r#\"{ unclosed_brace_in_raw_string }\"#;\n",
        "        let _ = raw;\n",
        "        self.s\n",
        "    }\n",
        "}\n",
    );
    fs::write(&file_path, text).unwrap();

    let overlay_mgr = OverlayManager::new();
    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "positions": [{ "line": 5, "character": 8 }]
        }
    });

    let res = handle_selection_range(&req, &overlay_mgr);
    assert_eq!(
        res,
        json!([chain_from_ranges(&[
            (5, 8, 5, 12),
            (2, 36, 6, 5),
            (2, 4, 6, 5),
            (1, 20, 7, 1),
            (0, 0, 8, 0),
        ])])
    );
}

#[test]
fn test_selection_range_token_boundaries_raw_ident_lifetime_and_non_token() {
    let dir = TempTestDir::new("tokens");
    let file_path = dir.path.join("src/tokens.rs");
    let file_str = file_path.to_str().unwrap();
    let uri = path_to_uri(file_str);

    let text = concat!(
        "pub fn demo<'a>(r#type: &'a str) -> usize {\n",
        "    let value = r#type.len() + 1;\n",
        "    value\n",
        "}\n",
    );
    fs::write(&file_path, text).unwrap();

    let overlay_mgr = OverlayManager::new();
    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "positions": [
                { "line": 1, "character": 8 },  // start boundary of `value`
                { "line": 1, "character": 10 }, // middle of `value`
                { "line": 1, "character": 13 }, // end boundary of `value`
                { "line": 1, "character": 16 }, // `r` of `r#type`
                { "line": 1, "character": 17 }, // `#` of `r#type`
                { "line": 1, "character": 19 }, // middle of `r#type`
                { "line": 1, "character": 22 }, // end boundary of `r#type`
                { "line": 0, "character": 12 }, // `'` of `'a`
                { "line": 0, "character": 13 }, // `a` of `'a`
                { "line": 0, "character": 14 }, // end boundary of `'a`
                { "line": 1, "character": 2 },  // whitespace indentation
                { "line": 1, "character": 14 }, // `=` punctuation
                { "line": 1, "character": 29 }, // `+` punctuation
                { "line": 1, "character": 32 }  // `;` punctuation
            ]
        }
    });

    let res = handle_selection_range(&req, &overlay_mgr);
    let arr = res.as_array().expect("selectionRange array");
    assert_eq!(arr.len(), 14);

    let value_range =
        json!({ "start": { "line": 1, "character": 8 }, "end": { "line": 1, "character": 13 } });
    for (idx, item) in arr[0..3].iter().enumerate() {
        assert_eq!(item["range"], value_range, "value idx={idx}");
    }

    let raw_ident_range =
        json!({ "start": { "line": 1, "character": 16 }, "end": { "line": 1, "character": 22 } });
    for (idx, item) in arr[3..7].iter().enumerate() {
        assert_eq!(item["range"], raw_ident_range, "r#type idx={idx}");
    }

    let lifetime_range =
        json!({ "start": { "line": 0, "character": 12 }, "end": { "line": 0, "character": 14 } });
    for (idx, item) in arr[7..10].iter().enumerate() {
        assert_eq!(item["range"], lifetime_range, "'a idx={idx}");
    }

    let body_range =
        json!({ "start": { "line": 0, "character": 42 }, "end": { "line": 3, "character": 1 } });
    for (idx, item) in arr[10..14].iter().enumerate() {
        assert_eq!(
            item["range"], body_range,
            "whitespace/punctuation must not emit token range (idx={idx})"
        );
    }
}

#[test]
fn test_selection_range_fn_with_outer_attr_and_doc_comment() {
    let dir = TempTestDir::new("fn_attrs_docs");
    let file_path = dir.path.join("src/attrs.rs");
    let file_str = file_path.to_str().unwrap();
    let uri = path_to_uri(file_str);

    let text = concat!(
        "//! Module doc comment not part of fn\n",
        "\n",
        "/// Computes the answer.\n",
        "#[inline]\n",
        "#[must_use] // keep attribute\n",
        "pub const fn answer(x: i32) -> i32 {\n",
        "    x + 42\n",
        "}\n",
    );
    fs::write(&file_path, text).unwrap();

    let overlay_mgr = OverlayManager::new();
    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "positions": [{ "line": 6, "character": 4 }]
        }
    });

    let res = handle_selection_range(&req, &overlay_mgr);
    assert_eq!(
        res,
        json!([chain_from_ranges(&[
            (6, 4, 6, 5),
            (5, 35, 7, 1),
            (2, 0, 7, 1),
            (0, 0, 8, 0),
        ])])
    );
}
