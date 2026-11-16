/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Line mapping and whitespace/comment stripping for document overlays.

use super::{DocumentOverlay, LineMap};

pub fn strip_trailing_whitespace_and_comments(s: &str) -> &str {
    let bytes = s.as_bytes();
    let mut cur = bytes.len();

    loop {
        while cur > 0 && bytes[cur - 1].is_ascii_whitespace() {
            cur -= 1;
        }

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

        let mut line_start = cur;
        while line_start > 0 && bytes[line_start - 1] != b'\n' {
            line_start -= 1;
        }
        let line_prefix = &bytes[line_start..cur];
        if let Some(pos) = line_prefix.windows(2).position(|w| w == b"//") {
            cur = line_start + pos;
            continue;
        }

        break;
    }

    &s[..cur]
}

pub fn strip_leading_whitespace_and_comments(s: &str) -> &str {
    let bytes = s.as_bytes();
    let mut cur = 0;

    loop {
        while cur < bytes.len() && bytes[cur].is_ascii_whitespace() {
            cur += 1;
        }

        if cur + 2 <= bytes.len() && bytes[cur] == b'/' && bytes[cur + 1] == b'/' {
            while cur < bytes.len() && bytes[cur] != b'\n' {
                cur += 1;
            }
            continue;
        }

        if cur + 2 <= bytes.len() && bytes[cur] == b'/' && bytes[cur + 1] == b'*' {
            let mut depth = 1usize;
            cur += 2;
            while cur + 2 <= bytes.len() {
                if bytes[cur] == b'/' && bytes[cur + 1] == b'*' {
                    depth += 1;
                    cur += 2;
                } else if bytes[cur] == b'*' && bytes[cur + 1] == b'/' {
                    depth -= 1;
                    cur += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    cur += 1;
                }
            }
            continue;
        }

        break;
    }

    &s[cur..]
}

pub fn is_byte_in_string_or_comment(line_str: &str, target_byte: usize) -> bool {
    let bytes = line_str.as_bytes();
    let mut i = 0;
    let len = bytes.len();
    while i < len {
        let b = bytes[i];
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'/' {
            return target_byte >= i;
        }
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
            let start = i;
            i += 2;
            let mut closed = false;
            while i + 1 < len {
                if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                    i += 2;
                    closed = true;
                    break;
                }
                i += 1;
            }
            let end = if closed { i } else { len };
            if target_byte >= start && target_byte < end {
                return true;
            }
            continue;
        }
        if b == b'\'' {
            // Distinguish lifetime (e.g. 'a, 'static) from char literal (e.g. 'c', '\n')
            let mut is_lifetime = false;
            if i + 1 < len && (bytes[i + 1] == b'_' || bytes[i + 1].is_ascii_alphabetic()) {
                let mut scan = i + 1;
                while scan < len && (bytes[scan] == b'_' || bytes[scan].is_ascii_alphanumeric()) {
                    scan += 1;
                }
                if scan >= len || bytes[scan] != b'\'' {
                    is_lifetime = true;
                }
            }
            if is_lifetime {
                i += 1;
                continue;
            }
        }
        if b == b'"' || b == b'\'' {
            let quote = b;
            let start = i;
            i += 1;
            let mut closed = false;
            while i < len {
                if bytes[i] == b'\\' {
                    i = (i + 2).min(len);
                    continue;
                }
                if bytes[i] == quote {
                    i += 1;
                    closed = true;
                    break;
                }
                i += 1;
            }
            let end = if closed { i } else { len };
            if target_byte >= start && target_byte < end {
                return true;
            }
            continue;
        }
        i += 1;
    }
    false
}

impl DocumentOverlay {
    /// Line correspondence between the compiled baseline and the live text; `None` while the
    /// buffer has no known baseline. Callers that map many lines build it once.
    ///
    /// A buffer with no known baseline that matches the file on disk (a std or dependency source opened after a
    /// jump into it: the workspace index records no hash for it) maps line for line, as the file would if it were
    /// not open. A workspace file whose recorded hash matches neither the buffer nor the disk is stale: no map.
    pub fn line_map(&self) -> Option<LineMap> {
        let baseline = self.compiled_text.as_deref().or_else(|| (self.unhashed_source && self.is_synced_with_disk()).then_some(self.text.as_str()))?;
        Some(LineMap::new(baseline, &self.text))
    }

    pub fn map_line_to_compiled(&self, live_line: u32) -> Option<u32> {
        self.line_map()?.to_baseline(live_line)
    }

    pub fn map_compiled_line_to_live(&self, compiled_line: u32) -> Option<u32> {
        self.line_map()?.to_live(compiled_line)
    }

    pub fn map_live_to_compiled(&self, line: u32, col: u32) -> Option<(u32, u32)> {
        let cl = self.map_line_to_compiled(line)?;
        Some((cl, col))
    }

