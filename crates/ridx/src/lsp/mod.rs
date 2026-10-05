//! Lightweight, zero-memory Language Server Protocol (LSP) bridge for Rust Intelligence.
//!
//! Provides instant (<50 µs) textDocument/definition, textDocument/references,
//! textDocument/hover, and workspace/symbol over stdio without running a heavy background daemon.

pub mod handlers;
pub mod protocol;

pub use protocol::*;

use handlers::*;
use ridx_format::WorkspaceIndex;
use serde_json::{json, Value};
use std::io::{self, BufRead};
use std::path::PathBuf;

fn sync_reloaded_index(ws: &mut WorkspaceIndex, overlay_mgr: &mut crate::overlay::OverlayManager) {
    if let Ok(true) = ws.reload_if_modified() {
        for file in overlay_mgr.open_file_paths() {
            if ws.is_file_reloaded(&file) {
                if let Ok(content) = std::fs::read_to_string(&file) {
                    let disk_hash = ridx_format::normalized_source_hash(content.as_bytes());
                    if let Some(compiled_hash) = ws.source_hash_for_file(&file) {
                        if disk_hash == compiled_hash {
                            overlay_mgr.advance_compiled_baseline(&file, content);
                        }
                    }
                }
            }
        }
    }
}

/// Runs the LSP server over standard I/O.
pub fn run_lsp_server(index_path: Option<PathBuf>) -> io::Result<()> {
    let stdin = io::stdin();
    let mut reader = stdin.lock();
    let stdout = io::stdout();
    let mut writer = stdout.lock();
    run_lsp_server_stream(&mut reader, &mut writer, index_path)
}

