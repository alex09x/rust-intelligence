/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Trivia scanners: whitespace, line comments, nested block comments, and shebang.

use crate::char_class::{is_ascii_ws, is_forbidden_raw_ident, is_ident_start_char, is_whitespace_char, scan_ident_tail};
use crate::keyword::classify_ident;
use crate::scan_char::{scan_byte_char, scan_char_or_lifetime};
use crate::scan_lit::{scan_quoted_str, scan_raw_str};
use crate::token::{TokenKind, HAS_ERROR};

/// Scans one or more consecutive whitespace characters starting at `start`.
/// Caller guarantees `start < bytes.len()` and the character at `start` is whitespace.
#[inline]
pub(crate) fn scan_whitespace(src: &str, bytes: &[u8], start: usize) -> usize {
    let mut pos = start;
    let len = bytes.len();
    while pos + 4 <= len {
        if is_ascii_ws(bytes[pos]) && is_ascii_ws(bytes[pos + 1]) && is_ascii_ws(bytes[pos + 2]) && is_ascii_ws(bytes[pos + 3]) {
            pos += 4;
        } else {
            break;
        }
    }
    while pos < len {
        let b = bytes[pos];
        if is_ascii_ws(b) {
            pos += 1;
        } else if b == 0xC2 || b == 0xE2 {
            // All non-ASCII Rust whitespace (`\u{0085}`, `\u{200E}`, `\u{200F}`,
            // `\u{2028}`, `\u{2029}`) begins with UTF-8 lead byte 0xC2 or 0xE2.
            let ch = src[pos..].chars().next().unwrap();
            if is_whitespace_char(ch) {
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

/// Scans a `//` line comment starting at `start` (`bytes[start..start+2] == b"//"`),
/// stopping immediately before `\n` (or at EOF).
#[inline]
pub(crate) fn scan_line_comment(bytes: &[u8], start: usize) -> usize {
    let body = start + 2;
    match bytes[body..].iter().position(|&b| b == b'\n') {
        Some(off) => body + off,
        None => bytes.len(),
    }
}

/// Scans a `/* ... */` block comment starting at `start` (`bytes[start..start+2] == b"/*"`),
/// supporting arbitrary Rust nesting depth (`/* outer /* inner */ outer */`).
/// Sets [`HAS_ERROR`] if EOF is reached with unclosed nesting (`depth > 0`).
pub(crate) fn scan_block_comment(bytes: &[u8], start: usize) -> (u8, usize) {
    let mut pos = start + 2;
    let mut depth = 1usize;
    while depth > 0 {
        let Some(off) = bytes[pos..].iter().position(|&b| b == b'*' || b == b'/') else {
            return (HAS_ERROR, bytes.len());
        };
        pos += off;
        if bytes[pos] == b'/' {
            if pos + 1 < bytes.len() && bytes[pos + 1] == b'*' {
                depth += 1;
                pos += 2;
            } else {
                pos += 1;
            }
        } else if pos + 1 < bytes.len() && bytes[pos + 1] == b'/' {
            depth -= 1;
            pos += 2;
        } else {
            pos += 1;
        }
    }
    (0, pos)
}

/// Checks whether `#!` at the start of the file (byte 0, or immediately after a UTF-8 BOM)
/// is a valid Rust shebang rather than an inner attribute `#![...]`.
///
/// Following `rustc_lexer::strip_shebang`, `#!` is a shebang iff the first non-whitespace,
/// non-comment character after `#!` is not `[`.
pub(crate) fn try_scan_shebang(src: &str, bytes: &[u8], start: usize) -> Option<(TokenKind, usize)> {
    if start + 2 > bytes.len() || bytes[start] != b'#' || bytes[start + 1] != b'!' {
        return None;
    }

    let mut look = start + 2;
    while look < bytes.len() {
        let b = bytes[look];
        if is_ascii_ws(b) {
            look = scan_whitespace(src, bytes, look);
            continue;
        }
        if b >= 0x80 {
            let ch = src[look..].chars().next().unwrap();
            if is_whitespace_char(ch) {
                look += ch.len_utf8();
                continue;
            }
            break;
        }
        if b == b'/' && look + 1 < bytes.len() {
            if bytes[look + 1] == b'/' {
                look = scan_line_comment(bytes, look);
                continue;
            }
            if bytes[look + 1] == b'*' {
                let (flags, end) = scan_block_comment(bytes, look);
                if (flags & HAS_ERROR) != 0 {
                    // Unclosed block comment: let normal lexing surface the error on `/*`.
                    return None;
                }
                look = end;
                continue;
            }
        }
        break;
    }

    if look < bytes.len() && bytes[look] == b'[' {
        return None;
    }

    let line_end = match bytes[start + 2..].iter().position(|&b| b == b'\n') {
        Some(off) => start + 2 + off,
        None => bytes.len(),
    };
    Some((TokenKind::Shebang, line_end))
}

pub(crate) fn find_matching_delim(src: &str, bytes: &[u8], open_pos: usize, open_b: u8, close_b: u8, max_pos: usize) -> Option<usize> {
    let len = bytes.len();
    let (mut pos, mut depth) = (open_pos + 1, 1u32);
    while pos < max_pos {
        match bytes[pos] {
            b if b == open_b => (depth += 1, pos += 1).1,
            b if b == close_b => {
                depth -= 1;
                if depth == 0 {
                    return Some(pos);
                }
                pos += 1;
            }
            b'/' if pos + 1 < len && bytes[pos + 1] == b'/' => pos = scan_line_comment(bytes, pos),
            b'/' if pos + 1 < len && bytes[pos + 1] == b'*' => {
                let (flags, end) = scan_block_comment(bytes, pos);
                if flags != 0 {
                    return None;
                }
                pos = end;
            }
            b'"' => {
                let (_, flags, end) = scan_quoted_str(src, bytes, pos, TokenKind::Str);
                if flags != 0 {
                    return None;
                }
                pos = end;
            }
            b'\'' => {
                let (_, flags, end) = scan_char_or_lifetime(src, bytes, pos);
                if flags != 0 {
                    return None;
                }
                pos = end;
            }
            b'r' if pos + 1 < len && matches!(bytes[pos + 1], b'"' | b'#') => {
                let (_, flags, end) = scan_r_prefix_or_ident(src, bytes, pos);
                if flags != 0 {
                    return None;
                }
                pos = end;
            }
            _ => pos += 1,
        }
    }
    None
}

pub(crate) enum SkipItem {
    Collapsed(usize, usize, u8),
    ContainsTarget,
    Bail,
}

pub(crate) fn skip_item_from_kw(src: &str, bytes: &[u8], after_kw: usize, kind: TokenKind, max_pos: usize) -> SkipItem {
    let len = bytes.len();
    let (mut pos, mut pb, mut prev_eq, mut prev_lt, mut saw_parens) = (after_kw, 0u32, false, false, kind != TokenKind::Fn);
    // `<…>` depth: a `{ … }` inside it is a const argument (`Matrix<T, { 1 + 1 }>`), not the body.
    let mut angle = 0u32;
    while pos < max_pos {
        match bytes[pos] {
            b'/' if pos + 1 < len && bytes[pos + 1] == b'/' => pos = scan_line_comment(bytes, pos),
            b'/' if pos + 1 < len && bytes[pos + 1] == b'*' => {
                let (flags, end) = scan_block_comment(bytes, pos);
                if flags != 0 {
                    return SkipItem::Bail;
                }
                pos = end;
            }
            b'"' => {
                let (_, flags, end) = scan_quoted_str(src, bytes, pos, TokenKind::Str);
                if flags != 0 {
                    return SkipItem::Bail;
                }
                (prev_eq, prev_lt) = (false, false);
                pos = end;
            }
            b'\'' => {
                let (_, flags, end) = scan_char_or_lifetime(src, bytes, pos);
                if flags != 0 {
                    return SkipItem::Bail;
                }
                (prev_eq, prev_lt) = (false, false);
                pos = end;
            }
            b'r' if pos + 1 < len && matches!(bytes[pos + 1], b'"' | b'#') => {
                let (_, flags, end) = scan_r_prefix_or_ident(src, bytes, pos);
                if flags != 0 {
                    return SkipItem::Bail;
                }
                (prev_eq, prev_lt) = (false, false);
                pos = end;
            }
            b'(' => {
                if pb == 0 {
                    saw_parens = true;
                }
                pb += 1;
                (prev_eq, prev_lt) = (false, false);
                pos += 1;
            }
            b'[' => {
                pb += 1;
                (prev_eq, prev_lt) = (false, false);
                pos += 1;
            }
            b')' | b']' => {
                pb = pb.saturating_sub(1);
                (prev_eq, prev_lt) = (false, false);
                pos += 1;
            }
            b';' if pb == 0 => {
                return if saw_parens && !matches!(kind, TokenKind::Enum | TokenKind::Impl | TokenKind::Trait) { SkipItem::Collapsed(pos, pos, b';') } else { SkipItem::Bail };
            }
            b'{' if pb == 0 => {
                if kind == TokenKind::Use || prev_eq || prev_lt || angle > 0 {
                    let Some(close) = find_matching_delim(src, bytes, pos, b'{', b'}', max_pos) else {
                        return if max_pos < len { SkipItem::ContainsTarget } else { SkipItem::Bail };
                    };
                    (prev_eq, prev_lt) = (false, false);
                    pos = close + 1;
                } else if saw_parens && !matches!(kind, TokenKind::Const | TokenKind::Static | TokenKind::Type) {
                    return match find_matching_delim(src, bytes, pos, b'{', b'}', max_pos) {
                        Some(close) => SkipItem::Collapsed(pos, close, b'}'),
                        None if max_pos < len => SkipItem::ContainsTarget,
                        None => SkipItem::Bail,
                    };
                } else {
                    return SkipItem::Bail;
                }
            }
            b'}' if pb == 0 => return SkipItem::Bail,
            b'=' if pb == 0 => {
                (prev_eq, prev_lt) = (true, false);
                pos += 1;
            }
            b'<' if pb == 0 => {
                (prev_eq, prev_lt, angle) = (false, true, angle + 1);
                pos += 1;
            }
            b'>' if pb == 0 => {
                if pos == 0 || bytes[pos - 1] != b'-' {
                    angle = angle.saturating_sub(1);
                }
                (prev_eq, prev_lt) = (false, false);
                pos += 1;
            }
            b' ' | b'\t' | b'\n' | b'\r' | 0x0B | 0x0C => pos += 1,
            _ => {
                (prev_eq, prev_lt) = (false, false);
                pos += 1;
            }
        }
    }
    if max_pos < len && (kind != TokenKind::Const || prev_eq) {
        SkipItem::ContainsTarget
    } else {
        SkipItem::Bail
    }
}

#[inline]
pub(crate) fn scan_r_prefix_or_ident(src: &str, bytes: &[u8], start: usize) -> (TokenKind, u8, usize) {
    let mut p = start + 1;
    if p >= bytes.len() || (bytes[p] != b'#' && bytes[p] != b'"') {
        let end = scan_ident_tail(src, bytes, p);
        return (classify_ident(&src[start..end]), 0, end);
    }
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
            let flags = if is_forbidden_raw_ident(&src[p..end]) { HAS_ERROR } else { 0 };
            return (TokenKind::RawIdent, flags, end);
        }
    }
    let end = scan_ident_tail(src, bytes, start + 1);
    (classify_ident(&src[start..end]), 0, end)
}

#[inline]
pub(crate) fn scan_b_prefix_or_ident(src: &str, bytes: &[u8], start: usize) -> (TokenKind, u8, usize) {
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
                    return scan_raw_str(src, bytes, p, p - (start + 2), TokenKind::RawByteStr);
                }
            }
            _ => {}
        }
    }
    let end = scan_ident_tail(src, bytes, start + 1);
    (classify_ident(&src[start..end]), 0, end)
}

