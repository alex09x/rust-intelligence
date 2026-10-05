//! Lightweight, zero-memory Language Server Protocol (LSP) bridge for Rust Intelligence.
//!
//! Provides instant (<50 µs) textDocument/definition, textDocument/references,
//! textDocument/hover, and workspace/symbol over stdio without running a heavy background daemon.

use ridx_format::{DefKind, WorkspaceIndex};
use serde_json::{json, Value};
use std::io::{self, BufRead, Read, Write};
use std::path::{Path, PathBuf};

fn lsp_symbol_kind(kind: DefKind) -> u32 {
    match kind {
        DefKind::Fn => 12,        // Function
        DefKind::AssocFn => 6,    // Method
        DefKind::Struct => 23,    // Struct
        DefKind::Enum => 10,      // Enum
        DefKind::Variant => 22,   // EnumMember
        DefKind::Trait => 11,     // Interface
        DefKind::Mod => 2,        // Module
        DefKind::Const | DefKind::AssocConst => 14, // Constant
        DefKind::Static => 13,    // Variable
        DefKind::Field => 8,      // Field
        DefKind::TyAlias => 26,   // TypeParameter
        _ => 13,
    }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16) {
                decoded.push(byte);
                i += 3;
                continue;
            }
        }
        decoded.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&decoded).to_string()
}

fn percent_encode_path(s: &str) -> String {
    let mut encoded = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                encoded.push(b as char);
            }
            _ => {
                use std::fmt::Write;
                let _ = write!(encoded, "%{:02X}", b);
            }
        }
    }
    encoded
}

fn uri_to_path(uri: &str) -> String {
    let stripped = uri
        .strip_prefix("file://localhost")
        .or_else(|| uri.strip_prefix("file://"))
        .unwrap_or(uri);
    percent_decode(stripped)
}

fn path_to_uri(file: &str) -> String {
    let abs_path = if file.starts_with('/') {
        file.to_string()
    } else if let Ok(canon) = Path::new(file).canonicalize() {
        canon.to_string_lossy().to_string()
    } else {
        format!("/{}", file)
    };
    format!("file://{}", percent_encode_path(&abs_path))
}

fn char_col_to_utf16_col(file_path: &str, line_1based: u32, col_1based: u32) -> u32 {
    let Ok(content) = std::fs::read_to_string(file_path) else {
        return col_1based.saturating_sub(1);
    };
    if let Some(line) = content.lines().nth(line_1based.saturating_sub(1) as usize) {
        let char_limit = col_1based.saturating_sub(1) as usize;
        let mut utf16_count = 0;
        for (i, c) in line.chars().enumerate() {
            if i >= char_limit {
                break;
            }
            utf16_count += c.len_utf16() as u32;
        }
        utf16_count
    } else {
        col_1based.saturating_sub(1)
    }
}

fn utf16_col_to_char_col(file_path: &str, line_1based: u32, utf16_col_0based: u32) -> u32 {
    let Ok(content) = std::fs::read_to_string(file_path) else {
        return utf16_col_0based + 1;
    };
    if let Some(line) = content.lines().nth(line_1based.saturating_sub(1) as usize) {
        let mut curr_utf16 = 0;
        let mut char_count = 0;
        for c in line.chars() {
            if curr_utf16 >= utf16_col_0based {
                break;
            }
            curr_utf16 += c.len_utf16() as u32;
            char_count += 1;
        }
        char_count + 1
    } else {
        utf16_col_0based + 1
    }
}

