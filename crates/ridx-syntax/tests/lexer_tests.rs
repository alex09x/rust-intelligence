/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use ridx_syntax::{lex, token_at, Token, TokenKind, HAS_ERROR, JOINT_NEXT};

fn assert_lossless(src: &str) -> Vec<Token> {
    let tokens = lex(src);
    if src.is_empty() {
        assert!(tokens.is_empty());
        assert_eq!(token_at(&tokens, 0), None);
        return tokens;
    }
    assert_eq!(tokens[0].start, 0);
    for pair in tokens.windows(2) {
        assert!(pair[0].start < pair[0].end, "empty token: {:?}", pair[0]);
        assert_eq!(pair[0].end, pair[1].start, "gap/overlap: {:?}", pair);
        let expected_joint = pair[0].kind.is_punct() && pair[1].kind.is_punct();
        assert_eq!(pair[0].is_joint(), expected_joint, "JOINT_NEXT mismatch between {:?} and {:?}", pair[0], pair[1]);
    }
    let last = tokens.last().unwrap();
    assert!(last.start < last.end);
    assert!(!last.is_joint());
    assert_eq!(last.end as usize, src.len());

    let reconstructed: String = tokens.iter().map(|t| t.text(src)).collect();
    assert_eq!(reconstructed, src);

    for (idx, tok) in tokens.iter().enumerate() {
        for b in tok.start..tok.end {
            assert_eq!(token_at(&tokens, b), Some(idx));
        }
    }
    assert_eq!(token_at(&tokens, src.len() as u32), None);
    tokens
}

fn non_trivia(src: &str) -> Vec<(TokenKind, &str, u8)> {
    assert_lossless(src).into_iter().filter(|t| !t.is_trivia()).map(|t| (t.kind, t.text(src), t.flags)).collect()
}

#[test]
fn empty_bom_and_shebang_vs_inner_attr() {
    assert_lossless("");

    let shebang = "#!/usr/bin/env rust-script\nfn main() {}";
    let toks = assert_lossless(shebang);
    assert_eq!(toks[0].kind, TokenKind::Shebang);
    assert!(toks[0].is_trivia());
    assert_eq!(toks[0].text(shebang), "#!/usr/bin/env rust-script");

    let bom_shebang = "\u{FEFF}#!/usr/bin/env cargo\npub fn f() {}";
    let toks = assert_lossless(bom_shebang);
    assert_eq!(toks[0].kind, TokenKind::Whitespace);
    assert_eq!(toks[1].kind, TokenKind::Shebang);

    let inner_attr = "#![no_std]\n#![allow(dead_code)]";
    let nt = non_trivia(inner_attr);
    assert_eq!(nt[0], (TokenKind::Hash, "#", JOINT_NEXT));
    assert_eq!(nt[1], (TokenKind::Bang, "!", 0));
    assert_eq!(nt[2], (TokenKind::LBracket, "[", 0));

    let spaced_inner = "#! /* comment */\n[allow(unused)]";
    let nt = non_trivia(spaced_inner);
    assert_eq!(nt[0], (TokenKind::Hash, "#", JOINT_NEXT));
    assert_eq!(nt[1], (TokenKind::Bang, "!", 0));
    assert_eq!(nt[2], (TokenKind::LBracket, "[", 0));
}

#[test]
fn subslice_token_at_bounds() {
    let src = "pub fn add(a: i32, b: i32) -> i32 { a + b }";
    let toks = assert_lossless(src);
    let sub = &toks[4..10];
    assert_eq!(token_at(sub, 0), None);
    assert_eq!(token_at(sub, sub[0].start - 1), None);
    assert_eq!(token_at(sub, sub[0].start), Some(0));
    assert_eq!(token_at(sub, sub.last().unwrap().end - 1), Some(sub.len() - 1));
    assert_eq!(token_at(sub, sub.last().unwrap().end), None);
}

