/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use ridx::lsp::editing::actions::handle_code_action;
use ridx::lsp::presentation::workspace_diag::handle_workspace_diagnostic;
use ridx::overlay::OverlayManager;
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

    // Sort descending so later edits do not shift earlier character offsets
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
fn test_refactor_organize_imports_execution() {
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///workspace/src/lib.rs";
    let file_path = "/workspace/src/lib.rs";

    let original_code = concat!(
        "use std::sync::Arc;\n",
        "use std::collections::HashMap;\n",
        "use std::sync::Arc;\n",
        "use std::collections::BTreeMap;\n",
        "use std::path::Path;\n",
        "\n",
        "pub fn run() {}\n",
    );

    overlay_mgr.open_document(uri, file_path, 1, original_code.to_string());

    let req = json!({
        "params": {
            "textDocument": { "uri": uri },
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": { "line": 5, "character": 0 }
            },
            "context": {
                "only": ["source.organizeImports"],
                "diagnostics": []
            }
        }
    });

    let res = handle_code_action(&req, &overlay_mgr);
    let actions = res.as_array().expect("actions list");
    let organize_act = actions
        .iter()
        .find(|a| a["kind"] == "source.organizeImports")
        .expect("organizeImports action found");

    let edits = organize_act["edit"]["changes"][uri]
        .as_array()
        .expect("import edits");
    let transformed = apply_text_edits(original_code, edits);

    let expected_imports = concat!(
        "use std::collections::BTreeMap;\n",
        "use std::collections::HashMap;\n",
        "use std::path::Path;\n",
        "use std::sync::Arc;\n",
        "\n",
        "pub fn run() {}\n",
    );
    assert_eq!(transformed, expected_imports);
}

#[test]
fn test_refactor_extract_variable_execution() {
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///workspace/src/calc.rs";
    let file_path = "/workspace/src/calc.rs";

    let original_code = concat!(
        "fn calculate(a: i32, b: i32) -> i32 {\n",
        "    let result = (a + b) * 2;\n",
        "    result\n",
        "}\n",
    );

    overlay_mgr.open_document(uri, file_path, 1, original_code.to_string());

    let req = json!({
        "params": {
            "textDocument": { "uri": uri },
            "range": {
                "start": { "line": 1, "character": 18 },
                "end": { "line": 1, "character": 23 }
            },
            "context": {
                "only": ["refactor.extract"],
                "diagnostics": []
            }
        }
    });

    let res = handle_code_action(&req, &overlay_mgr);
    let actions = res.as_array().expect("actions list");
    let extract_act = actions
        .iter()
        .find(|a| a["kind"] == "refactor.extract")
        .expect("extract variable action found");

    let edits = extract_act["edit"]["changes"][uri]
        .as_array()
        .expect("extract edits");
    let transformed = apply_text_edits(original_code, edits);

    let expected = concat!(
        "fn calculate(a: i32, b: i32) -> i32 {\n",
        "    let extracted = a + b;\n",
        "    let result = (extracted) * 2;\n",
        "    result\n",
        "}\n",
    );
    assert_eq!(transformed, expected);
}

#[test]
fn test_organize_imports_multiline_items() {
    let code = "use z::Z;\nuse a::{\n    A,\n};\n";
    let overlay_mgr = OverlayManager::new();
    let temp_file =
        std::env::temp_dir().join(format!("test_org_multiline_{}.rs", std::process::id()));
    let _ = fs::write(&temp_file, code);
    let uri = format!("file://{}", temp_file.display());
    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } },
            "context": { "only": ["source.organizeImports"], "diagnostics": [] }
        }
    });

    let actions = handle_code_action(&req, &overlay_mgr);
    let act_arr = actions.as_array().expect("actions array");
    assert!(
        !act_arr.is_empty(),
        "organize imports action must be proposed"
    );
    let edits = act_arr[0]["edit"]["changes"][&uri]
        .as_array()
        .expect("edits");
    let organized = apply_text_edits(code, edits);
    assert!(organized.starts_with("use a::{\n    A,\n};\nuse z::Z;"));
    let _ = fs::remove_file(temp_file);
}

#[test]
fn test_extract_variable_unicode_and_dependency_order() {
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/extract_dep.rs";
    let file = "/tmp/extract_dep.rs";

    // 1. Dependency order: fn f(){ let a=2; let b=a+1; } -> extract a+1
    let code1 = "fn f(){ let a=2; let b=a+1; }";
    overlay_mgr.open_document(uri, file, 1, code1.to_string());

    let req1 = json!({
        "params": {
            "textDocument": { "uri": uri },
            "range": { "start": { "line": 0, "character": 23 }, "end": { "line": 0, "character": 26 } },
            "context": { "only": ["refactor.extract"], "diagnostics": [] }
        }
    });
    let actions1 = handle_code_action(&req1, &overlay_mgr);
    let act_arr1 = actions1.as_array().expect("actions");
    assert!(
        !act_arr1.is_empty(),
        "extract variable action must be proposed"
    );
    let edits1 = act_arr1[0]["edit"]["changes"][uri]
        .as_array()
        .expect("edits");
    let transformed1 = apply_text_edits(code1, edits1);
    assert_eq!(
        transformed1,
        "fn f(){ let a=2; let extracted = a+1; let b=extracted; }"
    );

    // 2. Unicode safety: fn f(){ let привет=1; } -> extract 1 without panic
    let code2 = "fn f(){ let привет=1; }";
    overlay_mgr.open_document(uri, file, 2, code2.to_string());
    let sc = code2[..code2.find('1').unwrap()].encode_utf16().count() as u32;
    let ec = sc + 1;
    let req2 = json!({
        "params": {
            "textDocument": { "uri": uri },
            "range": { "start": { "line": 0, "character": sc }, "end": { "line": 0, "character": ec } },
            "context": { "only": ["refactor.extract"], "diagnostics": [] }
        }
    });
    let actions2 = handle_code_action(&req2, &overlay_mgr);
    let act_arr2 = actions2.as_array().expect("actions");
    assert!(
        !act_arr2.is_empty(),
        "extract variable must be proposed on unicode line"
    );
    let edits2 = act_arr2[0]["edit"]["changes"][uri]
        .as_array()
        .expect("edits");
    let transformed2 = apply_text_edits(code2, edits2);
    assert_eq!(
        transformed2,
        "fn f(){ let extracted = 1; let привет=extracted; }"
    );
}

