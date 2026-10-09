/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! LSP JSON-RPC framing with strict error detection.

use serde_json::{json, Value};

/// Encodes a JSON-RPC value with standard LSP `Content-Length: ...\r\n\r\n` headers.
pub fn make_lsp_msg(val: &Value) -> Vec<u8> {
    let s = val.to_string();
    format!("Content-Length: {}\r\n\r\n{}", s.len(), s).into_bytes()
}

/// Strictly parses all LSP JSON-RPC messages from a byte stream.
///
/// Returns an error with diagnostic hexdump if:
/// - A header is missing `Content-Length`;
/// - The byte stream is truncated;
/// - The body is malformed or invalid JSON;
/// - There are unconsumed trailing non-whitespace bytes.
pub fn parse_lsp_responses_strict(bytes: &[u8]) -> Result<Vec<Value>, String> {
    let mut results = Vec::new();
    let mut cur = 0;

    while cur < bytes.len() {
        let slice = &bytes[cur..];

        // Skip any leading whitespace/newlines between messages
        if slice[0] == b'\r' || slice[0] == b'\n' || slice[0] == b' ' {
            cur += 1;
            continue;
        }

        let Some(header_end) = slice.windows(4).position(|w| w == b"\r\n\r\n") else {
            return Err(format!(
                "Incomplete LSP header at offset {}: missing \\r\\n\\r\\n. Hexdump:\n{}",
                cur,
                hexdump(slice)
            ));
        };

        let header_str = String::from_utf8_lossy(&slice[..header_end]);
        let mut content_len = None;
        for line in header_str.lines() {
            if let Some(rest) = line.strip_prefix("Content-Length:") {
                if let Ok(val) = rest.trim().parse::<usize>() {
                    content_len = Some(val);
                }
            }
        }

        let Some(len) = content_len else {
            return Err(format!(
                "Malformed LSP header at offset {}: missing Content-Length header. Header:\n{}",
                cur, header_str
            ));
        };

        let body_start = cur + header_end + 4;
        let body_end = body_start + len;

        if body_end > bytes.len() {
            return Err(format!(
                "Truncated LSP body at offset {}: expected {} bytes, but only {} bytes available. Stream tail hexdump:\n{}",
                body_start,
                len,
                bytes.len() - body_start,
                hexdump(&bytes[body_start..])
            ));
        }

        let body_bytes = &bytes[body_start..body_end];
        match serde_json::from_slice::<Value>(body_bytes) {
            Ok(val) => results.push(val),
            Err(err) => {
                return Err(format!(
                    "Invalid JSON body at offset {}..{} ({}): {}\nBody text:\n{}",
                    body_start,
                    body_end,
                    err,
                    hexdump(body_bytes),
                    String::from_utf8_lossy(body_bytes)
                ));
            }
        }

        cur = body_end;
    }

    Ok(results)
}

/// Parses all LSP responses, panicking with details if framing or JSON is invalid.
pub fn parse_lsp_responses(bytes: &[u8]) -> Vec<Value> {
    parse_lsp_responses_strict(bytes).unwrap_or_else(|e| panic!("{}", e))
}

fn hexdump(slice: &[u8]) -> String {
    let limit = slice.len().min(128);
    let mut out = String::new();
    for (i, chunk) in slice[..limit].chunks(16).enumerate() {
        out.push_str(&format!("{:04x}: ", i * 16));
        for b in chunk {
            out.push_str(&format!("{:02x} ", b));
        }
        out.push('\n');
    }
    if slice.len() > limit {
        out.push_str(&format!("... ({} more bytes)\n", slice.len() - limit));
    }
    out
}

/// Test session builder that compiles LSP requests and notifications into in-memory bytes.
#[derive(Debug, Default)]
pub struct TestLspSession {
    bytes: Vec<u8>,
    req_id: u64,
}

impl TestLspSession {
    pub fn new() -> Self {
        Self {
            bytes: Vec::new(),
            req_id: 1,
        }
    }

    /// Appends standard initialize with default capabilities.
    pub fn initialize(&mut self, root_uri: &str) -> u64 {
        self.initialize_with_encodings(root_uri, &[])
    }

    /// Appends initialize with explicit position encodings (e.g. `["utf-8", "utf-16"]`).
    pub fn initialize_with_encodings(&mut self, root_uri: &str, encodings: &[&str]) -> u64 {
        let mut caps = json!({});
        if !encodings.is_empty() {
            caps["general"] = json!({
                "positionEncodings": encodings
            });
        }
        let id = self.send_request(
            "initialize",
            json!({
                "processId": null,
                "rootUri": root_uri,
                "capabilities": caps,
            }),
        );
        self.send_notification("initialized", json!({}));
        id
    }

    /// Appends a request and returns its auto-assigned ID.
    pub fn send_request(&mut self, method: &str, params: Value) -> u64 {
        let id = self.req_id;
        self.req_id += 1;
        let body = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        self.bytes.extend(make_lsp_msg(&body));
        id
    }

    /// Appends a notification (no ID).
    pub fn send_notification(&mut self, method: &str, params: Value) {
        let body = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        self.bytes.extend(make_lsp_msg(&body));
    }

    /// Appends textDocument/didOpen notification.
    pub fn did_open(&mut self, uri: &str, text: &str) {
        self.send_notification(
            "textDocument/didOpen",
            json!({
                "textDocument": {
                    "uri": uri,
                    "languageId": "rust",
                    "version": 1,
                    "text": text,
                }
            }),
        );
    }

    /// Returns the accumulated input bytes.
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strict_framing_valid() {
        let val = json!({"status": "ok"});
        let bytes = make_lsp_msg(&val);
        let parsed = parse_lsp_responses_strict(&bytes).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0]["status"], "ok");
    }

    #[test]
    fn test_strict_framing_truncated() {
        let val = json!({"status": "ok"});
        let mut bytes = make_lsp_msg(&val);
        bytes.truncate(bytes.len() - 3); // cut body short
        let res = parse_lsp_responses_strict(&bytes);
        assert!(res.is_err(), "Must report truncated body error");
        assert!(res.unwrap_err().contains("Truncated LSP body"));
    }

    #[test]
    fn test_session_position_encodings() {
        let mut session = TestLspSession::new();
        session.initialize_with_encodings("file:///test", &["utf-8", "utf-16"]);
        let bytes = session.into_bytes();
        let msgs = parse_lsp_responses(&bytes);
        assert_eq!(
            msgs[0]["params"]["capabilities"]["general"]["positionEncodings"],
            json!(["utf-8", "utf-16"])
        );
    }
}
