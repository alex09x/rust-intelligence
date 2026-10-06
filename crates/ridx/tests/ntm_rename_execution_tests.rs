/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use ridx::lsp::editing::rename::handle_rename;
use ridx::lsp::protocol::PositionConverter;
use ridx::overlay::OverlayManager;
use ridx_format::{IndexWriter, RawDefinition, RawReference, WorkspaceIndex};
use serde_json::json;
use std::fs;

fn utf16_col_to_char_slice(line: &str, utf16_col: usize) -> (String, String) {
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
    (
        line[..split_byte].to_string(),
        line[split_byte..].to_string(),
    )
}

fn apply_text_edits(content: &str, edits: &[serde_json::Value]) -> String {
    let mut parsed: Vec<(u32, u32, u32, u32, String)> = edits
        .iter()
        .map(|e| {
            let r = &e["range"];
            (
                r["start"]["line"].as_u64().unwrap() as u32,
                r["start"]["character"].as_u64().unwrap() as u32,
                r["end"]["line"].as_u64().unwrap() as u32,
                r["end"]["character"].as_u64().unwrap() as u32,
                e["newText"].as_str().unwrap().to_string(),
            )
        })
        .collect();

    parsed.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));

    let mut lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
    if content.ends_with('\n') {
        lines.push(String::new());
    }

    for (s_line, s_char, e_line, e_char, new_text) in parsed {
        let s_l = s_line as usize;
        let e_l = e_line as usize;
        let s_c = s_char as usize;
        let e_c = e_char as usize;

        if s_l == e_l {
            let line = &lines[s_l];
            let (before, _) = utf16_col_to_char_slice(line, s_c);
            let (_, after) = utf16_col_to_char_slice(line, e_c);
            let replaced = format!("{}{}{}", before, new_text, after);
            let repl_lines: Vec<String> = replaced.split('\n').map(|s| s.to_string()).collect();
            lines.splice(s_l..=s_l, repl_lines);
        } else {
            let (start_prefix, _) = utf16_col_to_char_slice(&lines[s_l], s_c);
            let (_, end_suffix) = utf16_col_to_char_slice(&lines[e_l], e_c);
            let combined = format!("{}{}{}", start_prefix, new_text, end_suffix);
            let repl_lines: Vec<String> = combined.split('\n').map(|s| s.to_string()).collect();
            lines.splice(s_l..=e_l, repl_lines);
        }
    }

    let mut result = lines.join("\n");
    if content.ends_with('\n') && !result.ends_with('\n') {
        result.push('\n');
    }
    result
}

#[test]
fn test_refactor_rename_local_variable_execution() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = "file:///workspace/src/counter.rs";
    let file_path = "/workspace/src/counter.rs";

    let original_code = concat!(
        "fn process_data(items: &[i32]) -> i32 {\n",
        "    let mut count = 0;\n",
        "    for _x in items {\n",
        "        count += 1;\n",
        "    }\n",
        "    let text = \"count in string\";\n",
        "    // count in comments\n",
        "    count\n",
        "}\n",
    );

    overlay_mgr.open_document(uri, file_path, 1, original_code.to_string());

    let rename_req = json!({
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": 1, "character": 13 },
            "newName": "total_items"
        }
    });

    let res = handle_rename(&rename_req, None, &overlay_mgr, &mut conv);
    let edits = res["changes"][uri].as_array().expect("edits array");
    assert!(!edits.is_empty(), "rename must return edits");

    let transformed = apply_text_edits(original_code, edits);

    assert!(transformed.contains("let mut total_items = 0;"));
    assert!(transformed.contains("total_items += 1;"));
    assert!(transformed.contains("    total_items\n"));
    assert!(transformed.contains("\"count in string\""));
    assert!(transformed.contains("// count in comments"));
    assert!(!transformed.contains("let mut count = 0;"));
}

#[test]
fn test_refactor_cross_file_workspace_rename_execution() {
    let temp_dir = std::env::temp_dir().join(format!("ridx_cross_rename_{}", std::process::id()));
    let _ = fs::create_dir_all(temp_dir.join("src"));

    let def_file = temp_dir.join("src/lib.rs");
    let def_code = "pub fn add_values(x: i32, y: i32) -> i32 {\n    x + y\n}\n";
    let _ = fs::write(&def_file, def_code);

    let caller_file = temp_dir.join("src/caller.rs");
    let caller_code = "pub fn call() -> i32 {\n    test_pkg::add_values(1, 2)\n}\n";
    let _ = fs::write(&caller_file, caller_code);

    let def_file_str = def_file.to_str().unwrap().to_string();
    let caller_file_str = caller_file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("test_pkg", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: ridx_format::DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        file: def_file_str.clone(),
        name: "add_values".to_string(),
        start_line: 1,
        start_col: 8,
        end_line: 1,
        end_col: 18,
        path: "test_pkg::add_values".to_string(),
        signature: Some("fn add_values(x: i32, y: i32) -> i32".to_string()),
    });

    writer.add_reference(RawReference {
        file: def_file_str.clone(),
        start_line: 1,
        start_col: 8,
        end_line: 1,
        end_col: 18,
        target_def_id: Some(1),
        target_path: "test_pkg::add_values".to_string(),
        target_unit: None,
        target_file: Some(def_file_str.clone()),
        target_line: 1,
        target_col: 8,
        kind: ridx_format::DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });

    writer.add_reference(RawReference {
        file: caller_file_str.clone(),
        start_line: 2,
        start_col: 15,
        end_line: 2,
        end_col: 25,
        target_def_id: Some(1),
        target_path: "test_pkg::add_values".to_string(),
        target_unit: None,
        target_file: Some(def_file_str.clone()),
        target_line: 1,
        target_col: 8,
        kind: ridx_format::DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });

    let index_path = temp_dir.join("index.ridx");
    let mut f = fs::File::create(&index_path).unwrap();
    writer.write_to(&mut f).unwrap();

    let ws = WorkspaceIndex::open(&index_path).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let def_uri = format!("file://{}", def_file_str);
    let caller_uri = format!("file://{}", caller_file_str);

    let rename_req = json!({
        "params": {
            "textDocument": { "uri": &def_uri },
            "position": { "line": 0, "character": 8 },
            "newName": "sum_values"
        }
    });

    let res = handle_rename(&rename_req, Some(&ws), &overlay_mgr, &mut conv);
    let changes = res["changes"].as_object().expect("changes object");

    let def_edits = changes
        .get(&def_uri)
        .expect("edits for lib.rs")
        .as_array()
        .unwrap();
    let caller_edits = changes
        .get(&caller_uri)
        .expect("edits for caller.rs")
        .as_array()
        .unwrap();

    let new_def_code = apply_text_edits(def_code, def_edits);
    let new_caller_code = apply_text_edits(caller_code, caller_edits);

    assert_eq!(
        new_def_code,
        "pub fn sum_values(x: i32, y: i32) -> i32 {\n    x + y\n}\n"
    );
    assert_eq!(
        new_caller_code,
        "pub fn call() -> i32 {\n    test_pkg::sum_values(1, 2)\n}\n"
    );

    let _ = fs::remove_dir_all(&temp_dir);
}
