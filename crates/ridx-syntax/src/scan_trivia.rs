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

use crate::char_class::{is_ascii_ws, is_whitespace_char};
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