fn send_response<W: Write>(writer: &mut W, response: &Value) -> io::Result<()> {
    let body = serde_json::to_string(response)?;
    write!(
        writer,
        "Content-Length: {}\r\n\r\n{}",
        body.len(),
        body
    )?;
    writer.flush()?;
    Ok(())
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

        // Automatically reload indices if files changed on disk
        if let Some(ref mut ws) = workspace {
            let _ = ws.reload_if_modified();
        }

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

                if let Some(id) = id {
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "capabilities": {
                                "textDocumentSync": 0,
                                "positionEncoding": "utf-16",
                                "definitionProvider": true,
                                "referencesProvider": true,
                                "hoverProvider": true,
                                "workspaceSymbolProvider": true
                            },
                            "serverInfo": {
                                "name": "ridx-lsp",
                                "version": "0.1.0"
                            }
                        }
                    });
                    send_response(&mut writer, &resp)?;
                }
            }

            "initialized" => {
                // Client confirmed initialization; no response needed
            }

            "textDocument/didOpen" | "textDocument/didChange" | "textDocument/didClose" => {
                // Documents not held in server memory; queried on-demand from disk index
            }

            "textDocument/didSave" => {
                if let Some(ref mut ws) = workspace {
                    let _ = ws.reload_if_modified();
                }
            }

            "textDocument/definition" => {
                if let Some(id) = id {
                    let mut result = Value::Null;

                    if let Some(ref ws) = workspace {
                        let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
                        let file = uri_to_path(uri);
                        let line_0based = msg["params"]["position"]["line"].as_u64().unwrap_or(0) as u32;
                        let utf16_col_0based =
                            msg["params"]["position"]["character"].as_u64().unwrap_or(0) as u32;
                        let line_1based = line_0based + 1;
                        let col_1based = utf16_col_to_char_col(&file, line_1based, utf16_col_0based);

                        // 1-based indexing in ridx
                        if let Some(def) = ws.find_definition_at(&file, line_1based, col_1based) {
                            let def_uri = path_to_uri(def.file);
                            let start_char = char_col_to_utf16_col(def.file, def.start_line, def.start_col);
                            let end_char = char_col_to_utf16_col(def.file, def.end_line, def.end_col);
                            result = json!({
                                "uri": def_uri,
                                "range": {
                                    "start": {
                                        "line": def.start_line.saturating_sub(1),
                                        "character": start_char
                                    },
                                    "end": {
                                        "line": def.end_line.saturating_sub(1),
                                        "character": end_char
                                    }
                                }
                            });
                        }
                    }

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
                    let mut result = json!([]);

                    if let Some(ref ws) = workspace {
                        let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
                        let file = uri_to_path(uri);
                        let line_0based = msg["params"]["position"]["line"].as_u64().unwrap_or(0) as u32;
                        let utf16_col_0based =
                            msg["params"]["position"]["character"].as_u64().unwrap_or(0) as u32;
                        let line_1based = line_0based + 1;
                        let col_1based = utf16_col_to_char_col(&file, line_1based, utf16_col_0based);
                        let include_decl = msg["params"]["context"]["includeDeclaration"].as_bool().unwrap_or(false);

                        if let Some(loc_refs) =
                            ws.find_references_at(&file, line_1based, col_1based)
                        {
                            let mut locs = Vec::new();
                            if include_decl && !loc_refs.target_file.is_empty() && loc_refs.target_file != "external" {
                                let decl_uri = path_to_uri(loc_refs.target_file);
                                let decl_start_line = loc_refs.target_line.saturating_sub(1);
                                let decl_start_col = char_col_to_utf16_col(
                                    loc_refs.target_file,
                                    loc_refs.target_line,
                                    loc_refs.target_col,
                                );
                                let decl_end_col = decl_start_col + (loc_refs.target_symbol.len() as u32);
                                locs.push(json!({
                                    "uri": decl_uri,
                                    "range": {
                                        "start": {
                                            "line": decl_start_line,
                                            "character": decl_start_col
                                        },
                                        "end": {
                                            "line": decl_start_line,
                                            "character": decl_end_col
                                        }
                                    }
                                }));
                            }

                            for r in loc_refs.references {
                                let span = r.span;
                                let r_uri = path_to_uri(r.file);
                                let start_char = char_col_to_utf16_col(r.file, span.start_line(), span.start_col());
                                let end_char = char_col_to_utf16_col(r.file, span.end_line(), span.end_col());
                                locs.push(json!({
                                    "uri": r_uri,
                                    "range": {
                                        "start": {
                                            "line": span.start_line().saturating_sub(1),
                                            "character": start_char
                                        },
                                        "end": {
                                            "line": span.end_line().saturating_sub(1),
                                            "character": end_char
                                        }
                                    }
                                }));
                            }
                            result = json!(locs);
                        }
                    }

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
                    let mut result = Value::Null;

                    if let Some(ref ws) = workspace {
                        let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
                        let file = uri_to_path(uri);
                        let line_0based = msg["params"]["position"]["line"].as_u64().unwrap_or(0) as u32;
                        let utf16_col_0based =
                            msg["params"]["position"]["character"].as_u64().unwrap_or(0) as u32;
                        let line_1based = line_0based + 1;
                        let col_1based = utf16_col_to_char_col(&file, line_1based, utf16_col_0based);

                        if let Some(hover) = ws.hover_at(&file, line_1based, col_1based) {
                            let mut md = String::new();
                            if let Some(ref def) = hover.definition {
                                md.push_str("```rust\n");
                                if let Some(sig) = def.signature {
                                    md.push_str(sig);
                                } else {
                                    md.push_str(&format!("{:?} {}", def.kind, def.name));
                                }
                                md.push_str("\n```\n\n");
                                md.push_str(&format!(
                                    "*Defined in `{}` at {}:{}*\n\n",
                                    def.file, def.start_line, def.start_col
                                ));
                            }
                            if let Some(ty) = hover.inferred_type {
                                md.push_str(&format!("**Type:** `{ty}`\n"));
                            }

                            if !md.is_empty() {
                                result = json!({
                                    "contents": {
                                        "kind": "markdown",
                                        "value": md.trim()
                                    }
                                });
                            }
                        }
                    }

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
                    let mut result = json!([]);

                    if let Some(ref ws) = workspace {
                        let query = msg["params"]["query"].as_str().unwrap_or("");
                        let symbols = ws.search_symbols(query);
                        let mut list = Vec::new();

                        for s in symbols.iter().take(100) {
                            let span = s.span;
                            let uri = path_to_uri(s.file);
                            let start_char = char_col_to_utf16_col(s.file, span.start_line(), span.start_col());
                            let end_char = char_col_to_utf16_col(s.file, span.end_line(), span.end_col());
                            list.push(json!({
                                "name": s.name,
                                "kind": lsp_symbol_kind(s.kind),
                                "containerName": s.crate_name,
                                "location": {
                                    "uri": uri,
                                    "range": {
                                        "start": {
                                            "line": span.start_line().saturating_sub(1),
                                            "character": start_char
                                        },
                                        "end": {
                                            "line": span.end_line().saturating_sub(1),
                                            "character": end_char
                                        }
                                    }
                                }
                            }));
                        }
                        result = json!(list);
                    }

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
                // Unhandled method, return error or empty response for request
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
