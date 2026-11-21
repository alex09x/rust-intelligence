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
