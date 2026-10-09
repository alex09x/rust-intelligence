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
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::OverlayManager;
use ridx_test_support::apply_text_edits;
use serde_json::json;
use std::fs;

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

    let res = handle_code_action(&req, None, &overlay_mgr).unwrap();
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

    let res = handle_code_action(&req, None, &overlay_mgr).unwrap();
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

    let actions = handle_code_action(&req, None, &overlay_mgr).unwrap();
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
    let actions1 = handle_code_action(&req1, None, &overlay_mgr).unwrap();
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

    // 2. Unicode safety: fn f(){ let grüßen=1; } -> extract 1 without panic
    let code2 = "fn f(){ let grüßen=1; }";
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
    let actions2 = handle_code_action(&req2, None, &overlay_mgr).unwrap();
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
        "fn f(){ let extracted = 1; let grüßen=extracted; }"
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

    let multi_root = MultiRootWorkspace::new();
    let res = handle_workspace_diagnostic(&req, &overlay_mgr, &multi_root);
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

fn assert_extract_rejected(code: &str, line: u64, start: u64, end: u64, msg: &str) {
    let mut overlay_mgr = OverlayManager::new();
    overlay_mgr.open_document("file:///tmp/t.rs", "/tmp/t.rs", 1, code.to_string());
    let req = json!({
        "params": {
            "textDocument": { "uri": "file:///tmp/t.rs" },
            "range": { "start": { "line": line, "character": start }, "end": { "line": line, "character": end } },
            "context": { "only": ["refactor.extract"], "diagnostics": [] }
        }
    });
    assert!(
        handle_code_action(&req, None, &overlay_mgr)
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty(),
        "{msg}"
    );
}

#[test]
fn test_extract_variable_multiline_short_circuit_and_loop_rejected() {
    assert_extract_rejected(
        "fn f(){\n    while next() {\n        work();\n    }\n}",
        1,
        10,
        16,
        "Loop condition must not allow extract variable",
    );
    assert_extract_rejected(
        "fn f(){\n    let ok = false &&\n        side_effect();\n}",
        2,
        8,
        21,
        "Short-circuit rhs must not allow extract variable",
    );
    assert_extract_rejected(
        "fn f(){\n    sink(first(), second());\n}",
        1,
        18,
        26,
        "Extract variable must NOT be proposed for second() when preceded by first()",
    );
}

#[test]
fn test_review_v4651_inline_completion_unicode_edit() {
    let code = "fn f(){ grüßen }";
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
    assert_eq!(range["start"]["character"], 8, "start of 'grüßen'");
    assert_eq!(range["end"]["character"], 14);
    assert_eq!(items[0]["insertText"], "grüßen");
    let _ = fs::remove_file(temp_file);
}
