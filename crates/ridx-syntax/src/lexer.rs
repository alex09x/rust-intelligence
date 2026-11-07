/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Main lossless Rust lexical scanner (`lex`).

use crate::char_class::{is_ascii_id_start, is_ascii_ws, is_forbidden_raw_ident, is_ident_start_char, is_whitespace_char, scan_ident_tail};
use crate::keyword::classify_ident;
use crate::scan_char::{scan_byte_char, scan_char_or_lifetime};
use crate::scan_lit::{scan_number, scan_quoted_str, scan_raw_str};
use crate::scan_trivia::{scan_block_comment, scan_line_comment, scan_whitespace, try_scan_shebang};
use crate::token::{Token, TokenKind, HAS_ERROR, JOINT_NEXT};

/// Tokenizes `src` into a contiguous, lossless sequence of [`Token`]s.
///
/// Every byte of `src` is covered by exactly one non-empty [`Token`], and concatenating
/// all token slices reproduces `src` identically.
pub fn lex(src: &str) -> Vec<Token> {
    assert!(src.len() <= u32::MAX as usize, "ridx-syntax supports source files up to u32::MAX bytes");
    if src.is_empty() {
        return Vec::new();
    }

    let bytes = src.as_bytes();
    let len = bytes.len();
    let mut tokens = Vec::with_capacity(len / 3 + 8);
    let mut pos = 0usize;

    // Optional UTF-8 BOM (`\u{FEFF}`) at byte 0 is treated as leading whitespace trivia.
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        let ws_end = scan_whitespace(src, bytes, 3);
        tokens.push(Token::new(TokenKind::Whitespace, 0, 0, ws_end as u32));
        pos = ws_end;
    }

    // Optional shebang `#!...` at start of file (byte 0 or immediately following UTF-8 BOM).
    if (pos == 0 || pos == 3) && pos < len && bytes[pos] == b'#' {
        if let Some((kind, end)) = try_scan_shebang(src, bytes, pos) {
            tokens.push(Token::new(kind, 0, pos as u32, end as u32));
            pos = end;
        }
    }

    while pos < len {
        let start = pos;
        let (kind, flags, end) = scan_next_token(src, bytes, start);
        debug_assert!(end > start && end <= len);

        if kind.is_punct() {
            if let Some(prev) = tokens.last_mut() {
                if prev.kind.is_punct() {
                    prev.flags |= JOINT_NEXT;
                }
            }
        }

        tokens.push(Token::new(kind, flags, start as u32, end as u32));
        pos = end;
    }

    tokens
}

#[inline]
fn scan_next_token(src: &str, bytes: &[u8], start: usize) -> (TokenKind, u8, usize) {
    let b = bytes[start];

    if is_ascii_ws(b) {
        let end = scan_whitespace(src, bytes, start);
        return (TokenKind::Whitespace, 0, end);
    }

    match b {
        b'/' => {
            if start + 1 < bytes.len() {
                if bytes[start + 1] == b'/' {
                    return (TokenKind::LineComment, 0, scan_line_comment(bytes, start));
                }
                if bytes[start + 1] == b'*' {
                    let (flags, end) = scan_block_comment(bytes, start);
                    return (TokenKind::BlockComment, flags, end);
                }
            }
            (TokenKind::Slash, 0, start + 1)
        }

        b'(' => (TokenKind::LParen, 0, start + 1),
        b')' => (TokenKind::RParen, 0, start + 1),
        b'{' => (TokenKind::LBrace, 0, start + 1),
        b'}' => (TokenKind::RBrace, 0, start + 1),
        b'[' => (TokenKind::LBracket, 0, start + 1),
        b']' => (TokenKind::RBracket, 0, start + 1),

        b'!' => (TokenKind::Bang, 0, start + 1),
        b'#' => (TokenKind::Hash, 0, start + 1),
        b'$' => (TokenKind::Dollar, 0, start + 1),
        b'%' => (TokenKind::Percent, 0, start + 1),
        b'&' => (TokenKind::Amp, 0, start + 1),
        b'*' => (TokenKind::Star, 0, start + 1),
        b'+' => (TokenKind::Plus, 0, start + 1),
        b',' => (TokenKind::Comma, 0, start + 1),
        b'-' => (TokenKind::Minus, 0, start + 1),
        b'.' => (TokenKind::Dot, 0, start + 1),
        b':' => (TokenKind::Colon, 0, start + 1),
        b';' => (TokenKind::Semi, 0, start + 1),
        b'<' => (TokenKind::Lt, 0, start + 1),
        b'=' => (TokenKind::Eq, 0, start + 1),
        b'>' => (TokenKind::Gt, 0, start + 1),
        b'?' => (TokenKind::Question, 0, start + 1),
        b'@' => (TokenKind::At, 0, start + 1),
        b'^' => (TokenKind::Caret, 0, start + 1),
        b'|' => (TokenKind::Pipe, 0, start + 1),
        b'~' => (TokenKind::Tilde, 0, start + 1),

        b'"' => scan_quoted_str(src, bytes, start, TokenKind::Str),
        b'\'' => scan_char_or_lifetime(src, bytes, start),
        b'0'..=b'9' => scan_number(src, bytes, start),

        b'r' => scan_r_prefix_or_ident(src, bytes, start),
        b'b' => scan_b_prefix_or_ident(src, bytes, start),
        b'c' => scan_c_prefix_or_ident(src, bytes, start),

        _ if is_ascii_id_start(b) => {
            let end = scan_ident_tail(src, bytes, start + 1);
            let kind = classify_ident(&src[start..end]);
            (kind, 0, end)
        }

        _ if b < 0x80 => (TokenKind::Unknown, HAS_ERROR, start + 1),

        _ => scan_non_ascii(src, bytes, start),
    }
}

