/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! A position request on no token. An editor asks where the mouse rests (Zed's hover, its Cmd-hover link), often
//! over indentation; the index would answer there for whatever spans the spot, as a `for` loop's `into_iter` spans
//! the whole loop body. rust-analyzer answers nothing off a token, and neither does this server.

use super::protocol::{uri_to_path, PositionConverter};
use crate::overlay::OverlayManager;
use serde_json::Value;

/// The position requests answered for the token at the position. (`prepareRename` off a token is an error, as
/// rust-analyzer's, which its handler gives.)
pub(crate) const POSITION_METHODS: [&str; 7] = [
    "textDocument/hover",
    "textDocument/definition",
    "textDocument/declaration",
    "textDocument/typeDefinition",
    "textDocument/implementation",
    "textDocument/references",
    "textDocument/documentHighlight",
];

/// Whether the request's position lies in whitespace (or past the line's end) with no name ending right before it.
/// A line that cannot be read counts as on a token: the handlers decide.
pub(crate) fn is_off_token(msg: &Value, ov: &OverlayManager, conv: &mut PositionConverter) -> bool {
    let Ok((uri, line0, col16)) = super::params::text_document_position(msg) else { return false };
    let file = uri_to_path(uri);
    let text = match ov.get(&file) {
        Some(o) => {
            // A `/* … */` comment may span lines: the buffer's parser records where each comment lies.
            let at = o.position_to_byte_offset(line0, col16);
            let plain_block = |&(s, e): &(usize, usize)| s <= at && at < e && o.text[s..].starts_with("/*") && !o.text[s..].starts_with("/**") && !o.text[s..].starts_with("/*!");
            if o.opaque.iter().any(plain_block) {
                return true;
            }
            o.text.lines().nth(line0 as usize).map(str::to_string)
        }
        None => conv.get_line(&file, line0 + 1).map(str::to_string),
    };
    text.is_some_and(|t| off_token_in(&t, col16))
}

/// Moves a request's position from just past a name onto the name's last character when nothing of a name lies under
/// it (`foo.spam|;`): rust-analyzer takes the token left of a caret, and an editor's F12 sends the caret's position.
/// Found with rust-analyzer's goto-definition fixtures run against ridx (`bench/ra_fixtures.py`), where such a caret
/// answered nothing.
pub(crate) fn snap_to_word(msg: &mut Value, ov: &OverlayManager, conv: &mut PositionConverter) {
    let Ok((uri, line0, col16)) = super::params::text_document_position(msg) else { return };
    let file = uri_to_path(uri);
    let line = match ov.get(&file) {
        Some(o) => o.text.lines().nth(line0 as usize).map(str::to_string),
        None => conv.get_line(&file, line0 + 1).map(str::to_string),
    };
    if let Some(back) = line.and_then(|l| word_end_before(&l, col16)) {
        msg["params"]["position"]["character"] = Value::from(col16 - back);
    }
}

/// The UTF-16 width of the name character right before column `col16` of `line` when no name character is at it.
fn word_end_before(line: &str, col16: u32) -> Option<u32> {
    let word = |c: char| c.is_alphanumeric() || c == '_';
    let (mut units, mut before) = (0u32, None);
    for c in line.chars() {
        if units >= col16 {
            return (!word(c)).then_some(before.filter(|&b: &char| word(b))?.len_utf16() as u32);
        }
        units += c.len_utf16() as u32;
        before = Some(c);
    }
    (units == col16).then_some(before.filter(|&b| word(b))?.len_utf16() as u32)
}

/// `is_off_token` on the line `line` at the UTF-16 column `col16`: whitespace with no name ending before it, or the
/// text of a comment (an intra-doc link `[Name]` in a doc comment excepted). The index answers inside a comment for
/// whatever expression spans it: a function body's type showed over `// …` lines.
fn off_token_in(line: &str, col16: u32) -> bool {
    let (mut units, mut before, mut at, mut at_char) = (0u32, None, None, 0usize);
    for (i, c) in line.chars().enumerate() {
        if units >= col16 {
            at = Some(c);
            at_char = i;
            break;
        }
        units += c.len_utf16() as u32;
        before = Some(c);
        at_char = i + 1;
    }
    // A caret just past a match arm's `=>` is on it: it goes to its `match`'s arm.
    let fat_arrow = before == Some('>') && line.chars().nth(at_char.wrapping_sub(2)) == Some('=');
    let blank = at.is_none_or(char::is_whitespace) && !before.is_some_and(|c| c.is_alphanumeric() || c == '_') && !fat_arrow;
    blank || in_comment_text(line, at_char)
}

/// Whether the character at index `at` lies in a `//` comment of `line` (outside string literals), and not inside
/// `[…]` of a doc comment (`///`, `//!`).
fn in_comment_text(line: &str, at: usize) -> bool {
    let chars: Vec<char> = line.chars().collect();
    let (mut i, mut in_str) = (0, false);
    while i + 1 < chars.len() && i < at {
        match chars[i] {
            '\\' if in_str => i += 1,
            '"' => in_str = !in_str,
            '/' if !in_str && chars[i + 1] == '/' => {
                let doc = matches!(chars.get(i + 2), Some('/' | '!')) && chars.get(i + 3) != Some(&'/');
                if !doc {
                    return true;
                }
                let open = chars[i..at].iter().rposition(|&c| c == '[');
                let close = chars[i..at].iter().rposition(|&c| c == ']');
                return !open.is_some_and(|o| close.is_none_or(|c| c < o));
            }
            // A `/* … */` comment within the line (one open at the line's start is the buffer parser's to find).
            '/' if !in_str && chars[i + 1] == '*' => {
                let close = (i + 2..chars.len().saturating_sub(1)).find(|&k| chars[k] == '*' && chars[k + 1] == '/').map_or(chars.len(), |k| k + 2);
                if at < close {
                    return true;
                }
                i = close;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_a_caret_just_past_a_name_is_on_it() {
        let line = "    foo.spam; Self {} é";
        assert_eq!(word_end_before(line, 12), Some(1), "`spam|;`");
        assert_eq!(word_end_before(line, 11), None, "on the name");
        assert_eq!(word_end_before(line, 18), Some(1), "`Self| {{}}`");
        assert_eq!(word_end_before(line, 8), None, "`foo.|spam`: on `s`");
        assert_eq!(word_end_before(line, 4), None, "indentation");
        assert_eq!(word_end_before(line, 23), Some(1), "the line's end after `é`");
        assert_eq!(word_end_before("x𝔸", 3), Some(2), "a name of two-unit characters");
    }

    #[test]
    fn test_whitespace_with_no_name_before_it_is_off_token() {
        let line = "        storage_failed.push(day); ";
        assert!(off_token_in(line, 3), "indentation");
        assert!(!off_token_in(line, 8), "the name's first letter");
        assert!(!off_token_in(line, 22), "right after the name, as an editor places a cursor");
        assert!(!off_token_in(line, 22 + 1), "the method");
        assert!(off_token_in(line, line.len() as u32), "past `;` and its space");
        assert!(off_token_in("", 0), "an empty line");
        assert!(!off_token_in("    0 => {}", 8), "just past `=>`");
        assert!(off_token_in("    0 >= 1", 8), "past `>=`");
        assert!(!off_token_in("x", 1), "the end of a line right after a name");
        assert!(!off_token_in("«é» x", 4), "`x` past non-ASCII characters");
        assert!(off_token_in("😀  x", 3), "a UTF-16 column past a two-unit character, in the spaces");
    }

    #[test]
    fn test_comment_text_is_off_token_but_a_doc_link_is_not() {
        let code = "    let n = v.len(); // count the items";
        assert!(!off_token_in(code, 14), "`len` before the comment");
        assert!(off_token_in(code, 26), "a word of the comment");
        assert!(off_token_in("    // Укороченный прогон", 9), "a comment line, past non-ASCII text");
        assert!(!off_token_in("    let s = \"a // b\";", 16), "`//` inside a string literal is no comment");
        let doc = "/// Returns a [`Loader`] built from the config.";
        assert!(off_token_in(doc, 6), "doc comment prose");
        assert!(!off_token_in(doc, 17), "an intra-doc link");
        assert!(off_token_in(doc, 30), "prose after the link");
        assert!(off_token_in("//// a separator, not a doc comment", 8));
        let block = "    let n = /* the count */ v.len();";
        assert!(off_token_in(block, 19), "a word of an inline block comment");
        assert!(!off_token_in(block, 31), "`len` after it");
    }

    #[test]
    fn test_a_block_comment_over_several_lines_is_off_token() {
        let mut ov = OverlayManager::new();
        let uri = "file:///tmp/off_token_block.rs";
        ov.open_or_update(uri, &uri_to_path(uri), 1, "fn f() {\n    /* counts\n       items */\n    let y = 1;\n}\n".to_string());
        let mut conv = PositionConverter::new();
        let at = |line: u32, character: u32| serde_json::json!({ "params": { "textDocument": { "uri": uri }, "position": { "line": line, "character": character } } });
        assert!(is_off_token(&at(2, 9), &ov, &mut conv), "`items` on the comment's second line");
        assert!(!is_off_token(&at(3, 8), &ov, &mut conv), "on `y`");
    }

    #[test]
    fn test_a_request_off_token_in_an_open_buffer() {
        let mut ov = OverlayManager::new();
        let uri = "file:///tmp/off_token.rs";
        ov.open_or_update(uri, &uri_to_path(uri), 1, "fn f() {\n    for x in [1] {\n        let y = x;\n    }\n}\n".to_string());
        let mut conv = PositionConverter::new();
        let at = |line: u32, character: u32| serde_json::json!({ "params": { "textDocument": { "uri": uri }, "position": { "line": line, "character": character } } });
        assert!(is_off_token(&at(2, 4), &ov, &mut conv), "indentation inside the loop");
        assert!(!is_off_token(&at(2, 12), &ov, &mut conv), "on `y`");
        assert!(!is_off_token(&serde_json::json!({ "params": {} }), &ov, &mut conv), "no position: the handler answers");
    }
}
