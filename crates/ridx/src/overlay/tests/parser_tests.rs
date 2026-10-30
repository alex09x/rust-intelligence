/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use crate::overlay::parser::{is_ident, OverlayParser};

#[test]
fn test_overlay_parser_rich_syntax() {
    let code = r#"
mod my_mod {
    const MY_CONST: i32 = 42;
    static MY_STATIC: i32 = 100;
    type MyType = i32;
    enum MyEnum { A, B }
    trait MyTrait {
        fn trait_fn();
    }
    struct MyStruct;
    fn test_func(mut x: i32, y: String) {
        /* /* nested block comment */ */
        // line comment
        let mut a = (1 + 2);
        let b = [1, 2, 3];
        let c = { let inner = 10; inner };
    }
}
"#;
    let line_starts = crate::overlay::mapping::compute_line_starts(code);
    let mut parser = OverlayParser::new(code, &line_starts);
    parser.parse();
    assert!(!parser.symbols.is_empty());
    assert!(!parser.scopes.is_empty());

    let names: Vec<&str> = parser.symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"my_mod"));
    assert!(names.contains(&"MY_CONST"));
    assert!(names.contains(&"MY_STATIC"));
    assert!(names.contains(&"MyType"));
    assert!(names.contains(&"MyEnum"));
    assert!(names.contains(&"MyTrait"));
    assert!(names.contains(&"trait_fn"));
    assert!(names.contains(&"MyStruct"));
    assert!(names.contains(&"test_func"));
    assert!(names.contains(&"x"));
    assert!(names.contains(&"y"));
    assert!(names.contains(&"a"));
    assert!(names.contains(&"b"));
    assert!(names.contains(&"c"));
    assert!(names.contains(&"inner"));
}

#[test]
fn test_overlay_parser_edge_cases_and_comments() {
    let code = "fn /* /* nested */ */ // comment\n func_with_comments(a: i32) {\n let // comment\n /* nested /* block */ */ val = /* /* block */ */ \"string with \\\"escape\\\" and 'char'\" /* unterminated";
    let line_starts = crate::overlay::mapping::compute_line_starts(code);
    let mut parser = OverlayParser::new(code, &line_starts);
    parser.parse();

    let incomplete_code = "fn ; struct ; enum ; trait ; type ; const ; static ; mod ; let ; let x = 10 } let y = /* unterminated";
    let ls = crate::overlay::mapping::compute_line_starts(incomplete_code);
    let mut p2 = OverlayParser::new(incomplete_code, &ls);
    p2.parse();

    // Let binding with multiple line comments and attributes
    let let_code = "#[inline]\npub fn foo() {\n    let // c1\n    mut // c2\n    z // c3\n    : // c4\n    i32 // c5\n    = // c6\n    42; // c7\n}\n";
    let ls_let = crate::overlay::mapping::compute_line_starts(let_code);
    let mut p3 = OverlayParser::new(let_code, &ls_let);
    p3.parse();
    assert!(p3.symbols.iter().any(|s| s.name == "z"));

    // Test is_ident edge cases (line 38)
    assert!(!is_ident(""));
    assert!(!is_ident("123num"));
    assert!(!is_ident("+"));
    assert!(is_ident("valid_name_42"));

    // Test byte_to_char_line_col exact line start hit (line 64)
    assert_eq!(p3.byte_to_char_line_col(0), (1, 1));
    assert_eq!(p3.byte_to_char_line_col(ls_let[1]), (2, 1));

    // Test fn without parens (line 362) and fn with tuple pattern param (lines 411-412)
    let pat_code = "trait T { fn no_parens; fn pat((a, b): (i32, i32)); }\n";
    let ls_pat = crate::overlay::mapping::compute_line_starts(pat_code);
    let mut p4 = OverlayParser::new(pat_code, &ls_pat);
    p4.parse();
    assert!(p4.symbols.iter().any(|s| s.name == "no_parens"));

    // Test unclosed block comment and parse_ident trailing whitespace (lines 228, 262, 273)
    let unclosed = "/* unclosed comment";
    let ls_unclosed = crate::overlay::mapping::compute_line_starts(unclosed);
    let p5 = OverlayParser::new(unclosed, &ls_unclosed);
    let mut pos_unclosed = 0;
    p5.skip_whitespace_and_comments(&mut pos_unclosed);
    assert_eq!(pos_unclosed, unclosed.len());
    let mut pos_ident = 0;
    assert_eq!(p5.parse_ident(&mut pos_ident), None);

    let ws_code = "   ";
    let ls_ws = crate::overlay::mapping::compute_line_starts(ws_code);
    let p_ws = OverlayParser::new(ws_code, &ls_ws);
    let mut pos_ws = 0;
    assert_eq!(p_ws.parse_ident(&mut pos_ws), None);

    // Non-ident character advancement in outer loop (line 228) and scope pop (line 174)
    let ops_code = "fn f() { let a = 1 + 2, ; }";
    let ls_ops = crate::overlay::mapping::compute_line_starts(ops_code);
    let mut p6 = OverlayParser::new(ops_code, &ls_ops);
    p6.parse();
    assert!(p6.symbols.iter().any(|s| s.name == "a"));
}