#[test]
fn strict_vs_contextual_keywords_and_raw_idents() {
    let strict = "as async await break const continue crate dyn else enum extern false fn for if \
                  impl in let loop match mod move mut pub ref return self Self static struct \
                  super trait true type unsafe use where while _";
    let nt = non_trivia(strict);
    assert_eq!(nt.len(), 39);
    for (kind, text, flags) in &nt[..38] {
        assert!(kind.is_keyword(), "{text} should be strict keyword, got {kind:?}");
        assert_eq!(*flags, 0);
    }
    assert_eq!(nt[38], (TokenKind::Underscore, "_", 0));

    let contextual = "default union gen macro_rules auto raw safe try _foo Δt привет";
    for (kind, text, flags) in non_trivia(contextual) {
        assert_eq!(kind, TokenKind::Ident, "{text} must stay Ident");
        assert_eq!(flags, 0);
    }

    let raw = "r#type r#gen r#async r#match r#$id r#self r#_";
    let nt = non_trivia(raw);
    assert_eq!(nt[0], (TokenKind::RawIdent, "r#type", 0));
    assert_eq!(nt[1], (TokenKind::RawIdent, "r#gen", 0));
    assert_eq!(nt[2], (TokenKind::RawIdent, "r#async", 0));
    assert_eq!(nt[3], (TokenKind::RawIdent, "r#match", 0));
    assert_eq!(nt[4], (TokenKind::Ident, "r", 0));
    assert_eq!(nt[5], (TokenKind::Hash, "#", JOINT_NEXT));
    assert_eq!(nt[6], (TokenKind::Dollar, "$", 0));
    assert_eq!(nt[7], (TokenKind::Ident, "id", 0));
    assert_eq!(nt[8], (TokenKind::RawIdent, "r#self", HAS_ERROR));
    assert_eq!(nt[9], (TokenKind::RawIdent, "r#_", HAS_ERROR));
}

#[test]
fn atomic_punct_joint_and_comment_separation() {
    let src = "Vec<Box<T>> :: -> => ..= &&str || Vec<T>// trailing\n";
    let nt = non_trivia(src);
    // Vec < Box < T > >
    assert_eq!(nt[1], (TokenKind::Lt, "<", 0));
    assert_eq!(nt[3], (TokenKind::Lt, "<", 0));
    assert_eq!(nt[5], (TokenKind::Gt, ">", JOINT_NEXT));
    assert_eq!(nt[6], (TokenKind::Gt, ">", 0));
    // ::
    assert_eq!(nt[7], (TokenKind::Colon, ":", JOINT_NEXT));
    assert_eq!(nt[8], (TokenKind::Colon, ":", 0));
    // ->
    assert_eq!(nt[9], (TokenKind::Minus, "-", JOINT_NEXT));
    assert_eq!(nt[10], (TokenKind::Gt, ">", 0));
    // =>
    assert_eq!(nt[11], (TokenKind::Eq, "=", JOINT_NEXT));
    assert_eq!(nt[12], (TokenKind::Gt, ">", 0));
    // ..=
    assert_eq!(nt[13], (TokenKind::Dot, ".", JOINT_NEXT));
    assert_eq!(nt[14], (TokenKind::Dot, ".", JOINT_NEXT));
    assert_eq!(nt[15], (TokenKind::Eq, "=", 0));
    // &&str
    assert_eq!(nt[16], (TokenKind::Amp, "&", JOINT_NEXT));
    assert_eq!(nt[17], (TokenKind::Amp, "&", 0));
    assert_eq!(nt[18], (TokenKind::Ident, "str", 0));
    // ||
    assert_eq!(nt[19], (TokenKind::Pipe, "|", JOINT_NEXT));
    assert_eq!(nt[20], (TokenKind::Pipe, "|", 0));
    // Vec<T>// trailing -> `>` before `//` must NOT have JOINT_NEXT
    assert_eq!(nt[24], (TokenKind::Gt, ">", 0));
}

#[test]
fn numbers_ranges_methods_and_tuple_fields() {
    let src = "0..10 0..=10 1.0 1. 1.foo() 1.e5 1.0e-5f64 0f32 0_f64 0x0f32 t.0.1";
    let nt = non_trivia(src);
    assert_eq!(nt[0], (TokenKind::Int, "0", 0));
    assert_eq!(nt[1], (TokenKind::Dot, ".", JOINT_NEXT));
    assert_eq!(nt[2], (TokenKind::Dot, ".", 0));
    assert_eq!(nt[3], (TokenKind::Int, "10", 0));

    assert_eq!(nt[4], (TokenKind::Int, "0", 0));
    assert_eq!(nt[5], (TokenKind::Dot, ".", JOINT_NEXT));
    assert_eq!(nt[6], (TokenKind::Dot, ".", JOINT_NEXT));
    assert_eq!(nt[7], (TokenKind::Eq, "=", 0));
    assert_eq!(nt[8], (TokenKind::Int, "10", 0));

    assert_eq!(nt[9], (TokenKind::Float, "1.0", 0));
    assert_eq!(nt[10], (TokenKind::Float, "1.", 0));

    // 1.foo()
    assert_eq!(nt[11], (TokenKind::Int, "1", 0));
    assert_eq!(nt[12], (TokenKind::Dot, ".", 0));
    assert_eq!(nt[13], (TokenKind::Ident, "foo", 0));

    // 1.e5 -> Int("1"), Dot, Ident("e5")
    assert_eq!(nt[16], (TokenKind::Int, "1", 0));
    assert_eq!(nt[17], (TokenKind::Dot, ".", 0));
    assert_eq!(nt[18], (TokenKind::Ident, "e5", 0));

    assert_eq!(nt[19], (TokenKind::Float, "1.0e-5f64", 0));
    assert_eq!(nt[20], (TokenKind::Float, "0f32", 0));
    assert_eq!(nt[21], (TokenKind::Float, "0_f64", 0));
    assert_eq!(nt[22], (TokenKind::Int, "0x0f32", 0));

    // t.0.1 -> Ident("t"), Dot, Float("0.1") (parser splits Float in field access)
    assert_eq!(nt[23], (TokenKind::Ident, "t", 0));
    assert_eq!(nt[24], (TokenKind::Dot, ".", 0));
    assert_eq!(nt[25], (TokenKind::Float, "0.1", 0));
}

