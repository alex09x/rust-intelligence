/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Scanners for numbers and double-quoted / raw string literals (including byte and C-strings).

use crate::char_class::{is_ascii_id_start, is_ident_start_char, scan_ident_tail};
use crate::token::{TokenKind, HAS_ERROR};

/// Eats an optional literal suffix (`ident_start` followed by `ident_continue*`).
#[inline]
pub(crate) fn eat_literal_suffix(src: &str, bytes: &[u8], pos: usize) -> usize {
    if pos >= bytes.len() {
        return pos;
    }
    let b = bytes[pos];
    if is_ascii_id_start(b) {
        scan_ident_tail(src, bytes, pos + 1)
    } else if b >= 0x80 {
        let ch = src[pos..].chars().next().unwrap();
        if is_ident_start_char(ch) {
            scan_ident_tail(src, bytes, pos + ch.len_utf8())
        } else {
            pos
        }
    } else {
        pos
    }
}

/// Scans an integer or floating-point literal starting at `start` (`bytes[start]` is `'0'..='9'`).
pub(crate) fn scan_number(src: &str, bytes: &[u8], start: usize) -> (TokenKind, u8, usize) {
    let mut pos = start;
    let mut flags = 0u8;

    if bytes[pos] == b'0' && pos + 1 < bytes.len() {
        match bytes[pos + 1] {
            b'b' => {
                pos += 2;
                let mut has_digit = false;
                while pos < bytes.len() {
                    match bytes[pos] {
                        b'0' | b'1' => {
                            has_digit = true;
                            pos += 1;
                        }
                        b'_' => pos += 1,
                        b'2'..=b'9' => {
                            has_digit = true;
                            flags |= HAS_ERROR;
                            pos += 1;
                        }
                        _ => break,
                    }
                }
                if !has_digit {
                    flags |= HAS_ERROR;
                }
                pos = eat_literal_suffix(src, bytes, pos);
                return (TokenKind::Int, flags, pos);
            }
            b'o' => {
                pos += 2;
                let mut has_digit = false;
                while pos < bytes.len() {
                    match bytes[pos] {
                        b'0'..=b'7' => {
                            has_digit = true;
                            pos += 1;
                        }
                        b'_' => pos += 1,
                        b'8' | b'9' => {
                            has_digit = true;
                            flags |= HAS_ERROR;
                            pos += 1;
                        }
                        _ => break,
                    }
                }
                if !has_digit {
                    flags |= HAS_ERROR;
                }
                pos = eat_literal_suffix(src, bytes, pos);
                return (TokenKind::Int, flags, pos);
            }
            b'x' => {
                pos += 2;
                let mut has_digit = false;
                while pos < bytes.len() {
                    match bytes[pos] {
                        b'0'..=b'9' | b'a'..=b'f' | b'A'..=b'F' => {
                            has_digit = true;
                            pos += 1;
                        }
                        b'_' => pos += 1,
                        _ => break,
                    }
                }
                if !has_digit {
                    flags |= HAS_ERROR;
                }
                pos = eat_literal_suffix(src, bytes, pos);
                return (TokenKind::Int, flags, pos);
            }
            _ => {}
        }
    }

    pos += 1;
    while pos < bytes.len() && (bytes[pos].is_ascii_digit() || bytes[pos] == b'_') {
        pos += 1;
    }

    if pos < bytes.len() && bytes[pos] == b'.' && is_float_dot(src, bytes, pos) {
        pos += 1;
        if pos < bytes.len() && bytes[pos].is_ascii_digit() {
            pos += 1;
            while pos < bytes.len() && (bytes[pos].is_ascii_digit() || bytes[pos] == b'_') {
                pos += 1;
            }
            if pos < bytes.len() && (bytes[pos] == b'e' || bytes[pos] == b'E') {
                let (next_pos, ok) = scan_exponent(bytes, pos);
                pos = next_pos;
                if !ok {
                    flags |= HAS_ERROR;
                }
            }
            pos = eat_literal_suffix(src, bytes, pos);
        }
        return (TokenKind::Float, flags, pos);
    }

    if pos < bytes.len() && (bytes[pos] == b'e' || bytes[pos] == b'E') {
        let (next_pos, ok) = scan_exponent(bytes, pos);
        pos = next_pos;
        if !ok {
            flags |= HAS_ERROR;
        }
        pos = eat_literal_suffix(src, bytes, pos);
        return (TokenKind::Float, flags, pos);
    }

    let suffix_start = pos;
    pos = eat_literal_suffix(src, bytes, pos);
    let suffix = &src[suffix_start..pos];
    if suffix == "f32" || suffix == "f64" {
        (TokenKind::Float, flags, pos)
    } else {
        (TokenKind::Int, flags, pos)
    }
}

#[inline]
fn is_float_dot(src: &str, bytes: &[u8], dot_pos: usize) -> bool {
    let next = dot_pos + 1;
    if next >= bytes.len() {
        return true;
    }
    let b = bytes[next];
    if b == b'.' || is_ascii_id_start(b) {
        return false;
    }
    if b >= 0x80 {
        let ch = src[next..].chars().next().unwrap();
        if is_ident_start_char(ch) {
            return false;
        }
    }
    true
}

#[inline]
fn scan_exponent(bytes: &[u8], start: usize) -> (usize, bool) {
    let mut pos = start + 1;
    if pos < bytes.len() && (bytes[pos] == b'+' || bytes[pos] == b'-') {
        pos += 1;
    }
    let mut has_digit = false;
    while pos < bytes.len() {
        match bytes[pos] {
            b'0'..=b'9' => {
                has_digit = true;
                pos += 1;
            }
            b'_' => pos += 1,
            _ => break,
        }
    }
    (pos, has_digit)
}

/// Scans a double-quoted string (`"..."`, `b"..."`, `c"..."`), given `quote_pos` pointing at `"`.
pub(crate) fn scan_quoted_str(src: &str, bytes: &[u8], quote_pos: usize, kind: TokenKind) -> (TokenKind, u8, usize) {
    let mut pos = quote_pos + 1;
    while let Some(off) = bytes[pos..].iter().position(|&b| b == b'"' || b == b'\\') {
        pos += off;
        if bytes[pos] == b'"' {
            pos += 1;
            pos = eat_literal_suffix(src, bytes, pos);
            return (kind, 0, pos);
        }
        pos += 1;
        if pos >= bytes.len() {
            return (kind, HAS_ERROR, bytes.len());
        }
        if bytes[pos] < 0x80 {
            pos += 1;
        } else {
            pos += src[pos..].chars().next().unwrap().len_utf8();
        }
    }
    (kind, HAS_ERROR, bytes.len())
}

/// Scans a raw string (`r#"..."#`, `br#"..."#`, `cr#"..."#`) where `quote_pos` points at `"`.
pub(crate) fn scan_raw_str(src: &str, bytes: &[u8], quote_pos: usize, hashes: usize, kind: TokenKind) -> (TokenKind, u8, usize) {
    let mut flags = if hashes >= 256 { HAS_ERROR } else { 0 };
    let mut pos = quote_pos + 1;
    while let Some(off) = bytes[pos..].iter().position(|&b| b == b'"') {
        pos += off + 1;
        if hashes == 0 || (pos + hashes <= bytes.len() && bytes[pos..pos + hashes].iter().all(|&b| b == b'#')) {
            pos += hashes;
            pos = eat_literal_suffix(src, bytes, pos);
            return (kind, flags, pos);
        }
    }
    flags |= HAS_ERROR;
    (kind, flags, bytes.len())
}