    pub fn map_compiled_to_live(&self, compiled_line: u32, compiled_col: u32) -> Option<(u32, u32)> {
        let ll = self.map_compiled_line_to_live(compiled_line)?;
        Some((ll, compiled_col))
    }

    pub fn has_modified_imports(&self) -> bool {
        if self.compiled_text.as_deref() == Some(&self.text) {
            return false;
        }
        if !self.is_dirty() {
            return false;
        }
        let Some(compiled_text) = self.compiled_text.as_ref() else {
            return !extract_normalized_imports(&self.text).is_empty();
        };
        extract_normalized_imports(&self.text) != extract_normalized_imports(compiled_text)
    }

    /// Whether the name at `line:col` (1-based) may resolve otherwise than the index recorded because of an import
    /// edited since: its own `use` changed, a glob changed, or there is no baseline to compare with. An import of other
    /// names (a `use` added in `mod tests`) changes nothing for it.
    ///
    /// A name in a path depends on the path's first segment too (`models::Item` after `use v2::models`), and a name
    /// after `.` is a method or field, which an imported trait (`use a::Trait as _;` too) may provide.
    pub fn imports_changed_at(&self, line: u32, col: u32) -> bool {
        if !self.has_modified_imports() {
            return false;
        }
        let Some(name) = self.get_ident_at(line, col) else { return true };
        let line_text = self.text.lines().nth(line.saturating_sub(1) as usize).unwrap_or("");
        let segments = path_segments_at(line_text, col);
        let after_dot = segments.len() <= 1 && method_position(line_text, col);
        let Some(changed) = self.changed_import_names() else { return true };
        let hit = |n: &str| changed.contains(n.trim_start_matches("r#"));
        hit(&name) || segments.iter().any(|s| hit(s)) || after_dot && !changed.is_empty()
    }

    /// Whether an import edited since the index was built may change what `name` resolves to in this file.
    pub fn imports_changed_for(&self, name: &str) -> bool {
        if !self.has_modified_imports() {
            return false;
        }
        self.changed_import_names().is_none_or(|changed| changed.contains(name.trim_start_matches("r#")))
    }

    /// The names whose imports differ between the compiled text and the buffer (`_` for an anonymous trait import);
    /// `None` when any name may have changed: no baseline, or a glob import changed.
    fn changed_import_names(&self) -> Option<std::collections::HashSet<String>> {
        let compiled = self.compiled_text.as_deref()?;
        let ((names_before, globs_before), (names_now, globs_now)) = (super::imports::imported_names(compiled), super::imports::imported_names(&self.text));
        if globs_before != globs_now {
            return None;
        }
        Some(names_before.symmetric_difference(&names_now).map(|(n, _)| n.trim_start_matches("r#").to_string()).collect())
    }
}

/// The segments of the path whose name is at 1-based char `col` of `line`, up to that name (`a::b::C` on `C` gives
/// `a`, `b`, `C`).
fn path_segments_at(line: &str, col: u32) -> Vec<String> {
    let chars: Vec<char> = line.chars().collect();
    let is_word = |c: char| c.is_alphanumeric() || c == '_' || c == '#';
    let mut end = (col.saturating_sub(1) as usize).min(chars.len());
    while end < chars.len() && is_word(chars[end]) {
        end += 1;
    }
    let mut start = end;
    while start > 0 && (is_word(chars[start - 1]) || chars[start - 1] == ':') {
        start -= 1;
    }
    chars[start..end].iter().collect::<String>().split("::").filter(|s| !s.is_empty()).map(str::to_string).collect()
}

/// Whether the name at 1-based char `col` of `line` follows a `.`: a method or field.
fn method_position(line: &str, col: u32) -> bool {
    let chars: Vec<char> = line.chars().collect();
    let mut s = (col.saturating_sub(1) as usize).min(chars.len());
    while s > 0 && (chars[s - 1].is_alphanumeric() || chars[s - 1] == '_') {
        s -= 1;
    }
    chars[..s].iter().rev().find(|c| !c.is_whitespace()) == Some(&'.')
}

pub use super::imports::extract_normalized_imports;

pub fn compute_line_starts(text: &str) -> Vec<usize> {
    let mut starts = vec![0];
    for (i, b) in text.as_bytes().iter().enumerate() {
        if *b == b'\n' {
            starts.push(i + 1);
        }
    }
    starts
}

/// Maps a 1-based line number from baseline text to live text based on prefix and suffix diffs.
/// Single-line form of [`LineMap`]; callers that map many lines of one text pair build the map
/// once instead.
pub fn map_text_line_to_live(baseline_text: &str, live_text: &str, baseline_line: u32) -> Option<u32> {
    LineMap::new(baseline_text, live_text).to_live(baseline_line)
}
