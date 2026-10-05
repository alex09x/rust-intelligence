/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Line mapping, byte-to-column conversions, and whitespace/comment stripping for document overlays.

use super::DocumentOverlay;

pub fn strip_trailing_whitespace_and_comments(s: &str) -> &str {
    let bytes = s.as_bytes();
    let mut cur = bytes.len();

    loop {
        // 1. Skip trailing whitespace
        while cur > 0 && bytes[cur - 1].is_ascii_whitespace() {
            cur -= 1;
        }

        // 2. Check for block comment ending at cur: "*/"
        if cur >= 2 && bytes[cur - 2] == b'*' && bytes[cur - 1] == b'/' {
            let mut depth = 1usize;
            let mut scan = cur - 2;
            let mut found_start = None;
            while scan >= 2 {
                if bytes[scan - 2] == b'*' && bytes[scan - 1] == b'/' {
                    depth += 1;
                    scan -= 2;
                } else if bytes[scan - 2] == b'/' && bytes[scan - 1] == b'*' {
                    depth -= 1;
                    scan -= 2;
                    if depth == 0 {
                        found_start = Some(scan);
                        break;
                    }
                } else {
                    scan -= 1;
                }
            }
            if let Some(start) = found_start {
                cur = start;
                continue;
            }
        }

        // 3. Check if current line contains a line comment "//" outside of strings/chars
        if cur > 0 {
            let line_start = bytes[..cur]
                .iter()
                .rposition(|&b| b == b'\n')
                .map(|p| p + 1)
                .unwrap_or(0);
            let line_slice = &bytes[line_start..cur];
            let mut in_quote: Option<u8> = None;
            let mut line_cur = 0;
            let mut comment_start = None;
            while line_cur < line_slice.len() {
                let b = line_slice[line_cur];
                if let Some(q) = in_quote {
                    if b == b'\\' && line_cur + 1 < line_slice.len() {
                        line_cur += 2;
                        continue;
                    }
                    if b == q {
                        in_quote = None;
                    }
                    line_cur += 1;
                } else if b == b'"' || b == b'\'' {
                    in_quote = Some(b);
                    line_cur += 1;
                } else if b == b'/'
                    && line_cur + 1 < line_slice.len()
                    && line_slice[line_cur + 1] == b'/'
                {
                    comment_start = Some(line_start + line_cur);
                    break;
                } else {
                    line_cur += 1;
                }
            }
            if let Some(cs) = comment_start {
                cur = cs;
                continue;
            }
        }

        break;
    }

    &s[..cur]
}

pub fn strip_leading_whitespace_and_comments(s: &str) -> &str {
    let bytes = s.as_bytes();
    let len = bytes.len();
    let mut cur = 0;

    loop {
        // 1. Skip leading whitespace
        while cur < len && bytes[cur].is_ascii_whitespace() {
            cur += 1;
        }

        // 2. Check for line comment: "//"
        if cur + 1 < len && bytes[cur] == b'/' && bytes[cur + 1] == b'/' {
            cur += 2;
            while cur < len && bytes[cur] != b'\n' {
                cur += 1;
            }
            continue;
        }

        // 3. Check for block comment: "/*"
        if cur + 1 < len && bytes[cur] == b'/' && bytes[cur + 1] == b'*' {
            let mut depth = 1usize;
            let mut scan = cur + 2;
            let mut found_end = None;
            while scan + 1 < len {
                if bytes[scan] == b'/' && bytes[scan + 1] == b'*' {
                    depth += 1;
                    scan += 2;
                } else if bytes[scan] == b'*' && bytes[scan + 1] == b'/' {
                    depth -= 1;
                    scan += 2;
                    if depth == 0 {
                        found_end = Some(scan);
                        break;
                    }
                } else {
                    scan += 1;
                }
            }
            if let Some(end) = found_end {
                cur = end;
                continue;
            } else {
                cur = len;
                break;
            }
        }

        break;
    }

    &s[cur..]
}

/// Computes 0-based byte offsets for each line start in the given text.
pub fn compute_line_starts(text: &str) -> Vec<usize> {
    let mut starts = vec![0];
    for (i, &b) in text.as_bytes().iter().enumerate() {
        if b == b'\n' {
            starts.push(i + 1);
        }
    }
    starts
}

