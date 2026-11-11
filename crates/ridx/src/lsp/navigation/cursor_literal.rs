/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! A cursor inside a string literal or a comment names a symbol only where the word under it spells that symbol
//! (`format!("{value}")`). A macro may point a whole literal at an item it generated code for
//! (`#[serde(skip_serializing_if = "Option::is_none")]` resolves to `Option`): `is_none` there is no use of `Option`.

/// Whether the cursor (1-based line, 1-based char column) of `text` sits in a literal or comment on a word other
/// than `name`.
pub(super) fn literal_word_is_not(text: &str, line: u32, col: u32, name: &str) -> bool {
    let Some(offset) = byte_offset(text, line, col) else { return false };
    let masked = crate::lsp::editing::action_parse::mask_comments_and_strings(text);
    let (orig, mask) = (text.as_bytes(), masked.as_bytes());
    if offset >= orig.len() || orig[offset] == mask[offset] {
        return false;
    }
    let is_id = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let start = orig[..offset].iter().rposition(|&c| !is_id(c)).map_or(0, |p| p + 1);
    let end = orig[offset..].iter().position(|&c| !is_id(c)).map_or(orig.len(), |p| offset + p);
    let word = &text[start..end];
    word != name.strip_prefix("r#").unwrap_or(name)
}

fn byte_offset(text: &str, line: u32, col: u32) -> Option<usize> {
    let line_start = if line <= 1 { 0 } else { text.match_indices('\n').nth(line as usize - 2)?.0 + 1 };
    let rest = &text[line_start..];
    let within = rest.char_indices().nth(col.saturating_sub(1) as usize).map_or(rest.len(), |(b, _)| b);
    Some(line_start + within)
}

#[cfg(test)]
mod tests {
    use super::literal_word_is_not;

    #[test]
    fn a_literal_names_only_the_word_it_spells() {
        let src = "#[serde(skip_serializing_if = \"Option::is_none\")]\nlet s = format!(\"{value}\"); // value\nlet o: Option<u8> = None;\n";
        assert!(literal_word_is_not(src, 1, 41, "Option"), "`is_none` in the literal is no use of `Option`");
        assert!(!literal_word_is_not(src, 1, 33, "Option"), "`Option` spelled in the literal");
        assert!(!literal_word_is_not(src, 2, 19, "value"), "an inline format argument");
        assert!(!literal_word_is_not(src, 3, 9, "Option"), "code outside literals is not judged here");
    }
}