#[inline]
pub(crate) fn scan_c_prefix_or_ident(src: &str, bytes: &[u8], start: usize) -> (TokenKind, u8, usize) {
    if start + 1 < bytes.len() {
        match bytes[start + 1] {
            b'"' => return scan_quoted_str(src, bytes, start + 1, TokenKind::CStr),
            b'r' => {
                let mut p = start + 2;
                while p < bytes.len() && bytes[p] == b'#' {
                    p += 1;
                }
                if p < bytes.len() && bytes[p] == b'"' {
                    return scan_raw_str(src, bytes, p, p - (start + 2), TokenKind::RawCStr);
                }
            }
            _ => {}
        }
    }
    let end = scan_ident_tail(src, bytes, start + 1);
    (classify_ident(&src[start..end]), 0, end)
}

#[cold]
pub(crate) fn scan_non_ascii(src: &str, bytes: &[u8], start: usize) -> (TokenKind, u8, usize) {
    let ch = src[start..].chars().next().unwrap();
    let after_first = start + ch.len_utf8();
    if is_whitespace_char(ch) {
        return (TokenKind::Whitespace, 0, scan_whitespace(src, bytes, after_first));
    }
    if is_ident_start_char(ch) {
        return (TokenKind::Ident, 0, scan_ident_tail(src, bytes, after_first));
    }
    (TokenKind::Unknown, HAS_ERROR, after_first)
}
