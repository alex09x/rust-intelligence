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
        Some(o) => o.text.lines().nth(line0 as usize).map(str::to_string),
        None => conv.get_line(&file, line0 + 1).map(str::to_string),
    };
    text.is_some_and(|t| off_token_in(&t, col16))
}

/// `is_off_token` on the line `line` at the UTF-16 column `col16`.
fn off_token_in(line: &str, col16: u32) -> bool {
    let (mut units, mut before, mut at) = (0u32, None, None);
    for c in line.chars() {
        if units >= col16 {
            at = Some(c);
            break;
        }
        units += c.len_utf16() as u32;
        before = Some(c);
    }
    at.is_none_or(char::is_whitespace) && !before.is_some_and(|c| c.is_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_whitespace_with_no_name_before_it_is_off_token() {
        let line = "        storage_failed.push(day); ";
        assert!(off_token_in(line, 3), "indentation");
        assert!(!off_token_in(line, 8), "the name's first letter");
        assert!(!off_token_in(line, 22), "right after the name, as an editor places a cursor");
        assert!(!off_token_in(line, 22 + 1), "the method");
        assert!(off_token_in(line, line.len() as u32), "past `;` and its space");
        assert!(off_token_in("", 0), "an empty line");
        assert!(!off_token_in("x", 1), "the end of a line right after a name");
        assert!(!off_token_in("«é» x", 4), "`x` past non-ASCII characters");
        assert!(off_token_in("😀  x", 3), "a UTF-16 column past a two-unit character, in the spaces");
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