#[test]
fn test_overlay_parser_raw_idents_unions_and_const_fn() {
    let code = r#"
        const fn compute_const() -> i32 { 10 }
        union FloatOrInt {
            f: f32,
            i: u32,
        }
        fn test_raw(r#type: i32, r#match: bool) {
            let r#yield = 42;
        }
    "#;
    let ls = crate::overlay::mapping::compute_line_starts(code);
    let mut parser = OverlayParser::new(code, &ls);
    parser.parse();
    let names: Vec<&str> = parser.symbols.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, vec!["compute_const", "FloatOrInt", "test_raw", "r#type", "r#match", "r#yield"]);
}

#[test]
fn test_overlay_parser_lifetimes_and_static_do_not_swallow_or_emit_keywords() {
    use ridx_format::DefKind;

    // 1. Standalone `'static` must emit only `T` (never `str` as a bogus `Static` item)
    let static_only = "pub const T: &'static str = \"x\";\n";
    let ls_static = crate::overlay::mapping::compute_line_starts(static_only);
    let mut p_static = OverlayParser::new(static_only, &ls_static);
    p_static.parse();
    let static_syms: Vec<(&str, DefKind)> = p_static.symbols.iter().map(|s| (s.name.as_str(), s.kind)).collect();
    assert_eq!(static_syms, vec![("T", DefKind::Const)]);

    // 2. Combined `'static` + `'a` lifetime parameters + `->` return type arrow
    let code = concat!("pub const T: &'static str = \"x\";\n", "pub fn f<'a>(s: &'a str) -> &'a str { let _ = 1; if let Some(v) = Some(2) { let _w = v; } s }\n",);
    let ls = crate::overlay::mapping::compute_line_starts(code);
    let mut parser = OverlayParser::new(code, &ls);
    parser.parse();
    let syms: Vec<(&str, DefKind)> = parser.symbols.iter().map(|s| (s.name.as_str(), s.kind)).collect();
    assert_eq!(syms, vec![("T", DefKind::Const), ("f", DefKind::Fn), ("s", DefKind::Local), ("_", DefKind::Local), ("Some", DefKind::Local), ("_w", DefKind::Local),]);
}

