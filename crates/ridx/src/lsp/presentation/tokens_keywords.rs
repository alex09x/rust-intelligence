/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! The token of a reference whose source spelling is a keyword: `Self` names the implementing type through the
//! keyword (a type token, not the struct's), and `self` / `super` / `crate` in a path are keywords, not modules.

use super::tokens::{defkind_to_token, TOKEN_TYPE_KEYWORD, TOKEN_TYPE_TYPE};
use ridx_format::DefKind;

/// The text of `line` from UTF-16 column `col` for `len` UTF-16 units, when it lies on character boundaries.
fn spelled(line: &str, col: u32, len: u32) -> Option<&str> {
    let (mut at, mut start, mut end) = (0u32, None, None);
    for (b, ch) in line.char_indices() {
        if at == col {
            start = Some(b);
        }
        if at == col + len {
            end = Some(b);
            break;
        }
        at += ch.len_utf16() as u32;
    }
    if at == col + len && end.is_none() {
        end = Some(line.len());
    }
    line.get(start?..end?)
}

/// Token type of a reference of `kind` spelled at `col..col + len` (UTF-16) on `line`.
pub(super) fn reference_token(line: Option<&str>, col: u32, len: u32, kind: DefKind) -> u32 {
    match line.and_then(|l| spelled(l, col, len)) {
        Some("Self") => TOKEN_TYPE_TYPE,
        Some("self" | "super" | "crate") if kind == DefKind::Mod => TOKEN_TYPE_KEYWORD,
        _ => defkind_to_token(kind).0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lsp::presentation::tokens::{TOKEN_TYPE_NAMESPACE, TOKEN_TYPE_STRUCT};

    #[test]
    fn keywords_in_paths_are_keyword_tokens() {
        let line = "use super::*; fn f() -> Self { crate::a::Kind::new() }";
        assert_eq!(reference_token(Some(line), 4, 5, DefKind::Mod), TOKEN_TYPE_KEYWORD);
        assert_eq!(reference_token(Some(line), 24, 4, DefKind::Struct), TOKEN_TYPE_TYPE, "`Self` is the type keyword");
        assert_eq!(reference_token(Some(line), 31, 5, DefKind::Mod), TOKEN_TYPE_KEYWORD);
        assert_eq!(reference_token(Some(line), 38, 1, DefKind::Mod), TOKEN_TYPE_NAMESPACE);
        assert_eq!(reference_token(Some(line), 41, 4, DefKind::Struct), TOKEN_TYPE_STRUCT);
        assert_eq!(reference_token(None, 0, 4, DefKind::Struct), TOKEN_TYPE_STRUCT);
        assert_eq!(reference_token(Some("    Unit(1)"), 4, 4, DefKind::StructCtor), TOKEN_TYPE_STRUCT, "a tuple struct's constructor");
    }

    #[test]
    fn a_shorthand_field_reads_as_the_field() {
        use crate::lsp::presentation::tokens::{filter_non_overlapping, RawToken, TOKEN_TYPE_PROPERTY, TOKEN_TYPE_VARIABLE};
        let tok = |token_type| RawToken { line: 3, col: 11, length: 4, token_type, token_modifiers: 0 };
        let kept = filter_non_overlapping(vec![tok(TOKEN_TYPE_VARIABLE), tok(TOKEN_TYPE_PROPERTY)]);
        assert_eq!(kept.iter().map(|t| t.token_type).collect::<Vec<_>>(), vec![TOKEN_TYPE_PROPERTY]);
    }
}
