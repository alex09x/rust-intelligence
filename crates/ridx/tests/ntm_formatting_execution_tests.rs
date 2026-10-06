/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use ridx::lsp::editing::format::{handle_formatting, handle_range_formatting};
use ridx::overlay::OverlayManager;
use serde_json::json;

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
fn test_refactor_formatting_and_range_formatting_execution() {
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///workspace/src/messy.rs";
    let file_path = "/workspace/src/messy.rs";

    let unformatted = concat!(
        "fn first( x: i32 ) -> i32 { x+1 }\n",
        "\n",
        "fn second(y:i32)->i32{\n",
        "let z=y*2;\n",
        "z\n",
        "}\n",
    );

    overlay_mgr.open_document(uri, file_path, 1, unformatted.to_string());

    let rustfmt_available = std::process::Command::new("rustfmt")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !rustfmt_available {
        eprintln!("Skipping formatting execution test: rustfmt binary is not available in PATH");
        return;
    }

    let fmt_req = json!({
        "params": {
            "textDocument": { "uri": uri },
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });
    let fmt_edits = handle_formatting(&fmt_req, &overlay_mgr);
    let edits_arr = fmt_edits.as_array().expect("formatting edits array");
    assert!(
        !edits_arr.is_empty(),
        "Full formatting MUST return edits for unformatted code when rustfmt is available"
    );
    let formatted = apply_text_edits(unformatted, edits_arr);
    assert_ne!(formatted, unformatted);
    assert!(formatted.contains("fn first(x: i32) -> i32 {"));
    assert!(formatted.contains("fn second(y: i32) -> i32 {"));

    let range_req = json!({
        "params": {
            "textDocument": { "uri": uri },
            "range": {
                "start": { "line": 2, "character": 0 },
                "end": { "line": 5, "character": 1 }
            },
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });
    let range_edits = handle_range_formatting(&range_req, &overlay_mgr);
    let r_arr = range_edits.as_array().expect("range edits array");
    assert!(
        !r_arr.is_empty(),
        "Range formatting MUST return edits for unformatted second function when rustfmt is available"
    );
    let partially_formatted = apply_text_edits(unformatted, r_arr);
    assert_ne!(partially_formatted, unformatted);
    assert!(partially_formatted.starts_with("fn first( x: i32 ) -> i32 {"));
    assert!(partially_formatted.contains("fn second(y: i32) -> i32 {"));
}