#[test]
fn test_overlay_parser_raw_strings_escaped_quotes_and_return_arrows() {
    use ridx_format::DefKind;

    let code = concat!(
        "pub fn parse_tokens<'a, 'r#static>(input: &'a str) -> &'r#static str {\n",
        "    let escaped_quote = '\\'';\n",
        "    let raw_str = r#\"{ fn bogus_fn() {} static BOGUS_STATIC: i32 = 0; 'unclosed }\"#;\n",
        "    let byte_raw = br##\"{ struct BogusStruct; }\"##;\n",
        "    let after_raw: &'static str = input;\n",
        "    after_raw\n",
        "}\n",
    );
    let ls = crate::overlay::mapping::compute_line_starts(code);
    let mut parser = OverlayParser::new(code, &ls);
    parser.parse();
    let syms: Vec<(&str, DefKind)> = parser.symbols.iter().map(|s| (s.name.as_str(), s.kind)).collect();
    assert_eq!(
        syms,
        vec![
            ("parse_tokens", DefKind::Fn),
            ("input", DefKind::Local),
            ("escaped_quote", DefKind::Local),
            ("raw_str", DefKind::Local),
            ("byte_raw", DefKind::Local),
            ("after_raw", DefKind::Local),
        ]
    );
    assert_eq!(parser.scopes.len(), 2);
    assert_eq!(parser.scopes[1].end_byte, code.len() - 1);
}

#[test]
fn test_overlay_parser_records_comment_and_literal_opaque_spans() {
    // Byte layout: line 0 is 22 bytes + newline, line 1 starts at 23, line 2 at 35.
    let code = "fn f<'a>(x: &'a str) {\n    // note\n    let s = \"a // b\"; /* c */ let ch = 'x'; let r = r#\"{\"#;\n}\n";
    let line_starts = crate::overlay::mapping::compute_line_starts(code);
    let mut parser = OverlayParser::new(code, &line_starts);
    parser.parse();

    // `// note`, `"a // b"`, `/* c */`, `'x'`, `r#"{"#`; lifetimes are tokens, not literals.
    assert_eq!(parser.opaque, vec![(27, 34), (47, 55), (57, 64), (74, 77), (87, 93)]);
    assert_eq!(&code[27..34], "// note");
    assert_eq!(&code[47..55], "\"a // b\"");
    assert_eq!(&code[57..64], "/* c */");
    assert_eq!(&code[74..77], "'x'");
    assert_eq!(&code[87..93], "r#\"{\"#");
    // The `{` inside the raw string opens no scope: root plus the fn body only.
    assert_eq!(parser.scopes.len(), 2);
}

#[test]
fn test_overlay_parser_fn_item_start_covers_attributes_and_attached_comments() {
    let code = concat!(
        "/// Doc.\n",                  // byte 0
        "#[inline]\n",                 // byte 9
        "#[cfg_attr(\n",               // byte 19
        "    test,\n",                 // byte 31
        "    allow(dead_code)\n",      // byte 41
        ")]\n",                        // byte 62
        "pub fn a() {}\n",             // byte 65, `{` at 76
        "\n",                          // byte 79
        "// far away\n",               // byte 80
        "\n",                          // byte 92
        "fn b() {}\n",                 // byte 93, `{` at 100
        "const Z: i32 = 1; /* t */\n", // byte 103
        "fn c() {}\n",                 // byte 129, `{` at 136
        "impl X { fn d() {} }\n",      // byte 139, `fn` at 148, `{` at 155
    );
    let line_starts = crate::overlay::mapping::compute_line_starts(code);
    let mut parser = OverlayParser::new(code, &line_starts);
    parser.parse();

    let fn_scopes: Vec<(usize, usize, usize)> = parser.scopes.iter().filter_map(|s| s.body_start_byte.map(|b| (s.sel_start_byte, b, s.end_byte))).collect();
    // a: doc comment and both attributes attached; b: blank line detaches `// far away`;
    // c: a code line with a trailing block comment is not trivia; d: starts after `{`.
    assert_eq!(fn_scopes, vec![(0, 76, 78), (93, 100, 102), (129, 136, 138), (148, 155, 157)]);
    assert_eq!(&code[148..150], "fn");
    assert_eq!(&code[76..78], "{}");
}