/// Runs the LSP server over arbitrary reader and writer streams.
pub fn run_lsp_server_stream<R: BufRead, W: io::Write>(
    mut reader: R,
    mut writer: W,
    index_path: Option<PathBuf>,
) -> io::Result<()> {
    let mut workspace: Option<WorkspaceIndex> = match index_path {
        Some(p) => WorkspaceIndex::open(&p).ok(),
        None => WorkspaceIndex::auto_discover().ok(),
    };

    let mut overlay_mgr = crate::overlay::OverlayManager::new();
    let mut cancelled_requests: std::collections::HashSet<Value> = std::collections::HashSet::new();
    let mut is_shutdown = false;

    loop {
        // Read headers
        let mut content_length: Option<usize> = None;
        let mut header_line = String::new();

        loop {
            header_line.clear();
            let bytes_read = reader.read_line(&mut header_line)?;
            if bytes_read == 0 {
                // EOF
                return Ok(());
            }
            let trimmed = header_line.trim_end_matches(['\r', '\n']);
            if trimmed.is_empty() {
                // End of headers
                break;
            }
            if let Some(len_str) = trimmed.strip_prefix("Content-Length:") {
                content_length = len_str.trim().parse::<usize>().ok();
            }
        }

        let Some(len) = content_length else {
            continue;
        };

        let mut body_buf = vec![0u8; len];
        reader.read_exact(&mut body_buf)?;

        let msg: Value = match serde_json::from_slice(&body_buf) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let id = msg.get("id").cloned();

        // Enforce LSP lifecycle: reject requests after shutdown
        if is_shutdown {
            if method == "exit" {
                return Ok(());
            }
            if let Some(ref req_id) = id {
                let resp = json!({
                    "jsonrpc": "2.0",
                    "id": req_id,
                    "error": {
                        "code": -32600,
                        "message": "Server is shutting down"
                    }
                });
                send_response(&mut writer, &resp)?;
            }
            continue;
        }

        // Handle request cancellation before dispatch
        if let Some(ref req_id) = id {
            if cancelled_requests.remove(req_id) {
                let resp = json!({
                    "jsonrpc": "2.0",
                    "id": req_id,
                    "error": {
                        "code": -32800,
                        "message": "Request cancelled"
                    }
                });
                send_response(&mut writer, &resp)?;
                continue;
            }
        }

        // Automatically reload indices if files changed on disk and sync open overlay baselines
        if let Some(ref mut ws) = workspace {
            sync_reloaded_index(ws, &mut overlay_mgr);
        }

        // Request-scoped line cache: reuses read file lines across all coordinate
        // conversions within this request (e.g. high-fanout references or symbols),
        // and automatically drops all allocated buffers when the request completes.
        let mut conv = PositionConverter::new();

        match method {
            "initialize" => {
                if workspace.is_none() {
                    // 1. Try to initialize from workspaceFolders
                    if let Some(folders) = msg["params"]["workspaceFolders"].as_array() {
                        for folder in folders {
                            if let Some(uri) = folder["uri"].as_str() {
                                let path = uri_to_path(uri);
                                if let Ok(ws) = WorkspaceIndex::open(&path) {
                                    workspace = Some(ws);
                                    break;
                                }
                            }
                        }
                    }
                    // 2. Try to initialize from rootUri or rootPath
                    if workspace.is_none() {
                        if let Some(root_uri) = msg["params"]["rootUri"].as_str() {
                            let path = uri_to_path(root_uri);
                            workspace = WorkspaceIndex::open(&path).ok();
                        } else if let Some(root_path) = msg["params"]["rootPath"].as_str() {
                            workspace = WorkspaceIndex::open(root_path).ok();
                        }
                    }
                    // 3. Fallback to auto-discovery
                    if workspace.is_none() {
                        workspace = WorkspaceIndex::auto_discover().ok();
                    }
                }

                let resp = json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "capabilities": {
                            "textDocumentSync": {
                                "openClose": true,
                                "change": 2, // Incremental document sync (TextDocumentSyncKind.Incremental)
                                "save": {
                                    "includeText": true
                                }
                            },
                            "workspace": {
                                "workspaceFolders": {
                                    "supported": true,
                                    "changeNotifications": true
                                }
                            },
                            "definitionProvider": true,
                            "referencesProvider": true,
                            "hoverProvider": true,
                            "workspaceSymbolProvider": true,
                            "documentSymbolProvider": true
                        },
                        "serverInfo": {
                            "name": "ridx",
                            "version": env!("CARGO_PKG_VERSION")
                        }
                    }
                });
                send_response(&mut writer, &resp)?;
            }

            "initialized" => {
                // No-op notification from client
            }

            "$/cancelRequest" => {
                if let Some(cancel_id) = msg["params"].get("id") {
                    cancelled_requests.insert(cancel_id.clone());
                    if cancelled_requests.len() > 1000 {
                        cancelled_requests.clear();
                    }
                }
            }

            "workspace/didChangeWorkspaceFolders" => {
                if let Some(added) = msg["params"]["event"]["added"].as_array() {
                    if workspace.is_none() {
                        for folder in added {
                            if let Some(uri) = folder["uri"].as_str() {
                                let path = uri_to_path(uri);
                                if let Ok(ws) = WorkspaceIndex::open(&path) {
                                    workspace = Some(ws);
                                    break;
                                }
                            }
                        }
                    }
                }
            }

            "workspace/didChangeWatchedFiles" => {
                if let Some(ref mut ws) = workspace {
                    sync_reloaded_index(ws, &mut overlay_mgr);
                }
            }

            "textDocument/didOpen" => {
                let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
                let file = uri_to_path(uri);
                let version = msg["params"]["textDocument"]["version"]
                    .as_i64()
                    .unwrap_or(1) as i32;
                let text = msg["params"]["textDocument"]["text"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();

                let is_compiled = if let Some(ref ws) = workspace {
                    if let Ok(disk_text) = std::fs::read_to_string(&file) {
                        if disk_text == text {
                            let text_hash = ridx_format::normalized_source_hash(text.as_bytes());
                            if let Some(compiled_hash) = ws.source_hash_for_file(&file) {
                                text_hash == compiled_hash
                            } else {
                                false
                            }
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                } else {
                    false
                };
                overlay_mgr.open_with_provenance(uri, &file, version, text, is_compiled);
            }

            "textDocument/didChange" => {
                let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
                let file = uri_to_path(uri);
                let version = msg["params"]["textDocument"]["version"]
                    .as_i64()
                    .unwrap_or(1) as i32;
                if let Some(changes) = msg["params"]["contentChanges"].as_array() {
                    if !overlay_mgr.apply_content_changes(&file, version, changes) {
                        let initial_text = std::fs::read_to_string(&file).unwrap_or_default();
                        overlay_mgr.open_with_provenance(uri, &file, version, initial_text, false);
                        overlay_mgr.apply_content_changes(&file, version, changes);
                    }
                }
            }

            "textDocument/didClose" => {
                let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
                let file = uri_to_path(uri);
                overlay_mgr.close(&file);
            }

            "textDocument/didSave" => {
                let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
                let file = uri_to_path(uri);
                if let Ok(content) = std::fs::read(&file) {
                    let hash = ridx_format::normalized_source_hash(&content);
                    overlay_mgr.reconcile_with_disk(&file, hash);
                }
                if let Some(ref mut ws) = workspace {
                    sync_reloaded_index(ws, &mut overlay_mgr);
                }
            }

            "textDocument/definition" => {
                if let Some(id) = id {
                    let result =
                        handle_definition(&msg, workspace.as_ref(), &overlay_mgr, &mut conv);
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result
                    });
                    send_response(&mut writer, &resp)?;
                }
            }

            "textDocument/references" => {
                if let Some(id) = id {
                    let result =
                        handle_references(&msg, workspace.as_ref(), &overlay_mgr, &mut conv);
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result
                    });
                    send_response(&mut writer, &resp)?;
                }
            }

            "textDocument/hover" => {
                if let Some(id) = id {
                    let result = handle_hover(&msg, workspace.as_ref(), &overlay_mgr, &mut conv);
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result
                    });
                    send_response(&mut writer, &resp)?;
                }
            }

            "workspace/symbol" => {
                if let Some(id) = id {
                    let result = handle_workspace_symbol(&msg, workspace.as_ref(), &mut conv);
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result
                    });
                    send_response(&mut writer, &resp)?;
                }
            }

            "textDocument/documentSymbol" => {
                if let Some(id) = id {
                    let result = handle_document_symbol(&msg, &overlay_mgr, &mut conv);
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result
                    });
                    send_response(&mut writer, &resp)?;
                }
            }

            "shutdown" => {
                is_shutdown = true;
                if let Some(id) = id {
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": Value::Null
                    });
                    send_response(&mut writer, &resp)?;
                }
            }

            "exit" => {
                if is_shutdown {
                    return Ok(());
                } else {
                    return Err(io::Error::other(
                        "exit notification received before shutdown",
                    ));
                }
            }

            _ => {
                // Unknown method: ignore notifications, error for requests
                if let Some(id) = id {
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": {
                            "code": -32601,
                            "message": format!("Method not found: {method}")
                        }
                    });
                    send_response(&mut writer, &resp)?;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_position_converter_caching_and_utf16() {
        let temp_dir = std::env::temp_dir().join(format!("ridx_conv_test_{}", std::process::id()));
        std::fs::create_dir_all(&temp_dir).unwrap();
        let file_path = temp_dir.join("test.rs");

        // Line 1: ASCII
        // Line 2: Cyrillic ("привет" - 6 chars, 6 UTF-16, 12 UTF-8 bytes)
        // Line 3: Emoji ("🚀 rocket" - 1 char = 2 UTF-16 code units)
        let content = "fn main() {\n    let s = \"привет\";\n    let rocket = \"🚀!\";\n}\n";
        std::fs::write(&file_path, content).unwrap();

        let path_str = file_path.to_str().unwrap();
        let mut conv = PositionConverter::new();

        // Line 2: `    let s = "` has 13 chars. `привет` starts at char 14.
        // UTF-16 count of first 13 chars is 13.
        let utf16_col = conv.char_col_to_utf16_col(path_str, 2, 14);
        assert_eq!(utf16_col, 13);
        assert!(conv.cache.contains_key(path_str));

        // Line 3: `    let rocket = "` has 18 chars. `🚀` is at char 19 (1-based).
        // `!` is at char 20 (1-based).
        // Before `!`, there are 18 ASCII chars (18 UTF-16) + 1 emoji (2 UTF-16 code units) = 20 UTF-16 code units.
        let utf16_col_bang = conv.char_col_to_utf16_col(path_str, 3, 20);
        assert_eq!(utf16_col_bang, 20);

        // Convert back from 0-based UTF-16 col 20 to 1-based char col
        let char_col = conv.utf16_col_to_char_col(path_str, 3, 20);
        assert_eq!(char_col, 20);

        // Verify request-scoped converter drops cleanly
        drop(conv);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_sync_reloaded_index_provenance_check() {
        use ridx_format::{DefKind, IndexWriter, RawDefinition, WorkspaceIndex};

        let temp_dir = std::env::temp_dir().join(format!("ridx_sync_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);
        let ws_dir = temp_dir.join("target/ridx");
        let src_dir = temp_dir.join("src");
        std::fs::create_dir_all(&ws_dir).unwrap();
        std::fs::create_dir_all(&src_dir).unwrap();

        let foo_path = src_dir.join("foo.rs");
        let foo_code = "fn foo() {}\n";
        std::fs::write(&foo_path, foo_code).unwrap();

        let foo_ridx = ws_dir.join("crate_foo.ridx");
        let deps_dir = ws_dir.join(".deps");
        std::fs::create_dir_all(&deps_dir).unwrap();
        let write_foo_meta = |code: &str| {
            let mut hashes = std::collections::HashMap::new();
            hashes.insert(
                foo_path.to_str().unwrap().to_string(),
                ridx_format::fnv1a_64(code.as_bytes()),
            );
            let meta = serde_json::json!({
                "name": "crate_foo",
                "version": "0.1.0",
                "unit_key": "crate_foo_unit",
                "object_path": foo_ridx.to_str().unwrap(),
                "file_hashes": hashes,
            });
            std::fs::write(
                deps_dir.join("crate_foo.json"),
                serde_json::to_string(&meta).unwrap(),
            )
            .unwrap();
        };

        // 1. Build initial index for crate_foo
        let mut writer_foo = IndexWriter::new("crate_foo", "0.1.0");
        writer_foo.add_definition(RawDefinition {
            def_id: 1,
            kind: DefKind::Fn,
            visibility: 0,
            is_macro_expansion: false,
            name: "foo".to_string(),
            path: "crate_foo::foo".to_string(),
            signature: Some("fn()".to_string()),
            file: foo_path.to_str().unwrap().to_string(),
            start_line: 1,
            start_col: 1,
            end_line: 1,
            end_col: 11,
        });
        let f_foo = std::fs::File::create(&foo_ridx).unwrap();
        writer_foo
            .write_to(&mut std::io::BufWriter::new(f_foo))
            .unwrap();
        write_foo_meta(foo_code);

        let mut ws = WorkspaceIndex::open(&ws_dir).unwrap();
        let mut overlay_mgr = crate::overlay::OverlayManager::new();

        // Open foo.rs in overlay with compiled provenance = true
        let foo_str = foo_path.to_str().unwrap();
        overlay_mgr.open_with_provenance("file://foo.rs", foo_str, 1, foo_code.to_string(), true);
        assert!(!overlay_mgr.get(foo_str).unwrap().is_dirty());

        // Modify foo.rs and save to disk (uncompiled saved edit)
        let modified_code = "fn foo() {}\nfn foo_uncompiled() {}\n";
        std::fs::write(&foo_path, modified_code).unwrap();
        overlay_mgr.update(foo_str, 2, modified_code.to_string());
        assert!(overlay_mgr.get(foo_str).unwrap().is_dirty());

        // Now, an unrelated crate `crate_bar` is compiled and published
        let mut writer_bar = IndexWriter::new("crate_bar", "0.1.0");
        writer_bar.add_definition(RawDefinition {
            def_id: 1,
            kind: DefKind::Fn,
            visibility: 0,
            is_macro_expansion: false,
            name: "bar".to_string(),
            path: "crate_bar::bar".to_string(),
            signature: Some("fn()".to_string()),
            file: "src/bar.rs".to_string(),
            start_line: 1,
            start_col: 1,
            end_line: 1,
            end_col: 11,
        });
        let bar_ridx = ws_dir.join("crate_bar.ridx");
        let f_bar = std::fs::File::create(&bar_ridx).unwrap();
        writer_bar
            .write_to(&mut std::io::BufWriter::new(f_bar))
            .unwrap();

        // Sync reloaded index: should reload crate_bar, but MUST NOT certify foo.rs!
        sync_reloaded_index(&mut ws, &mut overlay_mgr);
        assert!(
            overlay_mgr.get(foo_str).unwrap().is_dirty(),
            "unrelated crate reload must NOT certify uncompiled saved edits in foo.rs"
        );

        // Case: user saves newer edit while compilation runs (compilation had older snapshot `modified_code`)
        std::thread::sleep(std::time::Duration::from_millis(50));
        let mut writer_foo_v2 = IndexWriter::new("crate_foo", "0.1.0");
        writer_foo_v2.add_definition(RawDefinition {
            def_id: 1,
            kind: DefKind::Fn,
            visibility: 0,
            is_macro_expansion: false,
            name: "foo".to_string(),
            path: "crate_foo::foo".to_string(),
            signature: Some("fn()".to_string()),
            file: foo_path.to_str().unwrap().to_string(),
            start_line: 1,
            start_col: 1,
            end_line: 1,
            end_col: 11,
        });
        let f_foo2 = std::fs::File::create(&foo_ridx).unwrap();
        writer_foo_v2
            .write_to(&mut std::io::BufWriter::new(f_foo2))
            .unwrap();
        // Index published with provenance of modified_code
        write_foo_meta(modified_code);

        // But user already saved `newer_uncompiled` to disk
        std::thread::sleep(std::time::Duration::from_millis(50));
        let newer_uncompiled = "fn foo() {}\nfn foo_even_newer() {}\n";
        std::fs::write(&foo_path, newer_uncompiled).unwrap();
        overlay_mgr.update(foo_str, 3, newer_uncompiled.to_string());

        // Shard reload must NOT advance baseline because disk content hash != compiled index hash!
        sync_reloaded_index(&mut ws, &mut overlay_mgr);
        assert!(
            overlay_mgr.get(foo_str).unwrap().is_dirty(),
            "shard reload must NOT advance baseline when disk hash does not match compiled source hash"
        );

        // Case: compiler finishes and publishes shard with exact compiled provenance of newer_uncompiled
        std::thread::sleep(std::time::Duration::from_millis(50));
        let mut writer_foo_v3 = IndexWriter::new("crate_foo", "0.1.0");
        writer_foo_v3.add_definition(RawDefinition {
            def_id: 1,
            kind: DefKind::Fn,
            visibility: 0,
            is_macro_expansion: false,
            name: "foo".to_string(),
            path: "crate_foo::foo".to_string(),
            signature: Some("fn()".to_string()),
            file: foo_path.to_str().unwrap().to_string(),
            start_line: 1,
            start_col: 1,
            end_line: 1,
            end_col: 11,
        });
        let f_foo3 = std::fs::File::create(&foo_ridx).unwrap();
        writer_foo_v3
            .write_to(&mut std::io::BufWriter::new(f_foo3))
            .unwrap();
        write_foo_meta(newer_uncompiled);

        sync_reloaded_index(&mut ws, &mut overlay_mgr);
        assert!(
            !overlay_mgr.get(foo_str).unwrap().is_dirty(),
            "shard reload MUST advance baseline when shard provenance matches disk text"
        );

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    fn make_lsp_msg(val: &Value) -> Vec<u8> {
        let s = val.to_string();
        format!("Content-Length: {}\r\n\r\n{}", s.len(), s).into_bytes()
    }

    fn parse_lsp_responses(bytes: &[u8]) -> Vec<Value> {
        let mut results = Vec::new();
        let mut cur = 0;
        while cur < bytes.len() {
            let slice = &bytes[cur..];
            let Some(header_end) = slice.windows(4).position(|w| w == b"\r\n\r\n") else {
                break;
            };
            let header_str = String::from_utf8_lossy(&slice[..header_end]);
            let mut content_len = 0;
            for line in header_str.lines() {
                if let Some(rest) = line.strip_prefix("Content-Length:") {
                    content_len = rest.trim().parse::<usize>().unwrap_or(0);
                }
            }
            let body_start = cur + header_end + 4;
            let body_end = body_start + content_len;
            if body_end > bytes.len() {
                break;
            }
            if let Ok(val) = serde_json::from_slice::<Value>(&bytes[body_start..body_end]) {
                results.push(val);
            }
            cur = body_end;
        }
        results
    }

    #[test]
    fn test_lsp_server_stream_initialize_and_capabilities() {
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
            "method": "initialized",
            "params": {}
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
        let res = run_lsp_server_stream(std::io::Cursor::new(input), &mut output, None);
        assert!(res.is_ok());

        let responses = parse_lsp_responses(&output);
        assert_eq!(responses.len(), 2);

        // Verify initialize response capabilities
        let init_resp = &responses[0];
        assert_eq!(init_resp["id"], 1);
        let caps = &init_resp["result"]["capabilities"];
        assert_eq!(caps["textDocumentSync"]["change"], 2); // Incremental sync
        assert_eq!(caps["workspace"]["workspaceFolders"]["supported"], true);
        assert_eq!(
            caps["workspace"]["workspaceFolders"]["changeNotifications"],
            true
        );

        // Verify shutdown response
        let shut_resp = &responses[1];
        assert_eq!(shut_resp["id"], 2);
        assert_eq!(shut_resp["result"], Value::Null);
    }

    #[test]
    fn test_lsp_server_stream_request_cancellation() {
        let mut input = Vec::new();
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": { "capabilities": {} }
        })));
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "method": "$/cancelRequest",
            "params": { "id": 100 }
        })));
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 100,
            "method": "textDocument/definition",
            "params": {
                "textDocument": { "uri": "file:///tmp/dummy.rs" },
                "position": { "line": 0, "character": 0 }
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
        let res = run_lsp_server_stream(std::io::Cursor::new(input), &mut output, None);
        assert!(res.is_ok());

        let responses = parse_lsp_responses(&output);
        assert_eq!(responses.len(), 3);

        // Response for request 100 should be error -32800 (Request cancelled)
        let cancel_resp = &responses[1];
        assert_eq!(cancel_resp["id"], 100);
        assert_eq!(cancel_resp["error"]["code"], -32800);
        assert_eq!(cancel_resp["error"]["message"], "Request cancelled");
    }

    #[test]
    fn test_lsp_server_stream_incremental_sync_and_symbols() {
        let mut input = Vec::new();
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": { "capabilities": {} }
        })));
        let doc_uri = "file:///tmp/test_doc.rs";
        let initial_text = "fn main() {\n    let old_symbol = 1;\n}\n";
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": {
                    "uri": doc_uri,
                    "languageId": "rust",
                    "version": 1,
                    "text": initial_text
                }
            }
        })));
        // Incremental change: replace "old_symbol" (line 1, chars 8..18) with "new_symbol"
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": {
                    "uri": doc_uri,
                    "version": 2
                },
                "contentChanges": [
                    {
                        "range": {
                            "start": { "line": 1, "character": 8 },
                            "end": { "line": 1, "character": 18 }
                        },
                        "text": "new_symbol"
                    }
                ]
            }
        })));
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 10,
            "method": "textDocument/documentSymbol",
            "params": {
                "textDocument": { "uri": doc_uri }
            }
        })));
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 11,
            "method": "shutdown",
            "params": null
        })));
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "method": "exit"
        })));

        let mut output = Vec::new();
        let res = run_lsp_server_stream(std::io::Cursor::new(input), &mut output, None);
        assert!(res.is_ok());

        let responses = parse_lsp_responses(&output);
        assert_eq!(responses.len(), 3);

        let sym_resp = &responses[1];
        assert_eq!(sym_resp["id"], 10);
        let symbols = sym_resp["result"].as_array().expect("array of symbols");
        let found = symbols.iter().any(|s| s["name"] == "new_symbol");
        assert!(
            found,
            "documentSymbol must reflect incrementally updated variable name"
        );
    }

    #[test]
    fn test_lsp_server_stream_shutdown_rejects_further_requests() {
        let mut input = Vec::new();
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
            "id": 3,
            "method": "textDocument/documentSymbol",
            "params": {
                "textDocument": { "uri": "file:///tmp/dummy.rs" }
            }
        })));
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "method": "exit"
        })));

        let mut output = Vec::new();
        let res = run_lsp_server_stream(std::io::Cursor::new(input), &mut output, None);
        assert!(res.is_ok());

        let responses = parse_lsp_responses(&output);
        assert_eq!(responses.len(), 3);

        // Response for request 3 must be error -32600 (Server is shutting down)
        let reject_resp = &responses[2];
        assert_eq!(reject_resp["id"], 3);
        assert_eq!(reject_resp["error"]["code"], -32600);
        assert_eq!(reject_resp["error"]["message"], "Server is shutting down");
    }

    #[test]
    fn test_lsp_server_stream_exit_before_shutdown_returns_err() {
        let mut input = Vec::new();
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": { "capabilities": {} }
        })));
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "method": "exit"
        })));

        let mut output = Vec::new();
        let res = run_lsp_server_stream(std::io::Cursor::new(input), &mut output, None);
        assert!(res.is_err());
        assert_eq!(
            res.unwrap_err().to_string(),
            "exit notification received before shutdown"
        );
    }
}
