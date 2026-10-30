/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Text and workspace edit application with strict LSP 3.17 validation.
//!
//! Preserves CRLF line endings, enforces same-position insertion order,
//! and panics on overlapping edits or unknown document URIs.

use serde_json::Value;
use std::collections::HashMap;

/// Splits a line into (before, after) slices at a 0-indexed UTF-16 column offset.
pub fn utf16_col_to_char_slice(line: &str, utf16_col: usize) -> (String, String) {
    let mut utf16_count = 0;
    let mut split_byte = line.len();
    for (b_idx, c) in line.char_indices() {
        if utf16_count >= utf16_col {
            split_byte = b_idx;
            break;
        }
        utf16_count += c.len_utf16();
    }
    if utf16_count < utf16_col {
        split_byte = line.len();
    }
    (line[..split_byte].to_string(), line[split_byte..].to_string())
}

/// Converts an LSP Position (0-indexed line and UTF-16 character) to an exact byte offset.
pub fn position_to_byte_offset(content: &str, line: u32, character_utf16: u32) -> usize {
    let mut cur_line: u32 = 0;
    let mut cur_col_utf16: u32 = 0;
    let mut target_line_reached = false;

    let chars: Vec<(usize, char)> = content.char_indices().collect();

    for &(byte_idx, ch) in &chars {
        if cur_line == line {
            target_line_reached = true;
            if cur_col_utf16 == character_utf16 {
                return byte_idx;
            }
            if cur_col_utf16 > character_utf16 {
                panic!("position_to_byte_offset: character {} lands inside a surrogate pair (multi-code-unit character) on line {}", character_utf16, line);
            }
            if ch == '\r' || ch == '\n' {
                return byte_idx;
            }
        }

        if ch == '\n' {
            if cur_line == line {
                return byte_idx;
            }
            cur_line += 1;
            cur_col_utf16 = 0;
        } else if ch != '\r' && cur_line == line {
            cur_col_utf16 += ch.len_utf16() as u32;
        }
    }

    if cur_line == line || (cur_line == line + 1 && target_line_reached) {
        if cur_line == line && cur_col_utf16 > character_utf16 {
            panic!("position_to_byte_offset: character {} lands inside a surrogate pair (multi-code-unit character) on line {}", character_utf16, line);
        }
        content.len()
    } else {
        panic!("position_to_byte_offset out of bounds: line {} requested, but file has {} lines", line, cur_line + 1);
    }
}

/// Applies LSP `TextEdit`s to a document string, strictly following LSP 3.17.
///
/// Guarantees:
/// - Preserves CRLF (`\r\n`) and LF line endings without silent conversion;
/// - Enforces array order for same-position insertions (`[A, B]` -> `AB`);
/// - Panics on overlapping text edits.
pub fn apply_text_edits(content: &str, edits: &[Value]) -> String {
    if edits.is_empty() {
        return content.to_string();
    }

    // Step 1: Parse edits into byte offsets with original index
    let mut parsed: Vec<(usize, usize, String, usize)> = edits
        .iter()
        .enumerate()
        .map(|(idx, e)| {
            let r = &e["range"];
            let s_line = r["start"]["line"].as_u64().expect("start.line") as u32;
            let s_char = r["start"]["character"].as_u64().expect("start.character") as u32;
            let e_line = r["end"]["line"].as_u64().expect("end.line") as u32;
            let e_char = r["end"]["character"].as_u64().expect("end.character") as u32;
            let new_text = e["newText"].as_str().expect("newText").to_string();

            let s_byte = position_to_byte_offset(content, s_line, s_char);
            let e_byte = position_to_byte_offset(content, e_line, e_char);
            assert!(s_byte <= e_byte, "TextEdit start position ({}) is after end position ({})", s_byte, e_byte);
            (s_byte, e_byte, new_text, idx)
        })
        .collect();

    // Step 2: Validate against overlapping edits (sort by start asc, end asc)
    let mut for_validation = parsed.clone();
    for_validation.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    for window in for_validation.windows(2) {
        let (s1, e1, _, idx1) = &window[0];
        let (s2, _, _, idx2) = &window[1];
        if *s2 < *e1 {
            panic!("LSP 3.17 Violation: Overlapping TextEdits disallowed! Edit {} (bytes {}..{}) overlaps with edit {} (byte start {})", idx1, s1, e1, idx2, s2);
        }
    }

    // Step 3: Sort in reverse order for safe application from bottom to top.
    // For same start position, sort larger ranges (end desc) first so that
    // text shifts do not invalidate the coordinates of zero-width inserts.
    // For identical spans, sort by original index desc to preserve array insertion order.
    parsed.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)).then_with(|| b.3.cmp(&a.3)));

    let mut result = content.to_string();
    for (s_byte, e_byte, new_text, _) in parsed {
        result.replace_range(s_byte..e_byte, &new_text);
    }

    result
}

