/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Scanners for character literals, byte character literals, lifetimes, and escapes.

use crate::char_class::{is_forbidden_raw_ident, is_ident_start_char, scan_ident_tail};
use crate::scan_lit::eat_literal_suffix;
use crate::token::{TokenKind, HAS_ERROR};

/// Disambiguates and scans `'...` starting at `start` (`bytes[start] == b'\''`) into either
/// [`TokenKind::Lifetime`] or [`TokenKind::Char`].
pub(crate) fn scan_char_or_lifetime(src: &str, bytes: &[u8], start: usize) -> (TokenKind, u8, usize) {
    let pos = start + 1;
    if pos >= bytes.len() {
        return (TokenKind::Char, HAS_ERROR, pos);
    }
    let b = bytes[pos];
    if b == b'\'' {
        let end = eat_literal_suffix(src, bytes, pos + 1);
        return (TokenKind::Char, HAS_ERROR, end);
    }
    if b == b'\\' {
        let (ok, end_escape) = scan_escape_body(src, bytes, pos + 1, true);
        if end_escape < bytes.len() && bytes[end_escape] == b'\'' {
            let end = eat_literal_suffix(src, bytes, end_escape + 1);
            let flags = if ok { 0 } else { HAS_ERROR };
            return (TokenKind::Char, flags, end);
        }
        return (TokenKind::Char, HAS_ERROR, end_escape);
    }

    // Raw lifetime `'r#ident`
    if b == b'r' && pos + 2 < bytes.len() && bytes[pos + 1] == b'#' {
        let after_hash = pos + 2;
        let ch = src[after_hash..].chars().next().unwrap();
        if is_ident_start_char(ch) {
            let id_end = scan_ident_tail(src, bytes, after_hash + ch.len_utf8());
            if id_end < bytes.len() && bytes[id_end] == b'\'' {
                let end = eat_literal_suffix(src, bytes, id_end + 1);
                return (TokenKind::Char, HAS_ERROR, end);
            }
            let raw_name = &src[after_hash..id_end];
            let flags = if is_forbidden_raw_ident(raw_name) { HAS_ERROR } else { 0 };
            return (TokenKind::Lifetime, flags, id_end);
        }
    }

    let first_ch = src[pos..].chars().next().unwrap();
    if is_ident_start_char(first_ch) {
        let after_first = pos + first_ch.len_utf8();
        let id_end = scan_ident_tail(src, bytes, after_first);
        if id_end < bytes.len() && bytes[id_end] == b'\'' {
            let single_char = id_end == after_first;
            let end = eat_literal_suffix(src, bytes, id_end + 1);
            let flags = if single_char { 0 } else { HAS_ERROR };
            return (TokenKind::Char, flags, end);
        }
        return (TokenKind::Lifetime, 0, id_end);
    }

    if first_ch == '\n' || first_ch == '\r' {
        return (TokenKind::Char, HAS_ERROR, pos);
    }
    let after_ch = pos + first_ch.len_utf8();
    if after_ch < bytes.len() && bytes[after_ch] == b'\'' {
        let end = eat_literal_suffix(src, bytes, after_ch + 1);
        let flags = if first_ch == '\t' { HAS_ERROR } else { 0 };
        return (TokenKind::Char, flags, end);
    }
    (TokenKind::Char, HAS_ERROR, pos)
}

/// Scans a byte character literal `b'...'` where `start` points at `b` (`bytes[start..start+2] == b"b'"`).
pub(crate) fn scan_byte_char(src: &str, bytes: &[u8], start: usize) -> (TokenKind, u8, usize) {
    let pos = start + 2;
    if pos >= bytes.len() {
        return (TokenKind::Byte, HAS_ERROR, pos);
    }
    let b = bytes[pos];
    if b == b'\'' {
        let end = eat_literal_suffix(src, bytes, pos + 1);
        return (TokenKind::Byte, HAS_ERROR, end);
    }
    if b == b'\\' {
        let (ok, end_escape) = scan_escape_body(src, bytes, pos + 1, false);
        if end_escape < bytes.len() && bytes[end_escape] == b'\'' {
            let end = eat_literal_suffix(src, bytes, end_escape + 1);
            let flags = if ok { 0 } else { HAS_ERROR };
            return (TokenKind::Byte, flags, end);
        }
        return (TokenKind::Byte, HAS_ERROR, end_escape);
    }
    let first_ch = src[pos..].chars().next().unwrap();
    if is_ident_start_char(first_ch) {
        let after_first = pos + first_ch.len_utf8();
        let id_end = scan_ident_tail(src, bytes, after_first);
        if id_end < bytes.len() && bytes[id_end] == b'\'' {
            let valid = id_end == after_first && first_ch.is_ascii();
            let end = eat_literal_suffix(src, bytes, id_end + 1);
            return (TokenKind::Byte, if valid { 0 } else { HAS_ERROR }, end);
        }
        return (TokenKind::Byte, HAS_ERROR, id_end);
    }
    if first_ch == '\n' || first_ch == '\r' {
        return (TokenKind::Byte, HAS_ERROR, pos);
    }
    let after_ch = pos + first_ch.len_utf8();
    if after_ch < bytes.len() && bytes[after_ch] == b'\'' {
        let valid = first_ch.is_ascii() && first_ch != '\t';
        let end = eat_literal_suffix(src, bytes, after_ch + 1);
        return (TokenKind::Byte, if valid { 0 } else { HAS_ERROR }, end);
    }
    (TokenKind::Byte, HAS_ERROR, pos)
}

/// Scans the body of a character/byte escape right after `\` (`pos` points at the escaped char).
/// Returns `(valid_escape, end_pos_before_closing_quote)`.
fn scan_escape_body(src: &str, bytes: &[u8], pos: usize, allow_unicode: bool) -> (bool, usize) {
    if pos >= bytes.len() {
        return (false, pos);
    }
    match bytes[pos] {
        b'\n' | b'\r' => (false, pos),
        b'n' | b'r' | b't' | b'\\' | b'0' | b'\'' | b'"' => (true, pos + 1),
        b'x' => {
            let mut p = pos + 1;
            let mut digits = 0usize;
            while digits < 2 && p < bytes.len() && bytes[p].is_ascii_hexdigit() {
                if digits == 0 && allow_unicode && bytes[p] > b'7' {
                    let extra = if p + 1 < bytes.len() && bytes[p + 1].is_ascii_hexdigit() { 2 } else { 1 };
                    return (false, p + extra);
                }
                p += 1;
                digits += 1;
            }
            (digits == 2, p)
        }
        b'u' => {
            let mut p = pos + 1;
            if p < bytes.len() && bytes[p] == b'{' {
                p += 1;
                let mut hex_count = 0usize;
                let mut val = 0u32;
                let lead_ok = allow_unicode && (p >= bytes.len() || bytes[p] != b'_');
                while p < bytes.len() {
                    let ch = bytes[p] as char;
                    if let Some(d) = ch.to_digit(16) {
                        hex_count += 1;
                        val = val.saturating_mul(16).saturating_add(d);
                        p += 1;
                    } else if bytes[p] == b'_' {
                        p += 1;
                    } else if bytes[p] == b'}' {
                        p += 1;
                        let valid = lead_ok && (1..=6).contains(&hex_count) && char::from_u32(val).is_some();
                        return (valid, p);
                    } else {
                        break;
                    }
                }
                (false, p)
            } else {
                (false, p)
            }
        }
        b if b < 0x80 => (false, pos + 1),
        _ => {
            let ch = src[pos..].chars().next().unwrap();
            (false, pos + ch.len_utf8())
        }
    }
}
