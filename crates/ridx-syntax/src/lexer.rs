/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Main lossless Rust lexical scanner (`lex`).

use crate::char_class::scan_ident_tail;
use crate::keyword::classify_ident;
use crate::scan_char::scan_char_or_lifetime;
use crate::scan_lit::{scan_number, scan_quoted_str};
use crate::scan_trivia::{
    find_matching_delim, scan_b_prefix_or_ident, scan_block_comment, scan_c_prefix_or_ident, scan_line_comment, scan_non_ascii, scan_r_prefix_or_ident, scan_whitespace,
    skip_item_from_kw, try_scan_shebang, SkipItem,
};
use crate::token::{Token, TokenKind, HAS_ERROR, JOINT_NEXT};

/// Tokenizes `src` into a contiguous, lossless sequence of [`Token`]s.
///
/// Every byte of `src` is covered by exactly one non-empty [`Token`], and concatenating
/// all token slices reproduces `src` identically.
pub fn lex(src: &str) -> Vec<Token> {
    if src.is_empty() {
        return Vec::new();
    }
    let mut tokens = Vec::with_capacity(src.len() / 3 + 16);
    lex_into(src, &mut tokens);
    tokens
}

/// Tokenizes `src` into caller-owned `tokens` (`0` allocations after warmup).
pub(crate) fn lex_into(src: &str, tokens: &mut Vec<Token>) {
    assert!(src.len() <= u32::MAX as usize, "ridx-syntax supports source files up to u32::MAX bytes");
    tokens.clear();
    if src.is_empty() {
        return;
    }

    let bytes = src.as_bytes();
    let len = bytes.len();
    let mut pos = 0usize;

    // Optional UTF-8 BOM (`\u{FEFF}`) at byte 0 is treated as leading whitespace trivia.
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        let ws_end = scan_whitespace(src, bytes, 3);
        tokens.push(Token::new(TokenKind::Whitespace, 0, 0, ws_end as u32));
        pos = ws_end;
    }

    // Optional shebang `#!...` at start of file (byte 0 or immediately following UTF-8 BOM).
    if (pos == 0 || pos == 3) && pos < len && bytes[pos] == b'#' {
        if let Some((kind, end)) = try_scan_shebang(src, bytes, pos) {
            tokens.push(Token::new(kind, 0, pos as u32, end as u32));
            pos = end;
        }
    }

    let mut prev_punct = false;
    while pos < len {
        let start = pos;
        let (kind, flags, end) = scan_next_token(src, bytes, start);
        let cur_punct = kind.is_punct();
        if prev_punct && cur_punct {
            tokens.last_mut().unwrap().flags |= JOINT_NEXT;
        }
        prev_punct = cur_punct;
        tokens.push(Token::new(kind, flags, start as u32, end as u32));
        pos = end;
    }
}

