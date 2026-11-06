/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use super::document_symbols;
use serde_json::Value;

type SymbolShape = (String, u64, u64, Vec<(String, u64)>);

/// `(name, kind, selection line, children)` of a symbol tree.
fn shape(v: &Value) -> Vec<SymbolShape> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|s| {
            let kids = s["children"].as_array().unwrap().iter().map(|c| (c["name"].as_str().unwrap().to_string(), c["kind"].as_u64().unwrap())).collect();
            (s["name"].as_str().unwrap().to_string(), s["kind"].as_u64().unwrap(), s["selectionRange"]["start"]["line"].as_u64().unwrap(), kids)
        })
        .collect()
}

#[test]
fn items_impls_and_members_like_rust_analyzer() {
    let src = concat!(
        "use std::fmt;\n",
        "/// Docs.\n",
        "#[derive(Debug)]\n",
        "pub(crate) struct Span<'a> { pub lo: u32, #[doc(hidden)] hi: &'a str }\n",
        "enum Shape { Dot, Line(u8), Rect { w: u32, h: u32 } }\n",
        "impl<'a> fmt::Display for Span<'a>\n",
        "where\n",
        "    'a: 'static,\n",
        "{\n",
        "    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { let s = 1; Ok(()) }\n",
        "    fn new() -> Self { todo!() }\n",
        "}\n",
        "pub trait Area { type Out; const N: u8; fn area(self: Box<Self>) -> u32; }\n",
        "mod inner { pub fn f() { struct Local; let x = 0; } }\n",
        "const MAX: usize = { 3 };\n",
        "static mut COUNT: u32 = 0;\n",
        "type Alias = u8;\n",
        "macro_rules! m { () => {}; }\n",
        "m! { struct NotSeen; }\n",
        "extern \"C\" { fn c_fn(); }\n",
        "fn main() { let s = \"fn hidden() {}\"; }\n",
    );
    let got = shape(&document_symbols(src));
    let names: Vec<_> = got.iter().map(|g| (g.0.as_str(), g.1, g.2)).collect();
    assert_eq!(
        names,
        [
            ("Span", 23, 3),
            ("Shape", 10, 4),
            ("impl fmt::Display for Span<'a>", 19, 5),
            ("Area", 11, 12),
            ("inner", 2, 13),
            ("MAX", 14, 14),
            ("COUNT", 13, 15),
            ("Alias", 26, 16),
            ("m", 12, 17),
            ("main", 12, 20),
        ]
    );
    assert_eq!(got[0].3, [("lo".into(), 8), ("hi".into(), 8)]);
    assert_eq!(got[1].3, [("Dot".into(), 22), ("Line".into(), 22), ("Rect".into(), 22)]);
    assert_eq!(got[2].3, [("fmt".into(), 6), ("new".into(), 12)], "a `self` parameter makes a method");
    assert_eq!(got[3].3, [("Out".into(), 26), ("N".into(), 14), ("area".into(), 6)]);
    assert_eq!(got[4].3, [("f".into(), 12)]);
    let f = &document_symbols(src)[4]["children"][0];
    assert_eq!(f["children"][0]["name"], "Local", "items declared in a body are children; `let` bindings are not symbols");
    assert_eq!(f["children"].as_array().unwrap().len(), 1);
}

#[test]
fn impl_selection_is_the_self_type_and_negative_impls_keep_their_bang() {
    let src = "struct S;\nimpl !Send for S {}\nimpl S { fn get(&mut self) {} }\n";
    let syms = document_symbols(src);
    assert_eq!(syms[1]["name"], "impl !Send for S");
    assert_eq!(syms[1]["selectionRange"]["start"], serde_json::json!({ "line": 1, "character": 15 }));
    assert_eq!(syms[2]["name"], "impl S");
    assert_eq!(syms[2]["selectionRange"]["start"], serde_json::json!({ "line": 2, "character": 5 }));
    assert_eq!(syms[2]["children"][0]["kind"], 6);
}
