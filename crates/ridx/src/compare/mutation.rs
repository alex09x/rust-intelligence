/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Cross-file mutation testing harness for LSP servers.
//! Simulates editor edits (didOpen, didChange, didClose) to verify incremental recomputation.

use super::transport::send_lsp_msg;
use super::types::ScenarioMutation;
use std::collections::HashSet;
use std::io::Write;
use std::path::Path;

#[derive(Default)]
pub struct MutationManager {
    opened_uris: HashSet<String>,
}

impl MutationManager {
    pub fn new() -> Self {
        Self {
            opened_uris: HashSet::new(),
        }
    }

    pub fn apply<W: Write>(
        &mut self,
        writer: &mut W,
        mutation: &ScenarioMutation,
        root: &Path,
    ) -> std::io::Result<()> {
        let file_path = if Path::new(&mutation.file).is_absolute() {
            Path::new(&mutation.file).to_path_buf()
        } else {
            root.join(&mutation.file)
        };

        let uri = format!("file://{}", file_path.display());
        let original_content = std::fs::read_to_string(&file_path).unwrap_or_default();

        if self.opened_uris.insert(uri.clone()) {
            let open_msg = serde_json::json!({
                "jsonrpc": "2.0",
                "method": "textDocument/didOpen",
                "params": {
                    "textDocument": {
                        "uri": uri,
                        "languageId": "rust",
                        "version": 1,
                        "text": original_content
                    }
                }
            });
            send_lsp_msg(writer, &open_msg)?;
        }

        // Apply mutation: prepend new lines / text
        let mutated_content = format!("{}{}", mutation.text, original_content);
        let change_msg = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": {
                    "uri": uri,
                    "version": 2
                },
                "contentChanges": [
                    { "text": mutated_content }
                ]
            }
        });
        send_lsp_msg(writer, &change_msg)?;
        Ok(())
    }

    pub fn revert<W: Write>(
        &mut self,
        writer: &mut W,
        mutation: &ScenarioMutation,
        root: &Path,
    ) -> std::io::Result<()> {
        let file_path = if Path::new(&mutation.file).is_absolute() {
            Path::new(&mutation.file).to_path_buf()
        } else {
            root.join(&mutation.file)
        };

        let uri = format!("file://{}", file_path.display());
        if self.opened_uris.remove(&uri) {
            let close_msg = serde_json::json!({
                "jsonrpc": "2.0",
                "method": "textDocument/didClose",
                "params": {
                    "textDocument": { "uri": uri }
                }
            });
            send_lsp_msg(writer, &close_msg)?;
        }
        Ok(())
    }
}