#[test]
fn strings_cstrings_chars_and_lifetimes() {
    let src = r###""a\"b" r"raw" r##"a"#b"## b"bs" br#"rbs"# c"cs" cr#"rcs"# 'a '_ 'static 'r#async 'a' '_' '\'' '\\' '\x7f' '\u{1F600}' '😊' b'z' b'\xff'"###;
    let nt = non_trivia(src);
    let kinds: Vec<TokenKind> = nt
        .iter()
        .map(|(k, _, f)| {
            assert_eq!(*f, 0, "unexpected error flag in {nt:?}");
            *k
        })
        .collect();
    assert_eq!(
        kinds,
        vec![
            TokenKind::Str,
            TokenKind::RawStr,
            TokenKind::RawStr,
            TokenKind::ByteStr,
            TokenKind::RawByteStr,
            TokenKind::CStr,
            TokenKind::RawCStr,
            TokenKind::Lifetime,
            TokenKind::Lifetime,
            TokenKind::Lifetime,
            TokenKind::Lifetime,
            TokenKind::Char,
            TokenKind::Char,
            TokenKind::Char,
            TokenKind::Char,
            TokenKind::Char,
            TokenKind::Char,
            TokenKind::Char,
            TokenKind::Byte,
            TokenKind::Byte,
        ]
    );
}

#[test]
fn broken_and_unclosed_inputs_set_has_error_and_preserve_roundtrip() {
    let cases: &[(&str, TokenKind)] = &[
        ("\"unclosed string\nlet x = 1;", TokenKind::Str),
        ("\"trailing backslash\\", TokenKind::Str),
        ("b\"unclosed byte str", TokenKind::ByteStr),
        ("c\"unclosed c str", TokenKind::CStr),
        ("r#\"unclosed raw\" without hash", TokenKind::RawStr),
        ("br##\"unclosed\"#", TokenKind::RawByteStr),
        ("cr#\"unclosed c raw", TokenKind::RawCStr),
        ("/* outer /* nested unclosed */", TokenKind::BlockComment),
        ("'", TokenKind::Char),
        ("''", TokenKind::Char),
        ("'multi'", TokenKind::Char),
        ("'\\x80'", TokenKind::Char),
        ("'\\u{d800}'", TokenKind::Char),
        ("'\\u{110000}'", TokenKind::Char),
        ("b'", TokenKind::Byte),
        ("b''", TokenKind::Byte),
        ("b'ab'", TokenKind::Byte),
        ("b'😊'", TokenKind::Byte),
        ("0b", TokenKind::Int),
        ("0b102", TokenKind::Int),
        ("0o89", TokenKind::Int),
        ("0x_", TokenKind::Int),
        ("1e+", TokenKind::Float),
        ("1.5e", TokenKind::Float),
        ("\\", TokenKind::Unknown),
        ("`", TokenKind::Unknown),
        ("\0", TokenKind::Unknown),
        ("🚀", TokenKind::Unknown),
    ];

    for &(input, expected_err_kind) in cases {
        let toks = assert_lossless(input);
        assert!(toks.iter().any(|t| t.kind == expected_err_kind && t.has_error()), "expected {expected_err_kind:?} with HAS_ERROR on input {input:?}, got {toks:?}");
    }

    // Unclosed char literal on a line with subsequent code must NOT swallow the rest of the line.
    let overlay_line = "let c = '; let d = 'a';";
    let nt = non_trivia(overlay_line);
    assert_eq!(nt[3], (TokenKind::Char, "'", HAS_ERROR));
    assert_eq!(nt[4], (TokenKind::Semi, ";", 0));
    assert_eq!(nt[5], (TokenKind::Let, "let", 0));
    assert_eq!(nt[8], (TokenKind::Char, "'a'", 0));
}
