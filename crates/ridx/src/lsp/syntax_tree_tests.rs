/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! `ridx/syntaxTree`: rust-analyzer's JSON shape and kind names, lossless.

use super::*;
use serde_json::Value;

/// Every token of `v`, in order, as (kind, start offset, end offset).
fn tokens(v: &Value, out: &mut Vec<(String, u64, u64)>) {
    if v["type"] == "Token" {
        out.push((v["kind"].as_str().unwrap().to_string(), v["start"][0].as_u64().unwrap(), v["end"][0].as_u64().unwrap()));
    }
    for c in v["children"].as_array().into_iter().flatten() {
        tokens(c, out);
    }
}

fn kinds(v: &Value, out: &mut Vec<String>) {
    if v["type"] == "Node" {
        out.push(v["kind"].as_str().unwrap().to_string());
    }
    for c in v["children"].as_array().into_iter().flatten() {
        kinds(c, out);
    }
}

/// prod-code, m=20253: rust-analyzer's `viewSyntaxTree` JSON, lossless, from `SOURCE_FILE`.
#[test]
fn test_the_tree_is_rust_analyzers_json_and_lossless() {
    let src = "// lead\npub fn twice(x: u32) -> u32 {\n    let y = x * 2;\n    std::cmp::max(y, 1) >> 1\n}\n";
    let v: Value = serde_json::from_str(&syntax_tree_json(src, None)).unwrap();
    assert_eq!((v["type"].as_str(), v["kind"].as_str()), (Some("Node"), Some("SOURCE_FILE")));
    assert_eq!(v["start"], serde_json::json!([0, 0, 0]));
    let mut toks = Vec::new();
    tokens(&v, &mut toks);
    // Every byte, once, in order.
    let mut at = 0;
    for (_, s, e) in &toks {
        assert_eq!(*s, at, "{toks:?}");
        at = *e;
    }
    assert_eq!(at as usize, src.len());
    let toks: Vec<&str> = toks.iter().map(|(k, _, _)| k.as_str()).collect();
    for k in ["COMMENT", "PUB_KW", "FN_KW", "IDENT", "THIN_ARROW", "L_CURLY", "LET_KW", "STAR", "INT_NUMBER", "SEMICOLON", "COLON2", "SHR", "R_CURLY", "WHITESPACE"] {
        assert!(toks.contains(&k), "{k} in {toks:?}");
    }
    let mut ks = Vec::new();
    kinds(&v, &mut ks);
    for k in [
        "FN",
        "NAME",
        "PARAM_LIST",
        "PARAM",
        "IDENT_PAT",
        "PATH_TYPE",
        "RET_TYPE",
        "BLOCK_EXPR",
        "STMT_LIST",
        "LET_STMT",
        "BIN_EXPR",
        "PATH_EXPR",
        "PATH",
        "PATH_SEGMENT",
        "NAME_REF",
        "CALL_EXPR",
        "ARG_LIST",
        "LITERAL",
    ] {
        assert!(ks.contains(&k.to_string()), "{k} in {ks:?}");
    }
    // Line and column of `let`, 0-based, in bytes.
    let let_stmt = src.find("let").unwrap();
    assert!(syntax_tree_json(src, None).contains(&format!(r#""kind":"LET_STMT","start":[{let_stmt},2,4]"#)));
}

#[test]
fn test_a_range_gives_the_node_covering_it() {
    let src = "fn f() {\n    let y = 1 + 2;\n}\n";
    let s = src.find("1 + 2").unwrap();
    let v: Value = serde_json::from_str(&syntax_tree_json(src, Some(s..s + 5))).unwrap();
    assert_eq!(v["kind"], "BIN_EXPR");
    // `Vec<Vec<u8>>` closes two generic lists: no `>>` there.
    let v = syntax_tree_json("fn f(v: Vec<Vec<u8>>) {}\n", None);
    assert!(!v.contains("\"SHR\""), "{v}");
}

/// The nodes of `v` of kind `kind`.
fn of_kind<'v>(v: &'v Value, kind: &str, out: &mut Vec<&'v Value>) {
    if v["kind"] == kind {
        out.push(v);
    }
    for c in v["children"].as_array().into_iter().flatten() {
        of_kind(c, kind, out);
    }
}

fn tree(src: &str) -> Value {
    serde_json::from_str(&syntax_tree_json(src, None)).unwrap()
}

fn count(src: &str, kind: &str) -> usize {
    let v = tree(src);
    let mut found = Vec::new();
    of_kind(&v, kind, &mut found);
    found.len()
}

/// Review of 27bd4208: a long chain of operands is a deep tree, written with no recursion.
#[test]
fn test_a_deep_tree_is_written_on_a_small_stack() {
    let src = format!("fn f() -> u32 {{ {}1 }}\n", "1 + ".repeat(12_000));
    let len = std::thread::Builder::new().stack_size(1 << 20).spawn(move || syntax_tree_json(&src, None).len()).unwrap().join().unwrap();
    assert!(len > 1_000_000);
}

/// Review of 27bd4208: tokens are named by their role, as rust-analyzer's parser names them.
#[test]
fn test_tokens_are_named_by_their_role() {
    let mut toks = Vec::new();
    tokens(&tree("macro_rules! m { () => {} }\n"), &mut toks);
    assert_eq!(toks[0].0, "MACRO_RULES_KW", "{toks:?}");
    let mut toks = Vec::new();
    tokens(&tree("fn f() { let c = || 0; let b = true || false; }\n"), &mut toks);
    let pipes: Vec<&str> = toks.iter().filter(|t| t.0.starts_with("PIPE")).map(|t| t.0.as_str()).collect();
    assert_eq!(pipes, ["PIPE", "PIPE", "PIPE2"], "{toks:?}");
}

/// Review of 27bd4208: a macro call in an expression or a type has rust-analyzer's wrapper; an unclosed block its
/// statement list; the comments above an item are the item's.
#[test]
fn test_the_tree_has_rust_analyzers_wrappers() {
    assert_eq!(count("fn f() { let _ = vec![1u8]; }\n", "MACRO_EXPR"), 1);
    assert_eq!(count("fn f(x: ty!()) {}\n", "MACRO_TYPE"), 1);
    assert_eq!(count("macro_rules! m { () => {} }\nm!();\n", "MACRO_EXPR"), 0, "an item's macro call is no expression");
    assert_eq!(count("fn f() { let x = 1;", "STMT_LIST"), 1);
    let v = tree("/// docs\nfn f() {}\n");
    let mut found = Vec::new();
    of_kind(&v, "FN", &mut found);
    assert_eq!(found[0]["start"], serde_json::json!([0, 0, 0]));
    assert_eq!(found[0]["children"][0]["kind"], "COMMENT");
    let v = tree("// one\n\nfn f() {}\n");
    let mut found = Vec::new();
    of_kind(&v, "FN", &mut found);
    assert_eq!(found[0]["start"][0], 8, "a blank line keeps a comment off the item");
}

/// Review of 27bd4208: the smallest node of the tree returned, a synthetic one included; a malformed range is refused.
#[test]
fn test_a_range_selects_in_the_returned_tree_and_must_be_valid() {
    let src = "fn f(){foo();}\n";
    let s = src.find("foo").unwrap();
    let v: Value = serde_json::from_str(&syntax_tree_json(src, Some(s..s + 3))).unwrap();
    assert_eq!(v["kind"], "NAME_REF");
    let b = src.find('{').unwrap();
    let v: Value = serde_json::from_str(&syntax_tree_json(src, Some(b..src.len() - 1))).unwrap();
    assert_eq!(v["kind"], "STMT_LIST");
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("lib.rs");
    std::fs::write(&file, src).unwrap();
    let uri = format!("file://{}", file.display());
    let call = |range: Value| {
        let msg = serde_json::json!({ "params": { "textDocument": { "uri": uri }, "range": range } });
        let ov = crate::overlay::OverlayManager::new();
        crate::lsp::ridx_methods::dispatch("ridx/syntaxTree", &msg, None, &ov, &mut crate::lsp::protocol::PositionConverter::new())
    };
    assert!(call(serde_json::json!({ "start": { "line": 0, "character": 7 } })).is_err(), "no end");
    assert!(call(serde_json::json!({ "start": { "line": 4294967296u64, "character": 0 }, "end": { "line": 0, "character": 1 } })).is_err(), "no such line");
    assert!(call(serde_json::json!({ "start": { "line": 0, "character": 10 }, "end": { "line": 0, "character": 7 } })).is_err(), "reversed");
    assert!(call(serde_json::json!({ "start": { "line": 0, "character": 7 }, "end": { "line": 0, "character": 10 } })).is_ok());
}

/// The start offset of the first node of `kind` in `v`, in preorder.
fn start_of(v: &Value, kind: &str) -> Option<u64> {
    if v["type"] == "Node" && v["kind"] == kind {
        return v["start"][0].as_u64();
    }
    v["children"].as_array().into_iter().flatten().find_map(|c| start_of(c, kind))
}

/// Review of d7ed2459: a macro's wrapper by the slot it fills; inner documentation stays with its scope and outer
/// documentation crosses a blank line; a cursor at the start of an item is in it.
#[test]
fn test_slots_documentation_and_cursors_as_rust_analyzer_has_them() {
    let tree = |src: &str| -> Value { serde_json::from_str(&syntax_tree_json(src, None)).unwrap() };
    let wrapper = |src: &str| {
        let v = tree(src);
        let mut ks = Vec::new();
        kinds(&v, &mut ks);
        ks.into_iter().find(|k| k == "MACRO_TYPE" || k == "MACRO_EXPR")
    };
    assert_eq!(wrapper("type Alias = ty!();").as_deref(), Some("MACRO_TYPE"));
    assert_eq!(wrapper("fn f(x: u8) -> u16 { x as ty!() }").as_deref(), Some("MACRO_TYPE"));
    assert_eq!(wrapper("fn f(_: [u8; n!()]) {}").as_deref(), Some("MACRO_EXPR"));
    assert_eq!(wrapper("fn f(_: [ty!(); 2]) {}").as_deref(), Some("MACRO_TYPE"));
    assert_eq!(start_of(&tree("//! crate\n\nfn f() {}\n"), "FN"), Some(11));
    assert_eq!(start_of(&tree("//! crate\nfn f() {}\n"), "FN"), Some(10));
    assert_eq!(start_of(&tree("mod m {\n    //! module\n    fn f() {}\n}\n"), "FN"), Some(27));
    assert_eq!(start_of(&tree("/// docs\n\nfn f() {}\n"), "FN"), Some(0));
    assert_eq!(start_of(&tree("// plain\n\nfn f() {}\n"), "FN"), Some(10));
    let src = "fn a() {}fn b() {}";
    let v: Value = serde_json::from_str(&syntax_tree_json(src, Some(9..9))).unwrap();
    assert_eq!((v["kind"].as_str(), v["start"][0].as_u64()), (Some("FN"), Some(9)), "{v}");
    let v: Value = serde_json::from_str(&syntax_tree_json(src, Some(src.len()..src.len()))).unwrap();
    assert_ne!(v["kind"], "SOURCE_FILE", "the end of the text is in what ends there: {v}");
}
