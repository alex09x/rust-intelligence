/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Unified Cursor ($0) and Span («...») DSL for test fixtures.
//!
//! Accurately computes coordinates in both:
//! 1. Unicode scalar value (char) columns (1-based in ridx-driver index);
//! 2. UTF-16 code unit columns (0-based in LSP protocol).

/// Cursor position extracted from a `$0` marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CursorPos {
    pub line: u32,
    pub col_char: u32,
    pub col_utf16: u32,
    pub byte_offset: usize,
}

/// Annotated span extracted from `«...»` markers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnnotatedSpan {
    pub text: String,
    pub start_line: u32,
    pub start_col_char: u32,
    pub start_col_utf16: u32,
    pub end_line: u32,
    pub end_col_char: u32,
    pub end_col_utf16: u32,
    pub start_byte: usize,
    pub end_byte: usize,
}

/// Result of parsing a unified fixture with optional `$0` and `«...»` markers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedFixture {
    pub clean_text: String,
    pub cursor: Option<CursorPos>,
    pub spans: Vec<AnnotatedSpan>,
}

/// Unified parser for fixtures containing `$0` cursor and/or `«...»` span markers.
///
/// Markers are stripped simultaneously, ensuring neither offsets the coordinates
/// of the other.
pub fn parse_fixture(fixture: &str) -> ParsedFixture {
    assert!(!fixture.as_bytes().windows(2).any(|w| w[0] == b'\r' && w[1] != b'\n') && !fixture.ends_with('\r'), "Lone CR ('\\r' without '\\n') is not supported in test fixtures");
    let mut clean_text = String::with_capacity(fixture.len());
    let mut cursor: Option<CursorPos> = None;
    let mut spans = Vec::new();
    let mut open_stack: Vec<(u32, u32, u32, usize)> = Vec::new();

    let mut cur_line: u32 = 0;
    let mut cur_col_char: u32 = 0;
    let mut cur_col_utf16: u32 = 0;

    let chars: Vec<(usize, char)> = fixture.char_indices().collect();
    let mut i = 0;

    while i < chars.len() {
        let (byte_idx, ch) = chars[i];

        if ch == '$' && fixture[byte_idx..].starts_with("$0") {
            assert!(cursor.is_none(), "Fixture contains multiple $0 markers");
            cursor = Some(CursorPos { line: cur_line, col_char: cur_col_char, col_utf16: cur_col_utf16, byte_offset: clean_text.len() });
            i += 2; // Skip '$' and '0'
            continue;
        }

        if ch == '«' {
            open_stack.push((cur_line, cur_col_char, cur_col_utf16, clean_text.len()));
            i += 1;
            continue;
        }

        if ch == '»' {
            let (start_line, start_col_char, start_col_utf16, start_byte) = open_stack.pop().expect("Unmatched closing span marker '»'");
            let end_byte = clean_text.len();
            let text = clean_text[start_byte..end_byte].to_string();
            spans.push(AnnotatedSpan {
                text,
                start_line,
                start_col_char,
                start_col_utf16,
                end_line: cur_line,
                end_col_char: cur_col_char,
                end_col_utf16: cur_col_utf16,
                start_byte,
                end_byte,
            });
            i += 1;
            continue;
        }

        clean_text.push(ch);

        if ch == '\n' {
            cur_line += 1;
            cur_col_char = 0;
            cur_col_utf16 = 0;
        } else if ch == '\r' {
            // Keep carriage return without advancing column counts
        } else {
            cur_col_char += 1;
            cur_col_utf16 += ch.len_utf16() as u32;
        }

        i += 1;
    }

    assert!(open_stack.is_empty(), "Unmatched opening span marker '«'");

    ParsedFixture { clean_text, cursor, spans }
}

/// Convenience parser for fixtures containing only a `$0` cursor marker.
pub fn parse_cursor(fixture: &str) -> CursorPos {
    let parsed = parse_fixture(fixture);
    parsed.cursor.expect("Fixture must contain a $0 cursor marker")
}

/// Convenience parser for fixtures containing only `«...»` span annotations.
pub fn parse_spans(fixture: &str) -> (String, Vec<AnnotatedSpan>) {
    let parsed = parse_fixture(fixture);
    (parsed.clean_text, parsed.spans)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unified_cursor_and_spans() {
        let input = "fn hello() {\n    let $0«x» = 10;\n}";
        let parsed = parse_fixture(input);
        assert_eq!(parsed.clean_text, "fn hello() {\n    let x = 10;\n}");

        let cursor = parsed.cursor.unwrap();
        assert_eq!(cursor.line, 1);
        assert_eq!(cursor.col_char, 8);
        assert_eq!(cursor.col_utf16, 8);

        assert_eq!(parsed.spans.len(), 1);
        let span = &parsed.spans[0];
        assert_eq!(span.text, "x");
        assert_eq!(span.start_line, 1);
        assert_eq!(span.start_col_char, 8);
        assert_eq!(span.start_col_utf16, 8);
        assert_eq!(span.end_col_char, 9);
        assert_eq!(span.end_col_utf16, 9);
    }

    #[test]
    fn test_unicode_and_emoji_distinction() {
        // "/* 🦀 */ fn «grüßen»() {}"
        // "/* " -> 3 chars, 3 utf-16
        // "🦀"  -> 1 char, 2 utf-16 (surrogate pair)
        // " */ fn " -> 7 chars, 7 utf-16
        // Total before «grüßen»: 3 + 1 + 7 = 11 chars. UTF-16: 3 + 2 + 7 = 12 units!
        let input = "/* 🦀 */ fn «grüßen»() {}";
        let parsed = parse_fixture(input);
        assert_eq!(parsed.clean_text, "/* 🦀 */ fn grüßen() {}");

        let span = &parsed.spans[0];
        assert_eq!(span.text, "grüßen");
        // In driver index (1-based char): start_col_char + 1 == 12
        assert_eq!(span.start_col_char, 11);
        // In LSP (0-based UTF-16): start_col_utf16 == 12
        assert_eq!(span.start_col_utf16, 12);
        // "grüßen" is 6 chars, 6 UTF-16 code units (+6 / +6)
        assert_eq!(span.end_col_char, 17);
        assert_eq!(span.end_col_utf16, 18);
    }

    #[test]
    fn test_crlf_preservation() {
        let input = "line 1\r\nline $02\r\n";
        let parsed = parse_fixture(input);
        assert!(parsed.clean_text.contains("\r\n"));
        let cursor = parsed.cursor.unwrap();
        assert_eq!(cursor.line, 1);
        assert_eq!(cursor.col_char, 5);
        assert_eq!(cursor.col_utf16, 5);
    }
}
