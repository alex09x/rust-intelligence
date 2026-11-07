/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Byte-level ASCII lookup tables and zero-dependency Unicode character classification.

const F_WS: u8 = 1 << 0;
const F_ID_START: u8 = 1 << 1;
const F_ID_CONT: u8 = 1 << 2;

const ASCII_TABLE: [u8; 256] = {
    let mut table = [0u8; 256];
    table[b'\t' as usize] |= F_WS;
    table[b'\n' as usize] |= F_WS;
    table[0x0B] |= F_WS;
    table[0x0C] |= F_WS;
    table[b'\r' as usize] |= F_WS;
    table[b' ' as usize] |= F_WS;

    let mut c = b'a';
    while c <= b'z' {
        table[c as usize] |= F_ID_START | F_ID_CONT;
        c += 1;
    }
    let mut c = b'A';
    while c <= b'Z' {
        table[c as usize] |= F_ID_START | F_ID_CONT;
        c += 1;
    }
    table[b'_' as usize] |= F_ID_START | F_ID_CONT;
    let mut c = b'0';
    while c <= b'9' {
        table[c as usize] |= F_ID_CONT;
        c += 1;
    }
    table
};

#[inline]
pub(crate) const fn is_ascii_ws(b: u8) -> bool {
    (ASCII_TABLE[b as usize] & F_WS) != 0
}

#[inline]
pub(crate) const fn is_ascii_id_start(b: u8) -> bool {
    (ASCII_TABLE[b as usize] & F_ID_START) != 0
}

#[inline]
pub(crate) const fn is_ascii_id_cont(b: u8) -> bool {
    (ASCII_TABLE[b as usize] & F_ID_CONT) != 0
}

/// Rust lexical whitespace definition (`Pattern_White_Space`, matching `rustc_lexer`).
#[inline]
pub(crate) const fn is_whitespace_char(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\x0B' | '\x0C' | '\r' | ' ' | '\u{0085}' | '\u{200E}' | '\u{200F}' | '\u{2028}' | '\u{2029}')
}

/// Identifier start (`_` or `XID_Start`).
#[inline]
pub(crate) fn is_ident_start_char(c: char) -> bool {
    if c.is_ascii() {
        is_ascii_id_start(c as u8)
    } else {
        c.is_alphabetic() || matches!(c, '\u{1885}' | '\u{1886}' | '\u{2118}' | '\u{212E}' | '\u{309B}' | '\u{309C}')
    }
}

/// Identifier continuation (`XID_Continue`).
#[inline]
pub(crate) fn is_ident_continue_char(c: char) -> bool {
    if c.is_ascii() {
        is_ascii_id_cont(c as u8)
    } else {
        c.is_alphanumeric() || is_non_ascii_ident_continue_mark(c)
    }
}

#[inline]
fn is_non_ascii_ident_continue_mark(c: char) -> bool {
    matches!(
        c,
        '\u{00B7}'
            | '\u{0300}'..='\u{036F}'
            | '\u{0387}'
            | '\u{0483}'..='\u{0487}'
            | '\u{0591}'..='\u{05BD}'
            | '\u{05BF}'
            | '\u{05C1}'..='\u{05C2}'
            | '\u{05C4}'..='\u{05C5}'
            | '\u{05C7}'
            | '\u{0610}'..='\u{061A}'
            | '\u{064B}'..='\u{065F}'
            | '\u{0670}'
            | '\u{06D6}'..='\u{06DC}'
            | '\u{06DF}'..='\u{06E4}'
            | '\u{06E7}'..='\u{06E8}'
            | '\u{06EA}'..='\u{06ED}'
            | '\u{0730}'..='\u{074A}'
            | '\u{0901}'..='\u{0903}'
            | '\u{093A}'..='\u{094F}'
            | '\u{0951}'..='\u{0957}'
            | '\u{0962}'..='\u{0963}'
            | '\u{0E31}'
            | '\u{0E34}'..='\u{0E3A}'
            | '\u{0E47}'..='\u{0E4E}'
            | '\u{1369}'..='\u{1371}'
            | '\u{1885}'..='\u{1886}'
            | '\u{19DA}'
            | '\u{1DC0}'..='\u{1DFF}'
            | '\u{203F}'..='\u{2040}'
            | '\u{2054}'
            | '\u{20D0}'..='\u{20DC}'
            | '\u{2118}'
            | '\u{212E}'
            | '\u{3099}'..='\u{309C}'
            | '\u{FE20}'..='\u{FE2F}'
            | '\u{FE33}'..='\u{FE34}'
            | '\u{FE4D}'..='\u{FE4F}'
            | '\u{FF3F}'
    )
}

/// Scans an identifier body starting at `pos` (where the start character at `pos` has
/// already been validated or `pos` is the start of continuation characters).
#[inline(always)]
pub(crate) fn scan_ident_tail(src: &str, bytes: &[u8], mut pos: usize) -> usize {
    let len = bytes.len();
    while pos < len {
        let b = bytes[pos];
        if is_ascii_id_cont(b) {
            pos += 1;
        } else if b >= 0x80 {
            return scan_ident_tail_unicode(src, bytes, pos);
        } else {
            break;
        }
    }
    pos
}

#[cold]
fn scan_ident_tail_unicode(src: &str, bytes: &[u8], mut pos: usize) -> usize {
    let len = bytes.len();
    while pos < len {
        let b = bytes[pos];
        if is_ascii_id_cont(b) {
            pos += 1;
        } else if b >= 0x80 {
            let ch = src[pos..].chars().next().unwrap();
            if is_ident_continue_char(ch) {
                pos += ch.len_utf8();
            } else {
                break;
            }
        } else {
            break;
        }
    }
    pos
}

/// Checks whether a raw identifier name (`r#<ident>`) is forbidden in Rust (`_`, `self`,
/// `Self`, `super`, `crate`).
#[inline]
pub(crate) fn is_forbidden_raw_ident(ident: &str) -> bool {
    matches!(ident, "_" | "self" | "Self" | "super" | "crate")
}
