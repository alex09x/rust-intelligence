/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use ridx::compare::types::{CapabilityKind, ComparisonResult};
use ridx::compare::{read_lsp_response, read_lsp_response_with_writer, send_lsp_msg};
use serde_json::json;
use std::io::Cursor;

#[test]
fn test_comparison_preserves_full_path_and_rejects_errors() {
    let mut res = ComparisonResult::new(CapabilityKind::Def, "test_sym");
    res.ridx_found = true;
    res.prodcode_found = true;
    res.ridx_span = "/one/src/lib.rs:10:1".to_string();
    res.prodcode_span = "/other/src/lib.rs:10:1".to_string();
    assert!(!res.is_match(), "Different directory paths must NOT match");

    res.prodcode_span = "/one/src/lib.rs:10:1".to_string();
    assert!(res.is_match(), "Identical directory paths must match");

    res.ridx_found = false;
    res.prodcode_found = false;
    res.prodcode_span = "Error (failed)".to_string();
    assert!(!res.is_match(), "Errors must not match");
}

#[test]
fn test_review_v4651_harness_routes_response_ids_and_preserves_errors() {
    let notif = json!({
        "jsonrpc": "2.0",
        "method": "window/logMessage",
        "params": {}
    });
    let resp = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "result": ["test"]
    });
    let mut stream = Vec::new();
    send_lsp_msg(&mut stream, &notif).unwrap();
    send_lsp_msg(&mut stream, &resp).unwrap();

    let mut reader = std::io::BufReader::new(Cursor::new(stream));
    let parsed_resp = read_lsp_response(&mut reader, 2).expect("Must find response id 2");
    assert_eq!(parsed_resp["id"], 2);
    assert_eq!(parsed_resp["result"][0], "test");

    let err = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "error": { "code": -32602, "message": "Invalid" }
    });
    let mut err_stream = Vec::new();
    send_lsp_msg(&mut err_stream, &err).unwrap();

    let mut err_reader = std::io::BufReader::new(Cursor::new(err_stream));
    let err_resp = read_lsp_response(&mut err_reader, 2).expect("Must read error response");
    assert!(err_resp.get("error").is_some());
    assert_eq!(err_resp["error"]["code"], -32602);
}

#[test]
fn test_server_request_with_same_id_is_not_treated_as_response() {
    let srv_req = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "window/showMessageRequest",
        "params": { "message": "Hello" }
    });
    let client_resp = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "result": { "contents": "hover markdown" }
    });

    let mut stream = Vec::new();
    send_lsp_msg(&mut stream, &srv_req).unwrap();
    send_lsp_msg(&mut stream, &client_resp).unwrap();

    let mut reader = std::io::BufReader::new(Cursor::new(stream));
    let mut writer = Vec::new();
    let res = read_lsp_response_with_writer(&mut reader, &mut writer, 2).expect("Must retrieve client response id 2");

    assert!(res.get("method").is_none(), "Must not return server request as response");
    assert_eq!(res["result"]["contents"], "hover markdown");

    let out_str = String::from_utf8_lossy(&writer);
    assert!(out_str.contains("\"id\":2"), "Server request must be answered");
    assert!(out_str.contains("\"result\":null"));
}

#[test]
fn test_symbol_resolution_chooses_exact_candidate() {
    let resp = json!({
        "jsonrpc": "2.0",
        "id": 10,
        "result": [
            {
                "name": "foobar",
                "location": {
                    "uri": "file:///fixture/wrong.rs",
                    "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 6 } }
                }
            },
            {
                "name": "foo",
                "location": {
                    "uri": "file:///fixture/lib.rs",
                    "range": { "start": { "line": 10, "character": 4 }, "end": { "line": 10, "character": 7 } }
                }
            }
        ]
    });

    let loc = ridx::compare::client::extract_symbol_location(&resp, "foo").expect("Must resolve exact symbol 'foo'");
    assert_eq!(loc.0, "file:///fixture/lib.rs");
    assert_eq!(loc.1, 10);
    assert_eq!(loc.2, 4);

    let none_loc = ridx::compare::client::extract_symbol_location(&resp, "baz");
    assert!(none_loc.is_none(), "Unmatched symbol must return None");
}

#[test]
fn test_set_metrics_precision_recall_f1() {
    use ridx::compare::calculate_set_metrics;
    use std::collections::HashSet;

    let mut a = HashSet::new();
    a.insert("file:///src/a.rs:10:5".to_string());
    a.insert("file:///src/b.rs:20:5".to_string());

    let mut b = HashSet::new();
    b.insert("file:///src/a.rs:10:5".to_string());
    b.insert("file:///src/c.rs:30:5".to_string());

    let m = calculate_set_metrics(&a, &b);
    assert!((m.precision - 0.5).abs() < 1e-6);
    assert!((m.recall - 0.5).abs() < 1e-6);
    assert!((m.f1 - 0.5).abs() < 1e-6);

    let empty: HashSet<String> = HashSet::new();
    let m_empty = calculate_set_metrics(&empty, &empty);
    assert_eq!(m_empty.f1, 1.0);
}
