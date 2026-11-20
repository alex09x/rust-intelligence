/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! End-to-end and protocol conformance stress tests for Domain 5:
//! Language Intelligence (hover, completion, completionItem/resolve,
//! signatureHelp, inlayHint, and inlayHint/resolve).

use ridx::lsp::error::SERVER_NOT_INITIALIZED;
use ridx::lsp::run_lsp_server_stream;
use ridx_format::DefKind;
use ridx_test_support::{make_lsp_msg, parse_lsp_responses, FixtureIndexBuilder};
use serde_json::json;
use std::fs;
use std::io::Cursor;

#[test]
fn test_domain5_pre_initialize_requests_rejected_with_32002() {
    let mut input = Vec::new();

    // 1. Pre-init hover
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 101,
        "method": "textDocument/hover",
        "params": {
            "textDocument": { "uri": "file:///tmp/pre_init.rs" },
            "position": { "line": 0, "character": 0 }
        }
    })));

    // 2. Pre-init completion
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 102,
        "method": "textDocument/completion",
        "params": {
            "textDocument": { "uri": "file:///tmp/pre_init.rs" },
            "position": { "line": 0, "character": 0 }
        }
    })));

    // 3. Pre-init completionItem/resolve
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 103,
        "method": "completionItem/resolve",
        "params": {
            "label": "test_symbol"
        }
    })));

    // 4. Pre-init signatureHelp
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 104,
        "method": "textDocument/signatureHelp",
        "params": {
            "textDocument": { "uri": "file:///tmp/pre_init.rs" },
            "position": { "line": 0, "character": 0 }
        }
    })));

    // 5. Pre-init inlayHint
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 105,
        "method": "textDocument/inlayHint",
        "params": {
            "textDocument": { "uri": "file:///tmp/pre_init.rs" },
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": { "line": 5, "character": 0 }
            }
        }
    })));

    // 6. Pre-init inlayHint/resolve
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 106,
        "method": "inlayHint/resolve",
        "params": {
            "position": { "line": 0, "character": 0 },
            "label": ": i32"
        }
    })));

    // Clean initialization and termination
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": { "capabilities": {} }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "shutdown",
        "params": null
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "exit"
    })));

    let mut output = Vec::new();
    let res = run_lsp_server_stream(Cursor::new(input), &mut output, None);
    assert!(res.is_ok());

    let responses = parse_lsp_responses(&output);
    assert!(responses.len() >= 8);

    for (idx, resp) in responses.iter().take(6).enumerate() {
        assert_eq!(resp["id"], 101 + idx as i64);
        assert_eq!(resp["error"]["code"], SERVER_NOT_INITIALIZED);
    }

    assert_eq!(responses[6]["id"], 1);
    assert!(responses[6]["result"]["capabilities"].is_object());
    assert_eq!(responses[7]["id"], 2);
}