pub(crate) fn lex_for_offset(src: &str, target_offset: u32) -> Vec<Token> {
    if src.is_empty() {
        return Vec::new();
    }
    let bytes = src.as_bytes();
    let len = bytes.len();
    let mut tokens = Vec::with_capacity((len / 12).clamp(64, 4096));
    let mut pos = 0usize;
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        pos = scan_whitespace(src, bytes, 3);
        tokens.push(Token::new(TokenKind::Whitespace, 0, 0, pos as u32));
    }
    if (pos == 0 || pos == 3) && pos < len && bytes[pos] == b'#' {
        if let Some((kind, end)) = try_scan_shebang(src, bytes, pos) {
            tokens.push(Token::new(kind, 0, pos as u32, end as u32));
            pos = end;
        }
    }
    let (mut prev_punct, mut prev_non_trivia, mut collapse_kw) = (false, TokenKind::Whitespace, TokenKind::Whitespace);
    let (mut next_item_start, mut item_kw_start, mut kw_tok_idx, mut kw_start) = (None::<u32>, 0u32, 0usize, 0u32);
    let (mut pb_depth, mut brace_depth, mut target_item_depth) = (0u32, 0u32, 0u32);
    let (mut in_target_item, mut top_is_container, mut skip_delim_check, mut saw_eq) = (false, false, false, false);
    while pos < len {
        if pb_depth == 0 && collapse_kw == TokenKind::Whitespace && next_item_start.is_none() && (pos as u32) > target_offset && prev_non_trivia == TokenKind::RBrace {
            if brace_depth == 1 && top_is_container {
                tokens.push(Token::new(TokenKind::RBrace, 0, pos as u32, len as u32));
                break;
            }
            if brace_depth == 0 {
                tokens.push(Token::new(TokenKind::Whitespace, 0, pos as u32, len as u32));
                break;
            }
        }
        let start = pos;
        let b0 = bytes[start];
        if pb_depth == 0 && !in_target_item {
            if b0 == b'#' && start + 1 < len && bytes[start + 1] == b'[' && (start as u32) < target_offset {
                if let Some(close) = find_matching_delim(src, bytes, start + 1, b'[', b']', (target_offset as usize).min(len)) {
                    let inner = &src[start + 2..close];
                    if !inner.contains("cfg") && !inner.contains("path") && !inner.contains("macro") && !inner.contains("no_implicit") {
                        let end = close + 1;
                        prev_punct = false;
                        if let Some(last) = tokens.last_mut().filter(|t| t.is_trivia()) {
                            last.end = end as u32;
                        } else {
                            tokens.push(Token::new(TokenKind::Whitespace, 0, start as u32, end as u32));
                        }
                        pos = end;
                        continue;
                    }
                }
            }
            let is_const = matches!(collapse_kw, TokenKind::Const | TokenKind::Static);
            let try_delim = if b0 == b'{' && collapse_kw != TokenKind::Whitespace && collapse_kw != TokenKind::Bang && (is_const || prev_non_trivia != TokenKind::Eq) {
                Some((TokenKind::LBrace, TokenKind::RBrace, b'}', item_kw_start))
            } else if b0 == b'[' && is_const && saw_eq {
                Some((TokenKind::LBracket, TokenKind::RBracket, b']', item_kw_start))
            } else if (collapse_kw == TokenKind::Bang || prev_non_trivia == TokenKind::Bang) && matches!(b0, b'{' | b'(' | b'[') {
                let (ot, ct, cb) = match b0 {
                    b'{' => (TokenKind::LBrace, TokenKind::RBrace, b'}'),
                    b'(' => (TokenKind::LParen, TokenKind::RParen, b')'),
                    _ => (TokenKind::LBracket, TokenKind::RBracket, b']'),
                };
                Some((ot, ct, cb, if collapse_kw != TokenKind::Whitespace { item_kw_start } else { next_item_start.unwrap_or(start as u32) }))
            } else {
                None
            };
            if let Some((open_tk, close_tk, close_b, span_start)) = try_delim {
                let is_container = matches!(collapse_kw, TokenKind::Impl | TokenKind::Trait | TokenKind::Mod);
                let max_pos = if span_start <= target_offset { (target_offset as usize).min(len) } else { len };
                if skip_delim_check || (span_start <= target_offset && start >= max_pos) {
                    skip_delim_check = false;
                    if is_container {
                        if brace_depth == 0 {
                            top_is_container = true;
                        }
                    } else {
                        (in_target_item, target_item_depth) = (true, brace_depth);
                    }
                    (collapse_kw, next_item_start, saw_eq) = (TokenKind::Whitespace, None, false);
                } else if let Some(close) = find_matching_delim(src, bytes, start, b0, close_b, max_pos) {
                    let mod_ok = collapse_kw != TokenKind::Mod || {
                        let inner = &src[start + 1..close];
                        !inner.trim_start().starts_with("#![") && (!src[span_start as usize..start].contains("macro_use") || !inner.contains("macro_rules"))
                    };
                    if mod_ok {
                        if close_b == b'}' && matches!(collapse_kw, TokenKind::Fn | TokenKind::Impl | TokenKind::Trait | TokenKind::Struct | TokenKind::Enum) {
                            tokens.truncate(kw_tok_idx);
                            tokens.push(Token::new(collapse_kw, 0, kw_start, start as u32));
                            tokens.push(Token::new(TokenKind::LBrace, 0, start as u32, close as u32));
                            tokens.push(Token::new(TokenKind::RBrace, 0, close as u32, (close + 1) as u32));
                        } else {
                            tokens.push(Token::new(open_tk, 0, start as u32, (start + 1) as u32));
                            if close > start + 1 {
                                tokens.push(Token::new(TokenKind::Whitespace, 0, (start + 1) as u32, close as u32));
                            }
                            tokens.push(Token::new(close_tk, 0, close as u32, (close + 1) as u32));
                        }
                        (prev_punct, prev_non_trivia) = (false, close_tk);
                        if close_b == b'}' {
                            (collapse_kw, next_item_start, saw_eq) = (TokenKind::Whitespace, None, false);
                        }
                        pos = close + 1;
                        continue;
                    }
                } else if span_start <= target_offset && max_pos < len {
                    if is_container {
                        if brace_depth == 0 && (collapse_kw != TokenKind::Mod || !src[span_start as usize..start].contains("macro_use")) {
                            top_is_container = true;
                        }
                    } else {
                        (in_target_item, target_item_depth) = (true, brace_depth);
                    }
                    (collapse_kw, next_item_start, saw_eq) = (TokenKind::Whitespace, None, false);
                }
            }
        }
        let (kind, flags, mut end) = scan_next_token(src, bytes, start);
        if kind.is_trivia() {
            while flags == 0 && end < len {
                match bytes[end] {
                    b' ' | b'\t' | b'\n' | b'\r' | 0x0B | 0x0C => end = scan_whitespace(src, bytes, end),
                    b'/' if end + 1 < len && bytes[end + 1] == b'/' => end = scan_line_comment(bytes, end),
                    _ => break,
                }
            }
            prev_punct = false;
            if let Some(last) = tokens.last_mut().filter(|t| t.is_trivia()) {
                last.end = end as u32;
            } else {
                tokens.push(Token::new(kind, flags, start as u32, end as u32));
            }
            pos = end;
            continue;
        }
        let cur_punct = kind.is_punct();
        if prev_punct && cur_punct {
            tokens.last_mut().unwrap().flags |= JOINT_NEXT;
        }
        prev_punct = cur_punct;
        match kind {
            TokenKind::LParen | TokenKind::LBracket => pb_depth += 1,
            TokenKind::RParen | TokenKind::RBracket => pb_depth = pb_depth.saturating_sub(1),
            TokenKind::Eq if pb_depth == 0 => saw_eq = true,
            TokenKind::LBrace => {
                if brace_depth == 0 && pb_depth == 0 && collapse_kw != TokenKind::Whitespace {
                    top_is_container = matches!(collapse_kw, TokenKind::Impl | TokenKind::Trait | TokenKind::Mod);
                }
                brace_depth += 1;
                if pb_depth == 0 {
                    (collapse_kw, next_item_start, saw_eq, skip_delim_check) = (TokenKind::Whitespace, None, false, false);
                }
            }
            TokenKind::RBrace => {
                brace_depth = brace_depth.saturating_sub(1);
                if brace_depth == 0 {
                    top_is_container = false;
                }
                if pb_depth == 0 {
                    if in_target_item && brace_depth <= target_item_depth && (end as u32) > target_offset {
                        in_target_item = false;
                    }
                    (collapse_kw, next_item_start, saw_eq, skip_delim_check) = (TokenKind::Whitespace, None, false, false);
                }
            }
            TokenKind::Semi if pb_depth == 0 => {
                if in_target_item && brace_depth <= target_item_depth && (end as u32) > target_offset {
                    in_target_item = false;
                }
                (collapse_kw, next_item_start, saw_eq, skip_delim_check) = (TokenKind::Whitespace, None, false, false);
            }
            TokenKind::Fn | TokenKind::Struct | TokenKind::Enum | TokenKind::Use | TokenKind::Type | TokenKind::Const | TokenKind::Static | TokenKind::Impl | TokenKind::Trait
                if pb_depth == 0
                    && !in_target_item
                    && (collapse_kw == TokenKind::Whitespace
                        || (collapse_kw == TokenKind::Const
                            && kind == TokenKind::Fn
                            && !saw_eq
                            && !matches!(
                                prev_non_trivia,
                                TokenKind::Colon | TokenKind::Eq | TokenKind::For | TokenKind::Lt | TokenKind::Gt | TokenKind::Comma | TokenKind::Amp | TokenKind::Star
                            ))) =>
            {
                let item_s = *next_item_start.get_or_insert(start as u32);
                let max_pos = if item_s <= target_offset { (target_offset as usize).min(len) } else { len };
                match skip_item_from_kw(src, bytes, end, kind, max_pos) {
                    SkipItem::Collapsed(open_p, close_p, term_b) => {
                        tokens.push(Token::new(kind, 0, start as u32, open_p as u32));
                        if term_b == b';' {
                            tokens.push(Token::new(TokenKind::Semi, 0, open_p as u32, (close_p + 1) as u32));
                            (prev_punct, prev_non_trivia) = (false, TokenKind::Semi);
                        } else {
                            tokens.push(Token::new(TokenKind::LBrace, 0, open_p as u32, close_p as u32));
                            tokens.push(Token::new(TokenKind::RBrace, 0, close_p as u32, (close_p + 1) as u32));
                            (prev_punct, prev_non_trivia) = (false, TokenKind::RBrace);
                        }
                        (collapse_kw, next_item_start, saw_eq) = (TokenKind::Whitespace, None, false);
                        pos = close_p + 1;
                        continue;
                    }
                    SkipItem::ContainsTarget => {
                        if matches!(kind, TokenKind::Impl | TokenKind::Trait) {
                            (collapse_kw, item_kw_start, kw_tok_idx, kw_start, skip_delim_check) = (kind, item_s, tokens.len(), start as u32, true);
                        } else {
                            (in_target_item, target_item_depth) = (true, brace_depth);
                            (collapse_kw, next_item_start, saw_eq) = (TokenKind::Whitespace, None, false);
                        }
                    }
                    SkipItem::Bail => {
                        (collapse_kw, item_kw_start, kw_tok_idx, kw_start) = (kind, item_s, tokens.len(), start as u32);
                    }
                }
            }
            TokenKind::Fn | TokenKind::Struct | TokenKind::Enum | TokenKind::Impl | TokenKind::Trait | TokenKind::Mod | TokenKind::Const | TokenKind::Static
                if pb_depth == 0 && !in_target_item =>
            {
                let item_s = *next_item_start.get_or_insert(start as u32);
                if collapse_kw == TokenKind::Whitespace || (collapse_kw == TokenKind::Const && !saw_eq && prev_non_trivia != TokenKind::Colon) {
                    (collapse_kw, item_kw_start, kw_tok_idx, kw_start) = (kind, item_s, tokens.len(), start as u32);
                }
            }
            TokenKind::Ident if pb_depth == 0 && !in_target_item && matches!(&src[start..end], "macro_rules" | "macro") => {
                (collapse_kw, item_kw_start) = (TokenKind::Bang, *next_item_start.get_or_insert(start as u32));
            }
            _ if pb_depth == 0 && collapse_kw == TokenKind::Whitespace => {
                next_item_start.get_or_insert(start as u32);
            }
            _ => {}
        }
        prev_non_trivia = kind;
        tokens.push(Token::new(kind, flags, start as u32, end as u32));
        pos = end;
    }
    tokens
}