/// Applies an LSP `WorkspaceEdit` strictly across a map of file URI -> content.
pub fn apply_workspace_edit(files: &HashMap<String, String>, edit: &Value) -> HashMap<String, String> {
    apply_workspace_edit_with_versions(files, edit, None)
}

/// Applies an LSP `WorkspaceEdit` strictly, validating document versions if provided.
pub fn apply_workspace_edit_with_versions(files: &HashMap<String, String>, edit: &Value, expected_versions: Option<&HashMap<String, i64>>) -> HashMap<String, String> {
    let mut updated = files.clone();

    // LSP 3.17: If documentChanges is present, it must be preferred over changes.
    if let Some(doc_changes) = edit.get("documentChanges").and_then(|c| c.as_array()) {
        for doc in doc_changes {
            if let Some(kind) = doc.get("kind").and_then(|k| k.as_str()) {
                match kind {
                    "create" => {
                        let uri = doc["uri"].as_str().expect("create.uri");
                        updated.insert(uri.to_string(), String::new());
                    }
                    "rename" => {
                        let old_uri = doc["oldUri"].as_str().expect("rename.oldUri");
                        let new_uri = doc["newUri"].as_str().expect("rename.newUri");
                        let content = updated.remove(old_uri).unwrap_or_else(|| panic!("apply_workspace_edit: rename oldUri '{}' not found", old_uri));
                        updated.insert(new_uri.to_string(), content);
                    }
                    "delete" => {
                        let uri = doc["uri"].as_str().expect("delete.uri");
                        updated.remove(uri).unwrap_or_else(|| panic!("apply_workspace_edit: delete uri '{}' not found", uri));
                    }
                    other => panic!("apply_workspace_edit: unsupported documentChange kind '{}'", other),
                }
            } else if let Some(text_doc) = doc.get("textDocument") {
                let uri = text_doc["uri"].as_str().expect("textDocument.uri");
                if let Some(exp_map) = expected_versions {
                    if let Some(exp_ver) = exp_map.get(uri) {
                        let actual_ver = text_doc.get("version").and_then(|v| v.as_i64());
                        if actual_ver != Some(*exp_ver) {
                            panic!("apply_workspace_edit: version mismatch for URI '{}': expected {:?}, got {:?}", uri, exp_ver, actual_ver);
                        }
                    }
                }
                let edits_arr = doc["edits"].as_array().expect("documentChanges.edits");
                let content = updated.get(uri).unwrap_or_else(|| panic!("apply_workspace_edit: URI '{}' not found in workspace (available: {:?})", uri, files.keys()));
                let new_content = apply_text_edits(content, edits_arr);
                updated.insert(uri.to_string(), new_content);
            } else {
                panic!("apply_workspace_edit: malformed documentChange item: {:?}", doc);
            }
        }
    } else if let Some(changes) = edit.get("changes").and_then(|c| c.as_object()) {
        for (uri, edits) in changes {
            let edits_arr = edits.as_array().expect("changes[uri] must be an array");
            let content = updated.get(uri).unwrap_or_else(|| panic!("apply_workspace_edit: URI '{}' not found in workspace (available: {:?})", uri, files.keys()));
            let new_content = apply_text_edits(content, edits_arr);
            updated.insert(uri.clone(), new_content);
        }
    }

    updated
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_crlf_preservation_in_edits() {
        let text = "fn foo() {\r\n    let x = 1;\r\n}\r\n";
        let edits = vec![json!({
            "range": {
                "start": { "line": 1, "character": 8 },
                "end": { "line": 1, "character": 9 }
            },
            "newText": "renamed_x"
        })];
        let result = apply_text_edits(text, &edits);
        assert!(result.contains("\r\n"), "Must preserve CRLF line endings");
        assert_eq!(result, "fn foo() {\r\n    let renamed_x = 1;\r\n}\r\n");
    }

    #[test]
    fn test_crlf_whole_line_replacement_preserves_crlf() {
        let text = "abc\r\nx";
        let edits = vec![json!({
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": { "line": 0, "character": u32::MAX }
            },
            "newText": "replaced"
        })];
        let result = apply_text_edits(text, &edits);
        assert_eq!(result, "replaced\r\nx");
    }

    #[test]
    #[should_panic(expected = "lands inside a surrogate pair")]
    fn test_surrogate_pair_split_rejected() {
        let text = "🦀x";
        let edits = vec![json!({
            "range": {
                "start": { "line": 0, "character": 1 },
                "end": { "line": 0, "character": 2 }
            },
            "newText": "y"
        })];
        apply_text_edits(text, &edits);
    }

    #[test]
    fn test_overlap_insert_at_replace_boundary() {
        let text = "abcdef";
        let edits1 = vec![
            json!({ "range": { "start": { "line": 0, "character": 2 }, "end": { "line": 0, "character": 4 } }, "newText": "XX" }),
            json!({ "range": { "start": { "line": 0, "character": 2 }, "end": { "line": 0, "character": 2 } }, "newText": "IN" }),
        ];
        let edits2 = vec![
            json!({ "range": { "start": { "line": 0, "character": 2 }, "end": { "line": 0, "character": 2 } }, "newText": "IN" }),
            json!({ "range": { "start": { "line": 0, "character": 2 }, "end": { "line": 0, "character": 4 } }, "newText": "XX" }),
        ];
        assert_eq!(apply_text_edits(text, &edits1), "abINXXef");
        assert_eq!(apply_text_edits(text, &edits2), "abINXXef");
    }

    #[test]
    fn test_same_position_insert_order() {
        let text = "world\n";
        let edits = vec![
            json!({
                "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } },
                "newText": "A_"
            }),
            json!({
                "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } },
                "newText": "B_"
            }),
        ];
        let result = apply_text_edits(text, &edits);
        assert_eq!(result, "A_B_world\n");
    }

    #[test]
    #[should_panic(expected = "Overlapping TextEdits disallowed")]
    fn test_overlapping_edits_rejected() {
        let text = "abcdef";
        let edits = vec![
            json!({
                "range": { "start": { "line": 0, "character": 1 }, "end": { "line": 0, "character": 4 } },
                "newText": "X"
            }),
            json!({
                "range": { "start": { "line": 0, "character": 3 }, "end": { "line": 0, "character": 5 } },
                "newText": "Y"
            }),
        ];
        apply_text_edits(text, &edits);
    }

    #[test]
    #[should_panic(expected = "version mismatch")]
    fn test_workspace_edit_version_mismatch_rejected() {
        let mut files = HashMap::new();
        files.insert("file:///lib.rs".to_string(), "let x = 1;".to_string());
        let edit = json!({
            "documentChanges": [{
                "textDocument": { "uri": "file:///lib.rs", "version": 1 },
                "edits": []
            }]
        });
        let mut expected = HashMap::new();
        expected.insert("file:///lib.rs".to_string(), 2);
        apply_workspace_edit_with_versions(&files, &edit, Some(&expected));
    }

    #[test]
    #[should_panic(expected = "not found in workspace")]
    fn test_workspace_edit_missing_uri_rejected() {
        let files = HashMap::new();
        let edit = json!({
            "changes": {
                "file:///non_existent.rs": []
            }
        });
        apply_workspace_edit(&files, &edit);
    }
}