#[test]
fn test_domain5_e2e_full_intelligence_stream() {
    let temp = tempfile::tempdir().unwrap();
    let src_file = temp.path().join("src/lib.rs");
    let _ = fs::create_dir_all(temp.path().join("src"));
    let code = "pub fn add(a: i32, b: i32) -> i32 { a + b }\npub struct Point { x: i32, y: i32 }\nfn caller() { add(10, 20); }\n";
    fs::write(&src_file, code).unwrap();

    let mut builder = FixtureIndexBuilder::new("domain5_test", "0.1.0");
    let (c, _ids) = builder.add_defs_with_ids(code, src_file.to_str().unwrap(), "domain5_test", DefKind::Fn);
    let _ = fs::write(&src_file, c);

    let ridx_file = temp.path().join("index.ridx");
    builder.write_to_file(&ridx_file);

    let uri = format!("file://{}", src_file.to_str().unwrap());
    let mut input = Vec::new();

    // 1. Initialize
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "rootUri": format!("file://{}", temp.path().to_str().unwrap()),
            "capabilities": {}
        }
    })));

    // 2. didOpen
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri,
                "languageId": "rust",
                "version": 1,
                "text": code
            }
        }
    })));

    // 3. hover
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 10,
        "method": "textDocument/hover",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 8 }
        }
    })));

    // 4. completion
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 11,
        "method": "textDocument/completion",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 2, "character": 18 }
        }
    })));

    // 5. completionItem/resolve
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 12,
        "method": "completionItem/resolve",
        "params": {
            "label": "add",
            "kind": 3,
            "detail": "pub fn add(a: i32, b: i32) -> i32"
        }
    })));

    // 6. signatureHelp
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 13,
        "method": "textDocument/signatureHelp",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 2, "character": 21 }
        }
    })));

    // 7. inlayHint
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 14,
        "method": "textDocument/inlayHint",
        "params": {
            "textDocument": { "uri": &uri },
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": { "line": 3, "character": 0 }
            }
        }
    })));

    // 8. inlayHint/resolve
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 15,
        "method": "inlayHint/resolve",
        "params": {
            "position": { "line": 0, "character": 0 },
            "label": ": i32",
            "kind": 1
        }
    })));

    // 9. Shutdown and exit
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 99,
        "method": "shutdown",
        "params": null
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "exit"
    })));

    let mut output = Vec::new();
    let res = run_lsp_server_stream(Cursor::new(input), &mut output, Some(temp.path().to_path_buf()));
    assert!(res.is_ok());

    let responses = parse_lsp_responses(&output);

    // Verify hover response
    let hover_res = responses.iter().find(|r| r["id"] == 10).unwrap();
    assert!(hover_res["result"].is_object() || hover_res["result"].is_null());

    // Verify completion response
    let comp_res = responses.iter().find(|r| r["id"] == 11).unwrap();
    assert!(comp_res["result"]["items"].is_array());

    // Verify completionItem/resolve response
    let comp_resolve_res = responses.iter().find(|r| r["id"] == 12).unwrap();
    assert_eq!(comp_resolve_res["result"]["label"], "add");
    assert!(!comp_resolve_res["result"]["documentation"].is_null());

    // Verify signatureHelp response
    let sig_res = responses.iter().find(|r| r["id"] == 13).unwrap();
    assert!(sig_res["result"].is_object() || sig_res["result"].is_null());

    // Verify inlayHint response
    let hint_res = responses.iter().find(|r| r["id"] == 14).unwrap();
    assert!(hint_res["result"].is_array());

    // Verify inlayHint/resolve response
    let hint_resolve_res = responses.iter().find(|r| r["id"] == 15).unwrap();
    assert_eq!(hint_resolve_res["result"]["label"], ": i32");
    assert!(!hint_resolve_res["result"]["tooltip"].is_null());
}

#[test]
fn test_domain5_concurrent_intelligence_requests_stress() {
    let temp = tempfile::tempdir().unwrap();
    let src_file = temp.path().join("src/lib.rs");
    let _ = fs::create_dir_all(temp.path().join("src"));
    let code = "pub fn calculate(x: i32) -> i32 { x * 2 }\n";
    fs::write(&src_file, code).unwrap();

    let uri = format!("file://{}", src_file.to_str().unwrap());
    let mut input = Vec::new();

    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "rootUri": format!("file://{}", temp.path().to_str().unwrap()),
            "capabilities": {}
        }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri,
                "languageId": "rust",
                "version": 1,
                "text": code
            }
        }
    })));

    // Send 30 interleaved intelligence requests with mid-stream cancellation of request 115. The cancel follows its
    // request, as in the protocol (a cancel of a request not sent is ignored); 115 waits for a version never sent
    // (`minVersion`), so it is still pending when the cancel comes, whatever the timing.
    for i in 0..30 {
        let req_id = 100 + i;
        let method = match i % 3 {
            0 => "textDocument/hover",
            1 => "textDocument/completion",
            _ => "textDocument/signatureHelp",
        };
        let mut params = json!({ "textDocument": { "uri": &uri }, "position": { "line": 0, "character": 8 } });
        if req_id == 115 {
            params["minVersion"] = json!(2);
        }
        input.extend(make_lsp_msg(&json!({ "jsonrpc": "2.0", "id": req_id, "method": method, "params": params })));
        if req_id == 115 {
            input.extend(make_lsp_msg(&json!({ "jsonrpc": "2.0", "method": "$/cancelRequest", "params": { "id": 115 } })));
        }
    }

    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 999,
        "method": "shutdown",
        "params": null
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "exit"
    })));

    let mut output = Vec::new();
    let res = run_lsp_server_stream(Cursor::new(input), &mut output, Some(temp.path().to_path_buf()));
    assert!(res.is_ok());

    let responses = parse_lsp_responses(&output);
    for i in 0..30 {
        let req_id = 100 + i;
        let r = responses.iter().find(|res| res["id"] == req_id);
        assert!(r.is_some(), "Expected response for request {}", req_id);
        let resp = r.unwrap();
        if req_id == 115 {
            assert_eq!(resp["error"]["code"], -32800, "Cancelled request 115 must yield code -32800 (RequestCancelled)");
        } else {
            assert!(resp["error"].is_null(), "Non-cancelled request {} must succeed, got {:?}", req_id, resp["error"]);
        }
    }
}