#[test]
fn test_diagnostics_ignores_comment_example() {
    let diag = ridx::lsp::presentation::diagnostics::handle_document_diagnostic_text(
        "/* let x: u32 = \"wrong\"; */\nfn main() {}\n",
    );
    assert!(diag["items"].as_array().unwrap().is_empty());

    let diag_real = ridx::lsp::presentation::diagnostics::handle_document_diagnostic_text(
        "fn main() { let x: u32 = \"wrong\"; }\n",
    );
    let items_real = diag_real["items"].as_array().unwrap();
    assert!(!items_real.is_empty());
    assert_eq!(items_real[0]["code"], "E0308");
}

#[test]
fn test_workspace_diagnostics_reports_syntax_errors_in_open_overlays() {
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///workspace/src/syntax_err.rs";
    let file = "/workspace/src/syntax_err.rs";
    overlay_mgr.open_document(uri, file, 1, "fn broken() {\n".to_string());

    let req = json!({
        "params": {
            "previousResultIds": []
        }
    });

    let res = handle_workspace_diagnostic(&req, &overlay_mgr);
    let items = res["items"].as_array().expect("items array");
    assert!(
        !items.is_empty(),
        "Workspace diagnostics must report reports for open overlays with errors"
    );
    assert_eq!(items[0]["uri"], uri);
    assert_eq!(items[0]["kind"], "full");
    let errs = items[0]["items"].as_array().expect("diagnostic items");
    assert!(
        !errs.is_empty(),
        "Must contain syntax error for unclosed brace"
    );
    assert!(errs[0]["message"]
        .as_str()
        .unwrap()
        .contains("unclosed delimiter"));
}

#[test]
fn test_extract_variable_multiline_short_circuit_and_loop_rejected() {
    let mut overlay_mgr = OverlayManager::new();
    let uri_loop = "file:///tmp/extract_loop.rs";
    let file_loop = "/tmp/extract_loop.rs";
    let loop_code = "fn f(){\n    while next() {\n        work();\n    }\n}";
    overlay_mgr.open_document(uri_loop, file_loop, 1, loop_code.to_string());

    let req_loop = json!({
        "params": {
            "textDocument": { "uri": uri_loop },
            "range": { "start": { "line": 1, "character": 10 }, "end": { "line": 1, "character": 16 } },
            "context": { "only": ["refactor.extract"], "diagnostics": [] }
        }
    });
    let actions_loop = handle_code_action(&req_loop, &overlay_mgr);
    let act_loop_arr = actions_loop.as_array().expect("actions");
    assert!(
        act_loop_arr.is_empty(),
        "Extract variable must NOT be proposed inside loop condition (while next())"
    );

    let uri_sc = "file:///tmp/extract_multi_sc.rs";
    let file_sc = "/tmp/extract_multi_sc.rs";
    let sc_code = "fn f(){\n    let ok = false &&\n        side_effect();\n}";
    overlay_mgr.open_document(uri_sc, file_sc, 1, sc_code.to_string());

    let req_sc = json!({
        "params": {
            "textDocument": { "uri": uri_sc },
            "range": { "start": { "line": 2, "character": 8 }, "end": { "line": 2, "character": 21 } },
            "context": { "only": ["refactor.extract"], "diagnostics": [] }
        }
    });
    let actions_sc = handle_code_action(&req_sc, &overlay_mgr);
    let act_sc_arr = actions_sc.as_array().expect("actions");
    assert!(
        act_sc_arr.is_empty(),
        "Extract variable must NOT be proposed for multiline short-circuit operand"
    );
}

#[test]
fn test_review_v4651_inline_completion_unicode_edit() {
    let code = "fn f(){ привет }";
    let overlay_mgr = OverlayManager::new();
    let temp_file = std::env::temp_dir().join(format!("test_inline_u_{}.rs", std::process::id()));
    let _ = fs::write(&temp_file, code);
    let uri = format!("file://{}", temp_file.display());

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 14 }
        }
    });

    let res = ridx::lsp::intelligence::hints::handle_inline_completion(&req, Some(&overlay_mgr));
    let items = res["items"].as_array().expect("items array");
    assert!(!items.is_empty());
    let range = &items[0]["range"];
    assert_eq!(
        range["start"]["character"], 8,
        "Must start at start of 'привет', not earlier"
    );
    assert_eq!(range["end"]["character"], 14);
    assert_eq!(items[0]["insertText"], "привет");
    let _ = fs::remove_file(temp_file);
}