/// Handles `r"..."`, `r#"..."#`, `r#ident`, or regular identifier starting with `r`.
fn scan_r_prefix_or_ident(src: &str, bytes: &[u8], start: usize) -> (TokenKind, u8, usize) {
    let mut p = start + 1;
    while p < bytes.len() && bytes[p] == b'#' {
        p += 1;
    }
    let hashes = p - (start + 1);
    if p < bytes.len() && bytes[p] == b'"' {
        return scan_raw_str(src, bytes, p, hashes, TokenKind::RawStr);
    }
    if hashes == 1 && p < bytes.len() {
        let ch = src[p..].chars().next().unwrap();
        if is_ident_start_char(ch) {
            let end = scan_ident_tail(src, bytes, p + ch.len_utf8());
            let raw_name = &src[p..end];
            let flags = if is_forbidden_raw_ident(raw_name) { HAS_ERROR } else { 0 };
            return (TokenKind::RawIdent, flags, end);
        }
    }
    let end = scan_ident_tail(src, bytes, start + 1);
    let kind = classify_ident(&src[start..end]);
    (kind, 0, end)
}

/// Handles `b'...'`, `b"..."`, `br"..."`, `br#"..."#`, or regular identifier starting with `b`.
fn scan_b_prefix_or_ident(src: &str, bytes: &[u8], start: usize) -> (TokenKind, u8, usize) {
    if start + 1 < bytes.len() {
        match bytes[start + 1] {
            b'\'' => return scan_byte_char(src, bytes, start),
            b'"' => return scan_quoted_str(src, bytes, start + 1, TokenKind::ByteStr),
            b'r' => {
                let mut p = start + 2;
                while p < bytes.len() && bytes[p] == b'#' {
                    p += 1;
                }
                if p < bytes.len() && bytes[p] == b'"' {
                    let hashes = p - (start + 2);
                    return scan_raw_str(src, bytes, p, hashes, TokenKind::RawByteStr);
                }
            }
            _ => {}
        }
    }
    let end = scan_ident_tail(src, bytes, start + 1);
    let kind = classify_ident(&src[start..end]);
    (kind, 0, end)
}

/// Handles `c"..."`, `cr"..."`, `cr#"..."#` (Rust 2021+), or regular identifier starting with `c`.
fn scan_c_prefix_or_ident(src: &str, bytes: &[u8], start: usize) -> (TokenKind, u8, usize) {
    if start + 1 < bytes.len() {
        match bytes[start + 1] {
            b'"' => return scan_quoted_str(src, bytes, start + 1, TokenKind::CStr),
            b'r' => {
                let mut p = start + 2;
                while p < bytes.len() && bytes[p] == b'#' {
                    p += 1;
                }
                if p < bytes.len() && bytes[p] == b'"' {
                    let hashes = p - (start + 2);
                    return scan_raw_str(src, bytes, p, hashes, TokenKind::RawCStr);
                }
            }
            _ => {}
        }
    }
    let end = scan_ident_tail(src, bytes, start + 1);
    let kind = classify_ident(&src[start..end]);
    (kind, 0, end)
}

fn scan_non_ascii(src: &str, bytes: &[u8], start: usize) -> (TokenKind, u8, usize) {
    let ch = src[start..].chars().next().unwrap();
    let after_first = start + ch.len_utf8();
    if is_whitespace_char(ch) {
        let end = scan_whitespace(src, bytes, after_first);
        return (TokenKind::Whitespace, 0, end);
    }
    if is_ident_start_char(ch) {
        let end = scan_ident_tail(src, bytes, after_first);
        return (TokenKind::Ident, 0, end);
    }
    (TokenKind::Unknown, HAS_ERROR, after_first)
}