impl DocumentOverlay {
    /// Maps a 1-based line number from the live overlay buffer to the corresponding
    /// 1-based line number in the compiled disk snapshot.
    ///
    /// Returns `Some(compiled_line)` if the line is in a proven unchanged region,
    /// or `None` if the line falls in an edited, inserted, or modified region.
    #[allow(dead_code)]
    pub fn map_line_to_compiled(&self, line: u32) -> Option<u32> {
        if self.compiled_text.as_deref() == Some(&self.text) {
            return Some(line);
        }
        let compiled_text = self.compiled_text.as_ref()?;

        let curr_lines: Vec<&str> = self.text.lines().collect();
        let compiled_lines: Vec<&str> = compiled_text.lines().collect();

        let l_idx = (line.saturating_sub(1)) as usize;
        if l_idx >= curr_lines.len() {
            return None;
        }

        // Count common prefix lines
        let mut prefix_common = 0;
        while prefix_common < curr_lines.len()
            && prefix_common < compiled_lines.len()
            && curr_lines[prefix_common] == compiled_lines[prefix_common]
        {
            prefix_common += 1;
        }

        // If line is in proven unchanged prefix
        if l_idx < prefix_common {
            return Some(line);
        }

        // Count common suffix lines
        let mut suffix_common = 0;
        while suffix_common < curr_lines.len()
            && suffix_common < compiled_lines.len()
            && curr_lines[curr_lines.len() - 1 - suffix_common]
                == compiled_lines[compiled_lines.len() - 1 - suffix_common]
        {
            suffix_common += 1;
        }

        // If line is in proven unchanged suffix
        if l_idx >= curr_lines.len().saturating_sub(suffix_common) {
            let offset_from_end = curr_lines.len() - 1 - l_idx;
            let compiled_line_idx = compiled_lines.len() - 1 - offset_from_end;
            return Some((compiled_line_idx + 1) as u32);
        }

        // The line is in the modified/inserted region
        None
    }

    /// Maps a 1-based line number from the compiled index snapshot to the corresponding
    /// 1-based line number in the current live overlay buffer.
    ///
    /// Returns `Some(live_line)` if the line is in a proven unchanged region,
    /// or `None` if the line was deleted, replaced, or modified by unsaved edits.
    pub fn map_compiled_line_to_live(&self, compiled_line: u32) -> Option<u32> {
        if self.compiled_text.as_deref() == Some(&self.text) {
            return Some(compiled_line);
        }
        let compiled_text = self.compiled_text.as_ref()?;

        let curr_lines: Vec<&str> = self.text.lines().collect();
        let compiled_lines: Vec<&str> = compiled_text.lines().collect();

        let c_idx = (compiled_line.saturating_sub(1)) as usize;
        if c_idx >= compiled_lines.len() {
            return None;
        }

        // Count common prefix lines
        let mut prefix_common = 0;
        while prefix_common < curr_lines.len()
            && prefix_common < compiled_lines.len()
            && curr_lines[prefix_common] == compiled_lines[prefix_common]
        {
            prefix_common += 1;
        }

        if c_idx < prefix_common {
            return Some(compiled_line);
        }

        // Count common suffix lines
        let mut suffix_common = 0;
        while suffix_common < curr_lines.len()
            && suffix_common < compiled_lines.len()
            && curr_lines[curr_lines.len() - 1 - suffix_common]
                == compiled_lines[compiled_lines.len() - 1 - suffix_common]
        {
            suffix_common += 1;
        }

        if c_idx >= compiled_lines.len().saturating_sub(suffix_common) {
            let offset_from_end = compiled_lines.len() - 1 - c_idx;
            let live_line_idx = curr_lines.len() - 1 - offset_from_end;
            return Some((live_line_idx + 1) as u32);
        }

        None
    }

    /// Maps a 1-based (line, col) position from the live overlay buffer to compiled position.
    pub fn map_live_to_compiled(&self, line: u32, col: u32) -> Option<(u32, u32)> {
        let cl = self.map_line_to_compiled(line)?;
        Some((cl, col))
    }

    /// Maps a 1-based (line, col) position from the compiled snapshot to live buffer position.
    pub fn map_compiled_to_live(
        &self,
        compiled_line: u32,
        compiled_col: u32,
    ) -> Option<(u32, u32)> {
        let ll = self.map_compiled_line_to_live(compiled_line)?;
        Some((ll, compiled_col))
    }

