/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use ridx::lsp::editing::actions::{handle_code_action, handle_execute_command};
use ridx::lsp::editing::format::{handle_formatting, handle_range_formatting};
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
fn test_refactor_formatting_and_range_formatting_execution() {
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///workspace/src/fmt.rs";
    let file_path = "/workspace/src/fmt.rs";

    let unformatted = concat!(
        "fn first( x: i32 ) -> i32 {\n",
        "x\n",
        "}\n",
        "fn second( y: i32 ) -> i32 {\n",
        "y\n",
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

    // 1. Full formatting: MUST return non-empty edits and format both functions
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

    // 2. Range formatting: only second function (lines 3..5)
    let range_req = json!({
        "params": {
            "textDocument": { "uri": uri },
            "range": {
                "start": { "line": 3, "character": 0 },
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

    // Must have changes in both files
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
fn test_apply_workspace_edit_with_newtext() {
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/test_apply.rs";
    let file = "/tmp/test_apply.rs";
    overlay_mgr.open_document(uri, file, 1, "fn foo() {}\n".to_string());

    let edit_req = json!({
        "params": {
            "command": "ridx.applyEdits",
            "arguments": [
                {
                    "changes": {
                        uri: [
                            {
                                "range": { "start": { "line": 0, "character": 3 }, "end": { "line": 0, "character": 6 } },
                                "newText": "bar"
                            }
                        ]
                    }
                }
            ]
        }
    });

    let res = handle_execute_command(&edit_req, &mut overlay_mgr);
    assert_eq!(res["applied"], true);
    assert_eq!(res["count"], 1);
    let overlay = overlay_mgr.get(file).unwrap();
    assert_eq!(overlay.text, "fn bar() {}\n");
    assert!(overlay.version >= 2);
}

#[test]
fn test_diagnostics_ignores_comment_example() {
    let code = "/* let x: u32 = \"wrong\"; */\nfn main() {}\n";
    let diag = ridx::lsp::presentation::diagnostics::handle_document_diagnostic_text(code);
    let items = diag["items"].as_array().unwrap();
    assert!(
        items.is_empty(),
        "Should not report E0308 inside block comment"
    );

    let code_real = "fn main() { let x: u32 = \"wrong\"; }\n";
    let diag_real =
        ridx::lsp::presentation::diagnostics::handle_document_diagnostic_text(code_real);
    let items_real = diag_real["items"].as_array().unwrap();
    assert!(
        !items_real.is_empty(),
        "Must report E0308 for real code type mismatch"
    );
    assert_eq!(items_real[0]["code"], "E0308");
}

#[test]
fn test_comparison_preserves_full_path_and_rejects_errors() {
    use ridx::compare::types::{CapabilityKind, ComparisonResult};
    let mut res = ComparisonResult::new(CapabilityKind::Def, "test_sym");
    res.ridx_found = true;
    res.prodcode_found = true;
    res.ridx_span = "/one/src/lib.rs:10:1".to_string();
    res.prodcode_span = "/other/src/lib.rs:10:1".to_string();
    assert!(!res.is_match(), "Different directory paths must NOT match");

    res.prodcode_span = "/one/src/lib.rs:10:1".to_string();
    assert!(res.is_match(), "Identical directory paths must match");

    // Errors must not match even if both are found: false
    res.ridx_found = false;
    res.prodcode_found = false;
    res.prodcode_span = "Error (failed)".to_string();
    assert!(!res.is_match(), "Errors must not match");
}

#[test]
fn test_review_v4651_edits_use_original_coordinates() {
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/test_orig_coords.rs";
    let file = "/tmp/test_orig_coords.rs";
    overlay_mgr.open_document(uri, file, 1, "alpha beta".to_string());

    let edit_req = json!({
        "params": {
            "command": "ridx.applyEdits",
            "arguments": [
                {
                    "changes": {
                        uri: [
                            {
                                "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 5 } },
                                "newText": "a"
                            },
                            {
                                "range": { "start": { "line": 0, "character": 6 }, "end": { "line": 0, "character": 10 } },
                                "newText": "b"
                            }
                        ]
                    }
                }
            ]
        }
    });

    let res = handle_execute_command(&edit_req, &mut overlay_mgr);
    assert_eq!(res["applied"], true);
    let overlay = overlay_mgr.get(file).unwrap();
    assert_eq!(overlay.text, "a b");
}

#[test]
fn test_review_v4651_dual_edit_representation_applied_once() {
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/test_dual_rep.rs";
    let file = "/tmp/test_dual_rep.rs";
    overlay_mgr.open_document(uri, file, 1, "fn foo() {}".to_string());

    // Both documentChanges and changes are provided (like rename output)
    let edit_req = json!({
        "params": {
            "command": "ridx.applyEdits",
            "arguments": [
                {
                    "documentChanges": [
                        {
                            "textDocument": { "uri": uri, "version": 1 },
                            "edits": [
                                {
                                    "range": { "start": { "line": 0, "character": 3 }, "end": { "line": 0, "character": 6 } },
                                    "newText": "bar"
                                }
                            ]
                        }
                    ],
                    "changes": {
                        uri: [
                            {
                                "range": { "start": { "line": 0, "character": 3 }, "end": { "line": 0, "character": 6 } },
                                "newText": "bar"
                            }
                        ]
                    }
                }
            ]
        }
    });

    let res = handle_execute_command(&edit_req, &mut overlay_mgr);
    assert_eq!(res["applied"], true);
    assert_eq!(
        res["count"], 1,
        "Only documentChanges should be applied, not both"
    );
    let overlay = overlay_mgr.get(file).unwrap();
    assert_eq!(overlay.text, "fn bar() {}");
}

#[test]
fn test_review_v4651_stale_version_rejected() {
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/test_stale_ver.rs";
    let file = "/tmp/test_stale_ver.rs";
    overlay_mgr.open_document(uri, file, 7, "fn foo() {}".to_string());

    let stale_req = json!({
        "params": {
            "command": "ridx.applyEdits",
            "arguments": [
                {
                    "documentChanges": [
                        {
                            "textDocument": { "uri": uri, "version": 1 },
                            "edits": [
                                {
                                    "range": { "start": { "line": 0, "character": 3 }, "end": { "line": 0, "character": 6 } },
                                    "newText": "bar"
                                }
                            ]
                        }
                    ]
                }
            ]
        }
    });

    let res = handle_execute_command(&stale_req, &mut overlay_mgr);
    assert_eq!(res["applied"], false, "Stale version edit must be rejected");
    let overlay = overlay_mgr.get(file).unwrap();
    assert_eq!(overlay.text, "fn foo() {}");
    assert_eq!(overlay.version, 7, "Version must not rollback");
}

#[test]
fn test_review_v4651_closed_edit_remains_dirty() {
    let mut overlay_mgr = OverlayManager::new();
    let temp_file = std::env::temp_dir().join(format!("test_closed_{}.rs", std::process::id()));
    let _ = fs::write(&temp_file, "fn foo() {}");
    let uri = format!("file://{}", temp_file.display());
    let file = temp_file.to_str().unwrap();

    let edit_req = json!({
        "params": {
            "command": "ridx.applyEdits",
            "arguments": [
                {
                    "changes": {
                        &uri: [
                            {
                                "range": { "start": { "line": 0, "character": 3 }, "end": { "line": 0, "character": 6 } },
                                "newText": "bar"
                            }
                        ]
                    }
                }
            ]
        }
    });

    let res = handle_execute_command(&edit_req, &mut overlay_mgr);
    assert_eq!(res["applied"], true);
    let overlay = overlay_mgr.get(file).unwrap();
    assert_eq!(overlay.text, "fn bar() {}");
    assert!(
        overlay.is_dirty(),
        "Closed file edited on disk must remain dirty relative to compiled index"
    );
    let _ = fs::remove_file(temp_file);
}

#[test]
fn test_review_v4651_extract_preserves_short_circuit() {
    let code = "fn f(){ let ok = false && side_effect(); }";
    let overlay_mgr = OverlayManager::new();
    let temp_file = std::env::temp_dir().join(format!("test_sc_{}.rs", std::process::id()));
    let _ = fs::write(&temp_file, code);
    let uri = format!("file://{}", temp_file.display());

    // Target side_effect() (col 26..39)
    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "range": {
                "start": { "line": 0, "character": 26 },
                "end": { "line": 0, "character": 39 }
            },
            "context": {
                "diagnostics": [],
                "only": ["refactor.extract"]
            }
        }
    });

    let actions = handle_code_action(&req, &overlay_mgr);
    let act_arr = actions.as_array().expect("actions array");
    assert!(
        act_arr.is_empty(),
        "Extract variable must NOT be proposed for right-hand side of short-circuit operator"
    );
    let _ = fs::remove_file(temp_file);
}

#[test]
fn test_review_v4651_inline_completion_unicode_edit() {
    let code = "fn f(){ привет }";
    let overlay_mgr = OverlayManager::new();
    let temp_file = std::env::temp_dir().join(format!("test_inline_u_{}.rs", std::process::id()));
    let _ = fs::write(&temp_file, code);
    let uri = format!("file://{}", temp_file.display());

    // Cursor at end of 'привет' (col 14 in UTF-16)
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

#[test]
fn test_review_v4651_harness_routes_response_ids_and_preserves_errors() {
    use ridx::compare::client::{read_lsp_response, send_lsp_msg};
    use std::io::Cursor;

    let notif = json!({
        "jsonrpc": "2.0",
        "method": "window/logMessage",
        "params": {}
    });
    let resp = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "result": ["test"]
    });
    let mut stream = Vec::new();
    send_lsp_msg(&mut stream, &notif).unwrap();
    send_lsp_msg(&mut stream, &resp).unwrap();

    let mut reader = std::io::BufReader::new(Cursor::new(stream));
    let parsed_resp = read_lsp_response(&mut reader, 2).expect("Must find response id 2");
    assert_eq!(parsed_resp["id"], 2);
    assert_eq!(parsed_resp["result"][0], "test");

    // Stream with JSON-RPC error response
    let err = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "error": { "code": -32602, "message": "Invalid" }
    });
    let mut err_stream = Vec::new();
    send_lsp_msg(&mut err_stream, &err).unwrap();

    let mut err_reader = std::io::BufReader::new(Cursor::new(err_stream));
    let err_resp = read_lsp_response(&mut err_reader, 2).expect("Must read error response");
    assert!(err_resp.get("error").is_some());
    assert_eq!(err_resp["error"]["code"], -32602);
}
