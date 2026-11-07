/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Zero-allocation recursive-descent evaluator for Rust `#[cfg(...)]` predicates (`all`, `any`,
//! `not`, `true`, `false`, bare flags, `key = "value"` / raw strings) over [`Token`] slices.

use std::borrow::Cow;

use super::db::{CrateId, ItemDb};
use crate::sema_fn::norm_ident;
use crate::token::{Token, TokenKind as T};

/// Evaluates a single `cfg(...)` predicate token slice (the tokens inside `(` ... `)` of `cfg(...)`).
///
/// Never panics on malformed or truncated user input:
/// - Empty or incomplete `cfg()` while typing returns `true` so partial edits do not drop the module.
/// - Trailing tokens after a valid predicate evaluate strictly (`false` if unexpected garbage follows).
#[inline]
pub fn eval_cfg_tokens(src: &str, tokens: &[Token], db: &dyn ItemDb, krate: CrateId) -> bool {
    let mut cur = SigCursor::new(src, tokens);
    if cur.peek().is_none() {
        return true;
    }
    let val = eval_pred(&mut cur, db, krate, 0);
    // Allow optional trailing comma at top-level `#[cfg(unix,)]`.
    cur.eat(T::Comma);
    val && cur.peek().is_none()
}

const MAX_CFG_DEPTH: u32 = 32;

fn eval_pred(cur: &mut SigCursor<'_>, db: &dyn ItemDb, krate: CrateId, depth: u32) -> bool {
    if depth >= MAX_CFG_DEPTH {
        return false;
    }
    let Some(first) = cur.next() else {
        return true;
    };
    match first.kind {
        T::True => true,
        T::False => false,
        T::Ident | T::RawIdent => {
            let name = norm_ident(first.text(cur.src));
            match cur.peek_kind() {
                Some(T::LParen) => {
                    cur.next(); // consume `(`
                    match name {
                        "all" => eval_list(cur, db, krate, depth + 1, true),
                        "any" => eval_list(cur, db, krate, depth + 1, false),
                        "not" => {
                            if cur.eat(T::RParen) {
                                return false;
                            }
                            let inner = eval_pred(cur, db, krate, depth + 1);
                            cur.eat(T::Comma);
                            skip_to_rparen(cur);
                            !inner
                        }
                        _ => {
                            skip_to_rparen(cur);
                            false
                        }
                    }
                }
                Some(T::Eq) => {
                    cur.next(); // consume `=`
                    let Some(val_tok) = cur.next() else {
                        return false;
                    };
                    let Some(val) = decode_str_literal(cur.src, val_tok) else {
                        return false;
                    };
                    db.cfg_key_value(krate, name, &val)
                }
                _ => db.cfg_flag(krate, name),
            }
        }
        kind if kind.is_keyword() => {
            let name = first.text(cur.src);
            if cur.eat(T::Eq) {
                let Some(val_tok) = cur.next() else {
                    return false;
                };
                let Some(val) = decode_str_literal(cur.src, val_tok) else {
                    return false;
                };
                db.cfg_key_value(krate, name, &val)
            } else {
                db.cfg_flag(krate, name)
            }
        }
        _ => false,
    }
}

fn eval_list(cur: &mut SigCursor<'_>, db: &dyn ItemDb, krate: CrateId, depth: u32, is_all: bool) -> bool {
    let mut acc = is_all;
    loop {
        if cur.peek().is_none() {
            return acc;
        }
        if cur.eat(T::RParen) {
            return acc;
        }
        let item = eval_pred(cur, db, krate, depth);
        if is_all {
            acc &= item;
        } else {
            acc |= item;
        }
        if !cur.eat(T::Comma) {
            skip_to_rparen(cur);
            return acc;
        }
    }
}

fn skip_to_rparen(cur: &mut SigCursor<'_>) {
    let mut depth = 1u32;
    while let Some(t) = cur.next() {
        match t.kind {
            T::LParen => depth += 1,
            T::RParen => {
                depth -= 1;
                if depth == 0 {
                    return;
                }
            }
            _ => {}
        }
    }
}

/// Decodes a Rust `"..."` or `r#"..."#` string literal token without allocating whenever no
/// backslash escape is present (covering >99.9% of `cfg` and `#[path]` string literals).
pub fn decode_str_literal<'s>(src: &'s str, tok: Token) -> Option<Cow<'s, str>> {
    let raw = tok.text(src);
    match tok.kind {
        T::Str => {
            let inner = raw.strip_prefix('"')?;
            let inner = inner.strip_suffix('"').unwrap_or(inner);
            if !inner.contains('\\') {
                return Some(Cow::Borrowed(inner));
            }
            Some(Cow::Owned(unescape_str(inner)))
        }
        T::RawStr => {
            let rest = raw.strip_prefix('r')?;
            let hashes = rest.bytes().take_while(|&b| b == b'#').count();
            let after_open = rest.get(hashes..)?.strip_prefix('"')?;
            let close_len = 1 + hashes;
            let body =
                if after_open.len() >= close_len && after_open.as_bytes()[after_open.len() - close_len] == b'"' { &after_open[..after_open.len() - close_len] } else { after_open };
            Some(Cow::Borrowed(body))
        }
        _ => None,
    }
}

fn unescape_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('0') => out.push('\0'),
            Some('\\') => out.push('\\'),
            Some('"') => out.push('"'),
            Some('\'') => out.push('\''),
            Some('\n') => {
                while matches!(chars.peek(), Some(' ' | '\t' | '\n' | '\r')) {
                    chars.next();
                }
            }
            Some('x') => {
                let hi = chars.next().and_then(|c| c.to_digit(16));
                let lo = chars.next().and_then(|c| c.to_digit(16));
                if let (Some(h), Some(l)) = (hi, lo) {
                    out.push(((h << 4) | l) as u8 as char);
                }
            }
            Some('u') if chars.peek() == Some(&'{') => {
                chars.next();
                let mut val = 0u32;
                for c in chars.by_ref() {
                    if c == '}' {
                        break;
                    }
                    if let Some(d) = c.to_digit(16) {
                        val = (val << 4) | d;
                    }
                }
                if let Some(decoded) = char::from_u32(val) {
                    out.push(decoded);
                }
            }
            Some(other) => out.push(other),
            None => break,
        }
    }
    out
}

/// Trivia-skipping cursor over a slice of [`Token`]s.
pub(crate) struct SigCursor<'s> {
    pub(crate) src: &'s str,
    tokens: &'s [Token],
    pos: usize,
}

impl<'s> SigCursor<'s> {
    #[inline]
    pub(crate) fn new(src: &'s str, tokens: &'s [Token]) -> Self {
        let mut s = Self { src, tokens, pos: 0 };
        s.skip_trivia();
        s
    }

    #[inline]
    fn skip_trivia(&mut self) {
        while self.pos < self.tokens.len() && self.tokens[self.pos].is_trivia() {
            self.pos += 1;
        }
    }

    #[inline]
    pub(crate) fn peek(&self) -> Option<Token> {
        self.tokens.get(self.pos).copied()
    }

    #[inline]
    pub(crate) fn peek_kind(&self) -> Option<T> {
        self.peek().map(|t| t.kind)
    }

    #[inline]
    pub(crate) fn next(&mut self) -> Option<Token> {
        let tok = self.tokens.get(self.pos).copied()?;
        self.pos += 1;
        self.skip_trivia();
        Some(tok)
    }

    #[inline]
    pub(crate) fn eat(&mut self, kind: T) -> bool {
        if self.peek_kind() == Some(kind) {
            self.next();
            true
        } else {
            false
        }
    }
}
