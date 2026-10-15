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
use serde_json::json;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static SEL_TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

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
    let expected_disk_chain = json!([
        {
            "range": {
                "start": { "line": 2, "character": 17 },
                "end": { "line": 5, "character": 9 }
            },
            "parent": {
                "range": {
                    "start": { "line": 1, "character": 11 },
                    "end": { "line": 8, "character": 5 }
                },
                "parent": {
                    "range": {
                        "start": { "line": 0, "character": 14 },
                        "end": { "line": 9, "character": 1 }
                    },
                    "parent": {
                        "range": {
                            "start": { "line": 0, "character": 0 },
                            "end": { "line": 10, "character": 0 }
                        },
                        "parent": null
                    }
                }
            }
        }
    ]);
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
        json!([
            {
                "range": {
                    "start": { "line": 0, "character": 3 },
                    "end": { "line": 0, "character": 27 }
                },
                "parent": {
                    "range": {
                        "start": { "line": 0, "character": 0 },
                        "end": { "line": 1, "character": 0 }
                    },
                    "parent": null
                }
            }
        ])
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
        {
            "range": {
                "start": { "line": 0, "character": 7 },
                "end": { "line": 2, "character": 1 }
            },
            "parent": {
                "range": {
                    "start": { "line": 0, "character": 0 },
                    "end": { "line": 3, "character": 0 }
                },
                "parent": null
            }
        },
        {
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": { "line": 3, "character": 0 }
            },
            "parent": null
        }
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

    // Line 0: "pub fn γειά() {\r\n" -> `γειά` starts at UTF-16 col 7
    // Line 1: "    let s = \"🎉\"; { let inner = 1; }\r\n"
    //   "    let s = \"" is 13 UTF-16 units (0..13)
    //   "🎉" is 2 UTF-16 units (13..15)
    //   "\"; " is 3 UTF-16 units (15..18)
    //   "{ let inner = 1; }" spans UTF-16 cols 18..36
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
    let expected = json!([
        {
            "range": {
                "start": { "line": 1, "character": 18 },
                "end": { "line": 1, "character": 36 }
            },
            "parent": {
                "range": {
                    "start": { "line": 0, "character": 7 },
                    "end": { "line": 2, "character": 1 }
                },
                "parent": {
                    "range": {
                        "start": { "line": 0, "character": 0 },
                        "end": { "line": 3, "character": 0 }
                    },
                    "parent": null
                }
            }
        }
    ]);
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
        json!([
            {
                "range": {
                    "start": { "line": 2, "character": 11 },
                    "end": { "line": 6, "character": 5 }
                },
                "parent": {
                    "range": {
                        "start": { "line": 1, "character": 20 },
                        "end": { "line": 7, "character": 1 }
                    },
                    "parent": {
                        "range": {
                            "start": { "line": 0, "character": 0 },
                            "end": { "line": 8, "character": 0 }
                        },
                        "parent": null
                    }
                }
            }
        ])
    );
}
