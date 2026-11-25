/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use super::common::*;
use crate::lsp::*;
use ridx_format::writer::{IndexWriter, RawDefinition};
use serde_json::json;
use std::io::Cursor;

#[test]
fn test_lsp_server_stream_sync_and_unopened_change() {
    let temp_dir = std::env::temp_dir().join(format!("ridx_sync_test_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&temp_dir);
    let src_file = temp_dir.join("src_file.rs");
    let content = "pub fn foo() {}\n";
    std::fs::write(&src_file, content).unwrap();

    let hash = ridx_format::normalized_source_hash(content.as_bytes());
    let mut writer = IndexWriter::new("sync_crate", "0.1.0");
    let mut map = std::collections::HashMap::new();
    map.insert(src_file.to_str().unwrap().to_string(), hash);
    writer.set_file_hashes(map);
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: ridx_format::DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "foo".to_string(),
        path: "sync_crate::foo".to_string(),
        signature: None,
        file: src_file.to_str().unwrap().to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 14,
    });
    let index_file = temp_dir.join("index.ridx");
    let mut f = std::fs::File::create(&index_file).unwrap();
    writer.write_to(&mut f).unwrap();
    drop(f);

    let mut ws = WorkspaceIndex::open(&index_file).unwrap();
    let mut overlay_mgr = crate::overlay::OverlayManager::new();
    let uri = format!("file://{}", src_file.to_str().unwrap());
    overlay_mgr.open_document(&uri, src_file.to_str().unwrap(), 1, content.to_string());

    // Trigger reload
    std::thread::sleep(std::time::Duration::from_millis(50));
    // Beside the mapped shard and renamed over it (Windows refuses a write into a mapped file).
    let tmp = index_file.with_extension("ridx.tmp");
    let mut f2 = std::fs::File::create(&tmp).unwrap();
    writer.write_to(&mut f2).unwrap();
    drop(f2);
    std::fs::rename(&tmp, &index_file).unwrap();

    crate::lsp::sync_reloaded_index(&mut ws, &mut overlay_mgr);

    let mut input = Vec::new();
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": { "capabilities": {}, "workspaceFolders": [{ "uri": uri, "name": "root" }] }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "initialize",
        "params": { "capabilities": {} }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "workspace/didChangeConfiguration",
        "params": { "settings": {} }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "workspace/didChangeWatchedFiles",
        "params": { "changes": [{ "uri": uri, "type": 1 }] }
    })));
    let unopened_uri = format!("file://{}", src_file.to_str().unwrap());
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didChange",
        "params": {
            "textDocument": { "uri": unopened_uri, "version": 2 },
            "contentChanges": [{ "text": "pub fn foo_changed() {}\n" }]
        }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 3,
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
    assert!(!responses.is_empty());

    let double_init = responses.iter().find(|r| r["id"] == 2);
    assert!(double_init.is_some());
    assert!(double_init.unwrap()["error"].is_object());

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_lsp_server_stream_exit_before_shutdown_and_broken_pipe() {
    let mut input = Vec::new();
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "exit"
    })));
    let mut output = Vec::new();
    let res = run_lsp_server_stream(Cursor::new(input), &mut output, None);
    assert!(res.is_err());
    assert_eq!(res.unwrap_err().to_string(), "exit notification received before shutdown");

    struct BrokenWriter;
    impl std::io::Write for BrokenWriter {
        fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "broken pipe"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "broken pipe"))
        }
    }

    // 1. Uninitialized request error response fails to write
    let mut uninit_input = Vec::new();
    uninit_input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "textDocument/definition",
        "params": {}
    })));
    let res = run_lsp_server_stream(Cursor::new(uninit_input), BrokenWriter, None);
    assert!(res.is_err());

    // 2. Initialized request response fails to write
    let mut init_input = Vec::new();
    init_input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": { "capabilities": {} }
    })));
    let res2 = run_lsp_server_stream(Cursor::new(init_input), BrokenWriter, None);
    assert!(res2.is_err());

    // 3. Post-shutdown request rejection fails to write
    struct BrokenWriterAfterInit {
        writes: usize,
    }
    impl std::io::Write for BrokenWriterAfterInit {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.writes += 1;
            if self.writes > 2 {
                Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "broken pipe"))
            } else {
                Ok(buf.len())
            }
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    let mut shutdown_input = Vec::new();
    shutdown_input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": { "capabilities": {} }
    })));
    shutdown_input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "shutdown",
        "params": null
    })));
    shutdown_input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "textDocument/definition",
        "params": {}
    })));
    let res3 = run_lsp_server_stream(Cursor::new(shutdown_input), BrokenWriterAfterInit { writes: 0 }, None);
    assert!(res3.is_err());
}

#[test]
fn test_lsp_server_stream_folders_and_file_events() {
    let temp_dir = std::env::temp_dir().join(format!("ridx_lsp_folders_{}", std::process::id()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let folder_uri = format!("file://{}", temp_dir.display());

    let mut input = Vec::new();
    // 1. initialize with workspaceFolders when index_path is None and multi_root is empty (line 216)
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "capabilities": {},
            "workspaceFolders": [
                { "uri": folder_uri, "name": "root_folder" }
            ]
        }
    })));

    // 2. workspace/didRenameFiles (lines 298-311)
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "workspace/didRenameFiles",
        "params": {
            "files": [
                {
                    "oldUri": format!("file://{}/src/old.rs", temp_dir.display()),
                    "newUri": format!("file://{}/src/new.rs", temp_dir.display())
                }
            ]
        }
    })));

    // 3. workspace/didDeleteFiles (lines 313-324)
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "workspace/didDeleteFiles",
        "params": {
            "files": [
                {
                    "uri": format!("file://{}/src/new.rs", temp_dir.display())
                }
            ]
        }
    })));

    // 4. shutdown and exit
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "shutdown",
        "params": null
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "exit",
        "params": null
    })));

    let mut output = Vec::new();
    let res = run_lsp_server_stream(Cursor::new(input), &mut output, None);
    assert!(res.is_ok());

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_lsp_server_stream_initialize_with_empty_multi_root() {
    let mut input = Vec::new();
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "workspaceFolders": [
                { "uri": "file:///tmp/workspace", "name": "workspace" }
            ],
            "capabilities": {}
        }
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
    let res = run_lsp_server_stream(Cursor::new(input), &mut output, Some(PathBuf::from("/nonexistent/empty_mr.ridx")));
    assert!(res.is_ok());
    let responses = parse_lsp_responses(&output);
    assert_eq!(responses.len(), 2);
    assert_eq!(responses[0]["id"], 1);
}
