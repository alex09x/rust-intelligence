/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Multi-line use item parsing, attribute preservation, and import sorting.

use serde_json::{json, Value};

#[derive(Debug)]
struct UseEntry {
    attrs_and_comments: Vec<String>,
    use_lines: Vec<String>,
    sort_key: String,
}

/// Organizes and sorts import statements in the given document text.
/// Correctly handles multi-line use trees, attributes, and comments.
pub fn organize_imports_action(
    doc_text: &str,
    uri: &str,
    doc_version: Option<i32>,
) -> Option<Value> {
    let lines: Vec<&str> = doc_text.lines().collect();
    if lines.is_empty() {
        return None;
    }

    let mut entries: Vec<UseEntry> = Vec::new();
    let mut current_attrs: Vec<String> = Vec::new();
    let mut block_start_line = None;
    let mut block_end_line = None;
    let mut in_import_block = false;

    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();

        if trimmed.starts_with("#[") || trimmed.starts_with("//") {
            if !in_import_block {
                let following = lines
                    .iter()
                    .skip(i + 1)
                    .take_while(|l| {
                        let t = l.trim();
                        t.starts_with("#[") || t.starts_with("//") || t.is_empty()
                    })
                    .count();
                if let Some(nl) = lines.get(i + 1 + following).map(|l| l.trim()) {
                    if nl.starts_with("use ") || nl.starts_with("pub use ") {
                        block_start_line = Some(i);
                        in_import_block = true;
                    }
                }
            }
            if in_import_block {
                current_attrs.push(line.to_string());
            }
            i += 1;
        } else if trimmed.starts_with("use ") || trimmed.starts_with("pub use ") {
            if block_start_line.is_none() {
                block_start_line = Some(i);
            }
            in_import_block = true;

            let mut use_lines = vec![line.to_string()];
            let mut end_line_idx = i;
            while !use_lines.last().is_some_and(|l| l.contains(';'))
                && end_line_idx + 1 < lines.len()
            {
                end_line_idx += 1;
                use_lines.push(lines[end_line_idx].to_string());
            }

            if !use_lines.last().is_some_and(|l| l.contains(';')) {
                // Unclosed use item: preserve unchanged without edits
                return None;
            }

            i = end_line_idx;
            block_end_line = Some(i);
            let sort_key = use_lines[0].trim().to_string();
            entries.push(UseEntry {
                attrs_and_comments: std::mem::take(&mut current_attrs),
                use_lines,
                sort_key,
            });
            i += 1;
        } else if in_import_block {
            if trimmed.is_empty() {
                let next_non_empty = lines
                    .iter()
                    .skip(i + 1)
                    .find(|l| !l.trim().is_empty())
                    .map(|l| l.trim());
                if let Some(nl) = next_non_empty {
                    if nl.starts_with("use ") || nl.starts_with("pub use ") || nl.starts_with("#[")
                    {
                        i += 1;
                        continue;
                    }
                }
            }
            break;
        } else {
            i += 1;
        }
    }

    if let (Some(start_idx), Some(end_idx)) = (block_start_line, block_end_line) {
        if !entries.is_empty() {
            entries.sort_by(|a, b| a.sort_key.cmp(&b.sort_key));
            entries.dedup_by(|a, b| {
                a.sort_key == b.sort_key
                    && a.attrs_and_comments == b.attrs_and_comments
                    && a.use_lines == b.use_lines
            });
            let mut new_text_lines = Vec::new();
            for e in &entries {
                for a in &e.attrs_and_comments {
                    new_text_lines.push(a.clone());
                }
                for u in &e.use_lines {
                    new_text_lines.push(u.clone());
                }
            }
            let new_text = new_text_lines.join("\n");
            let end_char = lines[end_idx].encode_utf16().count() as u32;

            let edits_vec = vec![json!({
                "range": {
                    "start": { "line": start_idx as u32, "character": 0 },
                    "end": { "line": end_idx as u32, "character": end_char }
                },
                "newText": new_text
            })];

            let mut doc_changes = Vec::new();
            if let Some(ver) = doc_version {
                doc_changes.push(json!({
                    "textDocument": {
                        "uri": uri,
                        "version": ver
                    },
                    "edits": edits_vec
                }));
            }

            let edit = if doc_changes.is_empty() {
                json!({
                    "changes": {
                        uri: edits_vec
                    }
                })
            } else {
                json!({
                    "changes": {
                        uri: edits_vec
                    },
                    "documentChanges": doc_changes
                })
            };

            return Some(json!({
                "title": "Organize Imports",
                "kind": "source.organizeImports",
                "data": {
                    "uri": uri,
                    "version": doc_version,
                    "kind": "source.organizeImports"
                },
                "edit": edit
            }));
        }
    }

    None
}