#[inline(always)]
pub(crate) fn scan_next_token(src: &str, bytes: &[u8], start: usize) -> (TokenKind, u8, usize) {
    let b = bytes[start];

    match b {
        b' ' | b'\t' | b'\n' | b'\r' | 0x0B | 0x0C => (TokenKind::Whitespace, 0, scan_whitespace(src, bytes, start)),

        b'/' => {
            if start + 1 < bytes.len() {
                if bytes[start + 1] == b'/' {
                    return (TokenKind::LineComment, 0, scan_line_comment(bytes, start));
                }
                if bytes[start + 1] == b'*' {
                    let (flags, end) = scan_block_comment(bytes, start);
                    return (TokenKind::BlockComment, flags, end);
                }
            }
            (TokenKind::Slash, 0, start + 1)
        }

        b'(' => (TokenKind::LParen, 0, start + 1),
        b')' => (TokenKind::RParen, 0, start + 1),
        b'{' => (TokenKind::LBrace, 0, start + 1),
        b'}' => (TokenKind::RBrace, 0, start + 1),
        b'[' => (TokenKind::LBracket, 0, start + 1),
        b']' => (TokenKind::RBracket, 0, start + 1),

        b'!' => (TokenKind::Bang, 0, start + 1),
        b'#' => (TokenKind::Hash, 0, start + 1),
        b'$' => (TokenKind::Dollar, 0, start + 1),
        b'%' => (TokenKind::Percent, 0, start + 1),
        b'&' => (TokenKind::Amp, 0, start + 1),
        b'*' => (TokenKind::Star, 0, start + 1),
        b'+' => (TokenKind::Plus, 0, start + 1),
        b',' => (TokenKind::Comma, 0, start + 1),
        b'-' => (TokenKind::Minus, 0, start + 1),
        b'.' => (TokenKind::Dot, 0, start + 1),
        b':' => (TokenKind::Colon, 0, start + 1),
        b';' => (TokenKind::Semi, 0, start + 1),
        b'<' => (TokenKind::Lt, 0, start + 1),
        b'=' => (TokenKind::Eq, 0, start + 1),
        b'>' => (TokenKind::Gt, 0, start + 1),
        b'?' => (TokenKind::Question, 0, start + 1),
        b'@' => (TokenKind::At, 0, start + 1),
        b'^' => (TokenKind::Caret, 0, start + 1),
        b'|' => (TokenKind::Pipe, 0, start + 1),
        b'~' => (TokenKind::Tilde, 0, start + 1),

        b'"' => scan_quoted_str(src, bytes, start, TokenKind::Str),
        b'\'' => scan_char_or_lifetime(src, bytes, start),
        b'0'..=b'9' => scan_number(src, bytes, start),

        b'r' => scan_r_prefix_or_ident(src, bytes, start),
        b'b' => scan_b_prefix_or_ident(src, bytes, start),
        b'c' => scan_c_prefix_or_ident(src, bytes, start),

        b'a' | b'd'..=b'q' | b's'..=b'z' | b'A'..=b'Z' | b'_' => {
            let end = scan_ident_tail(src, bytes, start + 1);
            (classify_ident(&src[start..end]), 0, end)
        }

        0x00..=0x7F => (TokenKind::Unknown, HAS_ERROR, start + 1),

        _ => scan_non_ascii(src, bytes, start),
    }
}