    /// Converts a 0-based UTF-16 character column on a 1-based line to a 1-based Unicode character column.
    pub fn utf16_col_to_char_col(&self, line_1based: u32, utf16_col_0based: u32) -> u32 {
        let line_idx = line_1based.saturating_sub(1) as usize;
        if line_idx >= self.line_starts.len() {
            return utf16_col_0based + 1;
        }
        let start = self.line_starts[line_idx];
        let end = if line_idx + 1 < self.line_starts.len() {
            self.line_starts[line_idx + 1]
        } else {
            self.text.len()
        };
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
        let end = if line_idx + 1 < self.line_starts.len() {
            self.line_starts[line_idx + 1]
        } else {
            self.text.len()
        };
        let line_str = &self.text[start..end];

        let target_char = char_col_1based.saturating_sub(1);
        let mut utf16_count = 0;
        for (i, c) in line_str.chars().enumerate() {
            if (i as u32) >= target_char {
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
        let end = if line_idx + 1 < self.line_starts.len() {
            self.line_starts[line_idx + 1]
        } else {
            self.text.len()
        };
        let line_str = &self.text[start..end];

        let target_char = col.saturating_sub(1) as usize;
        let mut byte_offset = start;
        for (i, c) in line_str.chars().enumerate() {
            if i >= target_char {
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
        let end = if line_idx + 1 < self.line_starts.len() {
            self.line_starts[line_idx + 1]
        } else {
            self.text.len()
        };
        let line_str = &self.text[start..end];

        let mut cur_utf16 = 0;
        let mut byte_offset = start;
        for c in line_str.chars() {
            if cur_utf16 >= utf16_col_0based {
                break;
            }
            cur_utf16 += c.len_utf16() as u32;
            byte_offset += c.len_utf8();
        }
        byte_offset
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_whitespace_and_comments() {
        assert_eq!(
            strip_trailing_whitespace_and_comments("code; // comment\n  "),
            "code;"
        );
        assert_eq!(
            strip_trailing_whitespace_and_comments("code; /* block */   "),
            "code;"
        );
        assert_eq!(
            strip_trailing_whitespace_and_comments("code; /* nested /* b */ */"),
            "code;"
        );
        assert_eq!(
            strip_trailing_whitespace_and_comments("code; \"// in quote\""),
            "code; \"// in quote\""
        );

        assert_eq!(
            strip_leading_whitespace_and_comments("  // comment\ncode;"),
            "code;"
        );
        assert_eq!(
            strip_leading_whitespace_and_comments("  /* block */ code;"),
            "code;"
        );
        assert_eq!(
            strip_leading_whitespace_and_comments("  /* nested /* b */ */ code;"),
            "code;"
        );
    }

    #[test]
    fn test_byte_and_char_conversions() {
        let code = "fn test() {\n    let привет = \"🚀\";\n}\n".to_string();
        let overlay = DocumentOverlay::new("file:///test/conv.rs", "src/conv.rs", 1, code);

        let (line, col) = overlay.byte_to_char_line_col(0);
        assert_eq!(line, 1);
        assert_eq!(col, 1);

        let byte_off = overlay.line_char_col_to_byte(1, 1);
        assert_eq!(byte_off, 0);

        let byte_pos = overlay.position_to_byte_offset(0, 0);
        assert_eq!(byte_pos, 0);

        // Out of bounds checks
        assert_eq!(overlay.line_char_col_to_byte(999, 1), overlay.text.len());
        assert_eq!(overlay.position_to_byte_offset(999, 1), overlay.text.len());
    }

    #[test]
    fn test_line_mapping_compiled_and_live() {
        let saved = "line1\nline2\nline3\nline4\n".to_string();
        let mut overlay = DocumentOverlay::new("file:///test/m.rs", "src/m.rs", 1, saved.clone());
        overlay.compiled_text = Some(saved);

        // No edits -> 1:1 mapping
        assert_eq!(overlay.map_compiled_line_to_live(2), Some(2));
        assert_eq!(overlay.map_line_to_compiled(2), Some(2));

        // Edit: insert a line at line 2
        let edited = "line1\ninserted\nline2\nline3\nline4\n".to_string();
        overlay.update(2, edited);

        assert_eq!(overlay.map_compiled_line_to_live(1), Some(1));
        assert_eq!(overlay.map_compiled_line_to_live(2), Some(3));
        assert_eq!(overlay.map_line_to_compiled(2), None);
        assert_eq!(overlay.map_line_to_compiled(3), Some(2));

        // Position mapping helpers
        assert_eq!(overlay.map_compiled_to_live(2, 5), Some((3, 5)));
        assert_eq!(overlay.map_live_to_compiled(3, 5), Some((2, 5)));
        assert_eq!(overlay.map_live_to_compiled(2, 5), None);

        // Out of bounds compiled line
        assert_eq!(overlay.map_compiled_line_to_live(999), None);
        assert_eq!(overlay.map_compiled_to_live(999, 5), None);

        // Modified line returns None in map_compiled_line_to_live
        let replaced = "line1\nreplaced_line2\nline3\nline4\n".to_string();
        overlay.update(3, replaced);
        assert_eq!(overlay.map_compiled_line_to_live(2), None);
    }

    #[test]
    fn test_whitespace_and_comment_edge_cases() {
        // Unclosed block comment in strip_leading_whitespace_and_comments
        assert_eq!(
            strip_leading_whitespace_and_comments("/* unclosed comment"),
            ""
        );
        // Unclosed block comment in strip_trailing_whitespace_and_comments
        assert_eq!(
            strip_trailing_whitespace_and_comments("code /* unclosed"),
            "code /* unclosed"
        );
        // Escaped quotes in comment line
        let line_with_escaped = "let x = \"escaped \\\" quote\"; // real comment\n";
        assert_eq!(
            strip_trailing_whitespace_and_comments(line_with_escaped),
            "let x = \"escaped \\\" quote\";"
        );

        // Last line byte offset in position_to_byte_offset
        let code = "first\nsecond".to_string();
        let overlay = DocumentOverlay::new("file:///test/last.rs", "src/last.rs", 1, code);
        assert_eq!(overlay.position_to_byte_offset(1, 2), 8);

        // Multiline block comment check in strip_trailing_whitespace_and_comments (line 48)
        assert_eq!(
            strip_trailing_whitespace_and_comments("code /* line1\nline2 */"),
            "code"
        );
    }
}
