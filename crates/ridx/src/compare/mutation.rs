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
use super::types::{ExternalLspResult, FileMutation, ScenarioMutation};
use std::collections::HashSet;
use std::io::Write;
use std::path::Path;

#[derive(Default)]
pub struct MutationManager {
    opened_documents: HashSet<String>,
}

impl MutationManager {
    pub fn new() -> Self {
        Self { opened_documents: HashSet::new() }
    }

    pub fn apply<W: Write>(&mut self, writer: &mut W, mutation: &ScenarioMutation, root: &Path) -> std::io::Result<()> {
        let entries: Vec<FileMutation> = if !mutation.file_mutations.is_empty() {
            mutation.file_mutations.clone()
        } else if !mutation.file.is_empty() {
            vec![FileMutation { file: mutation.file.clone(), text: mutation.text.clone(), is_append: false, replace_target: None, replace_with: None }]
        } else {
            Vec::new()
        };

        for entry in entries {
            let file_path = if Path::new(&entry.file).is_absolute() { Path::new(&entry.file).to_path_buf() } else { root.join(&entry.file) };

            let uri = format!("file://{}", file_path.display());
            let original_content = std::fs::read_to_string(&file_path).unwrap_or_default();

            if self.opened_documents.insert(uri.clone()) {
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

            let mutated_content = if let (Some(tgt), Some(rep)) = (&entry.replace_target, &entry.replace_with) {
                original_content.replacen(tgt, rep, 1)
            } else if entry.is_append {
                format!("{}\n{}", original_content, entry.text)
            } else {
                format!("{}{}", entry.text, original_content)
            };

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
        }
        Ok(())
    }

    pub fn revert<W: Write>(&mut self, writer: &mut W) -> std::io::Result<()> {
        for uri in self.opened_documents.drain() {
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

pub fn verify_mutation(res: Option<&ExternalLspResult>, mutation: &ScenarioMutation) -> bool {
    if mutation.expect_not_found || mutation.kind == "rename_stale" || mutation.kind.starts_with("out_of_scope") {
        // Must NOT find old/unresolved symbol. Return true if server returned not found / failed / empty count
        match res {
            None => true,
            Some(r) => !r.found || r.count == 0 || r.span.contains("failed") || r.span.is_empty(),
        }
    } else if mutation.kind == "new_symbol" {
        match res {
            Some(r) => {
                if !r.found || r.count == 0 {
                    return false;
                }
                if let Some(exp_f) = &mutation.expected_target_file {
                    r.span.contains(exp_f)
                } else {
                    true
                }
            }
            None => false,
        }
    } else {
        // line_shift or standard mutation
        match res {
            Some(r) => r.found && r.count > 0,
            None => false,
        }
    }
}
