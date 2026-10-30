/*
 * Rust Intelligence (ridx)
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for codeLens/resolve (Domain 7, Part 5).
//! Tests command argument shape, location URI/range structures, custom range preservation, extra fields, and empty/null data handling.

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

static LENS_P5_COUNTER: AtomicU64 = AtomicU64::new(1);

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
    fn new(shard: Option<PathBuf>) -> Self {
        let mut d = Self {
            lifecycle: LifecycleManager::new(),
            multi_root: MultiRootWorkspace::new(),
            overlay_mgr: OverlayManager::new(),
            cancelled: Arc::new(Mutex::new(HashSet::new())),
            writer: Vec::new(),
        };
        if let Some(s) = shard {
            assert!(d.multi_root.add_root(s));
        }
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
fn test_code_lens_resolve_arguments_structure() {
    let mut dsp = TestDispatcher::new(None);
    let file = std::env::temp_dir().join(format!("ridx_lens_p5_args_{}.rs", LENS_P5_COUNTER.fetch_add(1, Ordering::Relaxed)));

    let req = json!({
        "jsonrpc": "2.0", "id": 501, "method": "codeLens/resolve",
        "params": {
            "range": { "start": { "line": 2, "character": 4 }, "end": { "line": 2, "character": 12 } },
            "data": { "uri": to_uri(&file), "line": 3, "character": 5, "name": "sample_fn" }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 501);
    let cmd = &resp["result"]["command"];
    assert_eq!(cmd["command"], "ridx.showReferences");
    assert_eq!(cmd["title"], "References: sample_fn");
    assert_eq!(cmd["arguments"][0], to_uri(&file));
    assert_eq!(cmd["arguments"][1]["line"], 2);
    assert_eq!(cmd["arguments"][1]["character"], 4);
    assert_eq!(cmd["arguments"][2], json!([]));
}

#[test]
fn test_code_lens_resolve_location_range_and_uri() {
    let id = LENS_P5_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_lens_p5_loc_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let mut writer = IndexWriter::new("loc_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "target".to_string(),
        path: "loc_crate::target".to_string(),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 7,
    });
    writer.add_reference(RawReference {
        file: "src/lib.rs".to_string(),
        start_line: 3,
        start_col: 5,
        end_line: 3,
        end_col: 11,
        target_def_id: Some(1),
        target_path: "loc_crate::target".to_string(),
        target_file: Some("src/lib.rs".to_string()),
        target_unit: None,
        target_line: 1,
        target_col: 1,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    let file = dir.join("src/lib.rs");
    std::fs::write(&file, "fn target() {}\nfn caller() {\n    target();\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(Some(shard));

    let req = json!({
        "jsonrpc": "2.0", "id": 502, "method": "codeLens/resolve",
        "params": {
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 9 } },
            "data": { "uri": to_uri(&file), "line": 1, "character": 1, "name": "target" }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["result"]["command"]["title"], "1 reference");
    let locs = resp["result"]["command"]["arguments"][2].as_array().unwrap();
    assert_eq!(locs.len(), 1);
    assert_eq!(locs[0]["range"]["start"]["line"], 2);
    assert_eq!(locs[0]["range"]["start"]["character"], 4);
    assert_eq!(locs[0]["range"]["end"]["line"], 2);
    assert_eq!(locs[0]["range"]["end"]["character"], 10);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_code_lens_resolve_preserves_custom_lens_range() {
    let mut dsp = TestDispatcher::new(None);
    let file = std::env::temp_dir().join(format!("ridx_lens_p5_range_{}.rs", LENS_P5_COUNTER.fetch_add(1, Ordering::Relaxed)));

    let req = json!({
        "jsonrpc": "2.0", "id": 503, "method": "codeLens/resolve",
        "params": {
            "range": { "start": { "line": 10, "character": 2 }, "end": { "line": 10, "character": 18 } },
            "data": { "uri": to_uri(&file), "line": 11, "character": 3, "name": "preserved_fn" }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 503);
    assert_eq!(resp["result"]["range"]["start"]["line"], 10);
    assert_eq!(resp["result"]["range"]["start"]["character"], 2);
    assert_eq!(resp["result"]["range"]["end"]["line"], 10);
    assert_eq!(resp["result"]["range"]["end"]["character"], 18);
}

#[test]
fn test_code_lens_resolve_preserves_additional_fields() {
    let mut dsp = TestDispatcher::new(None);
    let file = std::env::temp_dir().join(format!("ridx_lens_p5_extra_{}.rs", LENS_P5_COUNTER.fetch_add(1, Ordering::Relaxed)));

    let req = json!({
        "jsonrpc": "2.0", "id": 504, "method": "codeLens/resolve",
        "params": {
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 5 } },
            "data": { "uri": to_uri(&file), "line": 1, "character": 1, "name": "test_extra" },
            "customMeta": "custom_value_preserved",
            "tag": 42
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 504);
    assert_eq!(resp["result"]["customMeta"], "custom_value_preserved");
    assert_eq!(resp["result"]["tag"], 42);
}

#[test]
fn test_code_lens_resolve_empty_data_object() {
    let mut dsp = TestDispatcher::new(None);

    let req = json!({
        "jsonrpc": "2.0", "id": 505, "method": "codeLens/resolve",
        "params": {
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 5 } },
            "data": {}
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 505);
    assert_eq!(resp["result"]["command"]["title"], "References: ");
    assert_eq!(resp["result"]["command"]["arguments"][2], json!([]));
}

#[test]
fn test_code_lens_resolve_null_data() {
    let mut dsp = TestDispatcher::new(None);

    let req = json!({
        "jsonrpc": "2.0", "id": 506, "method": "codeLens/resolve",
        "params": {
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 5 } },
            "data": Value::Null
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 506);
    assert_eq!(resp["result"]["command"]["title"], "References: ");
    assert_eq!(resp["result"]["command"]["arguments"][2], json!([]));
}
