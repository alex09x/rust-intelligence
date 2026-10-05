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
use std::io::{self, BufRead, Read};
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

    let mut workspace: Option<WorkspaceIndex> = match index_path {
        Some(p) => WorkspaceIndex::open(&p).ok(),
        None => WorkspaceIndex::auto_discover().ok(),
    };

    let mut overlay_mgr = crate::overlay::OverlayManager::new();

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
                    // Try to initialize from rootUri or rootPath
                    if let Some(root_uri) = msg["params"]["rootUri"].as_str() {
                        let path = uri_to_path(root_uri);
                        workspace = WorkspaceIndex::open(&path).ok();
                    } else if let Some(root_path) = msg["params"]["rootPath"].as_str() {
                        workspace = WorkspaceIndex::open(root_path).ok();
                    }
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
                                "change": 1, // Full document sync (TextDocumentSyncKind.Full)
                                "save": {
                                    "includeText": true
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
                    if let Some(last) = changes.last() {
                        if let Some(text) = last["text"].as_str() {
                            if !overlay_mgr.update(&file, version, text.to_string()) {
                                overlay_mgr.open_with_provenance(
                                    uri,
                                    &file,
                                    version,
                                    text.to_string(),
                                    false,
                                );
                            }
                        }
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
                return Ok(());
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
}
