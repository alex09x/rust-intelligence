/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Position mapping and coordinate conversions between UTF-8 bytes, UTF-16 code units, and line/columns.

use super::DocumentOverlay;

impl DocumentOverlay {
    /// Converts a 0-based UTF-16 character column on a 1-based line to a 1-based Unicode character column.
    pub fn utf16_col_to_char_col(&self, line_1based: u32, utf16_col_0based: u32) -> u32 {
        let line_idx = line_1based.saturating_sub(1) as usize;
        if line_idx >= self.line_starts.len() {
            return utf16_col_0based + 1;
        }
        let start = self.line_starts[line_idx];
        let end = if line_idx + 1 < self.line_starts.len() { self.line_starts[line_idx + 1] } else { self.text.len() };
        let line_str = &self.text[start..end];

        let mut utf16_count = 0;
        let mut char_count = 0;
        for c in line_str.chars() {
            if utf16_count >= utf16_col_0based {
                break;
            }
            utf16_count += c.len_utf16() as u32;
            char_count += 1;
        }
        char_count + 1
    }

    /// Converts a 1-based Unicode character column on a 1-based line to a 0-based UTF-16 character column.
    pub fn char_col_to_utf16_col(&self, line_1based: u32, char_col_1based: u32) -> u32 {
        let line_idx = line_1based.saturating_sub(1) as usize;
        if line_idx >= self.line_starts.len() {
            return char_col_1based.saturating_sub(1);
        }
        let start = self.line_starts[line_idx];
        let end = if line_idx + 1 < self.line_starts.len() { self.line_starts[line_idx + 1] } else { self.text.len() };
        let line_str = &self.text[start..end];

        let target_char = char_col_1based.saturating_sub(1);
        let mut utf16_count = 0;
        for (i, c) in line_str.chars().enumerate() {
            if c == '\n' || c == '\r' || (i as u32) >= target_char {
                break;
            }
            utf16_count += c.len_utf16() as u32;
        }
        utf16_count
    }

    /// Converts a byte offset to 1-based line and 1-based character column.
    #[allow(dead_code)]
    pub fn byte_to_char_line_col(&self, byte_offset: usize) -> (u32, u32) {
        let b = byte_offset.min(self.text.len());
        let line_idx = match self.line_starts.binary_search(&b) {
            Ok(idx) => idx,
            Err(idx) => idx.saturating_sub(1),
        };
        let line = (line_idx + 1) as u32;
        let line_start_byte = self.line_starts[line_idx];
        let line_prefix = &self.text[line_start_byte..b];
        let char_col = (line_prefix.chars().count() + 1) as u32;
        (line, char_col)
    }

    /// Converts 1-based line and 1-based character column to byte offset.
    pub fn line_char_col_to_byte(&self, line: u32, col: u32) -> usize {
        let line_idx = line.saturating_sub(1) as usize;
        if line_idx >= self.line_starts.len() {
            return self.text.len();
        }
        let start = self.line_starts[line_idx];
        let end = if line_idx + 1 < self.line_starts.len() { self.line_starts[line_idx + 1] } else { self.text.len() };
        let line_str = &self.text[start..end];

        let target_char = col.saturating_sub(1) as usize;
        let mut byte_offset = start;
        for (i, c) in line_str.chars().enumerate() {
            if c == '\n' || c == '\r' || i >= target_char {
                break;
            }
            byte_offset += c.len_utf8();
        }
        byte_offset
    }

    /// Converts a 0-based LSP line and 0-based UTF-16 character column to a byte offset.
    pub fn position_to_byte_offset(&self, line_0based: u32, utf16_col_0based: u32) -> usize {
        let line_idx = line_0based as usize;
        if line_idx >= self.line_starts.len() {
            return self.text.len();
        }
        let start = self.line_starts[line_idx];
        let end = if line_idx + 1 < self.line_starts.len() { self.line_starts[line_idx + 1] } else { self.text.len() };
        let line_str = &self.text[start..end];

        let mut cur_utf16 = 0;
        let mut byte_offset = start;
        for c in line_str.chars() {
            if c == '\n' || c == '\r' || cur_utf16 >= utf16_col_0based {
                break;
            }
            cur_utf16 += c.len_utf16() as u32;
            byte_offset += c.len_utf8();
        }
        byte_offset
    }

    /// Converts a byte offset to 0-based LSP line and 0-based UTF-16 character column.
    pub fn byte_offset_to_position(&self, byte_offset: usize) -> (u32, u32) {
        let b = byte_offset.min(self.text.len());
        let line_idx = match self.line_starts.binary_search(&b) {
            Ok(idx) => idx,
            Err(idx) => idx.saturating_sub(1),
        };
        let line_0based = line_idx as u32;
        let line_start_byte = self.line_starts[line_idx];
        let line_prefix = &self.text[line_start_byte..b];
        let utf16_col = line_prefix.encode_utf16().count() as u32;
        (line_0based, utf16_col)
    }
}
