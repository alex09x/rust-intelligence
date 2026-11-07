/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! An error-tolerant recursive-descent parser over the significant tokens. It never fails: what it cannot parse
//! goes into `Error` nodes, and every token ends up in the tree.

mod expr;
mod expr_atoms;
mod generics;
mod items;
mod items_fn;
mod patterns;
mod stmts;
mod types;

use crate::syntax_kind::SyntaxKind;
use crate::token::{Token, TokenKind};
use crate::tree::{Event, SyntaxTree};

/// Parses `src` into a lossless syntax tree.
pub fn parse(src: &str) -> SyntaxTree<'_> {
    let tokens = crate::lex(src);
    let mut sig: Vec<u32> = Vec::with_capacity(tokens.len() * 3 / 4 + 8);
    let mut i = 0usize;
    let tlen = tokens.len();
    while i < tlen {
        if !tokens[i].is_trivia() {
            sig.push(i as u32);
        }
        i += 1;
    }
    let events_cap = sig.len() * 3 + 16;
    let mut p = Parser {
        src,
        tokens: &tokens,
        sig: &sig,
        pos: 0,
        events: Vec::with_capacity(events_cap),
        nodes_count: 0,
        #[cfg(debug_assertions)]
        fuel: 0,
    };
    let m = p.start();
    items::item_list(&mut p, false);
    while !p.at_eof() {
        p.err_bump();
    }
    m.complete(&mut p, SyntaxKind::SourceFile);
    let Parser { events, nodes_count, .. } = p;
    SyntaxTree::build(src, tokens, &sig, events, nodes_count as usize)
}

pub(crate) struct Parser<'a> {
    src: &'a str,
    tokens: &'a [Token],
    sig: &'a [u32],
    pos: usize,
    events: Vec<Event>,
    nodes_count: u32,
    /// Lookahead calls since the last consumed token: a loop that stops consuming is a bug, caught in debug.
    #[cfg(debug_assertions)]
    fuel: u32,
}

/// An open node; must be completed or abandoned.
pub(crate) struct Marker {
    pos: u32,
}

/// A completed node, which can still get a new parent with [`CompletedMarker::precede`].
#[derive(Clone, Copy)]
pub(crate) struct CompletedMarker {
    pos: u32,
    kind: SyntaxKind,
}

impl Marker {
    #[inline]
    pub(crate) fn complete(self, p: &mut Parser<'_>, kind: SyntaxKind) -> CompletedMarker {
        p.events[self.pos as usize].kind = kind;
        p.nodes_count += 1;
        p.events.push(Event::FINISH);
        CompletedMarker { pos: self.pos, kind }
    }

    #[inline]
    pub(crate) fn abandon(self, p: &mut Parser<'_>) {
        if self.pos as usize == p.events.len() - 1 {
            p.events.pop();
        }
    }
}

impl CompletedMarker {
    /// Opens a node before this one that will contain it.
    #[inline]
    pub(crate) fn precede(self, p: &mut Parser<'_>) -> Marker {
        let m = p.start();
        p.events[self.pos as usize].set_fp(m.pos - self.pos);
        m
    }

    #[inline]
    pub(crate) fn kind(self) -> SyntaxKind {
        self.kind
    }
}

impl<'a> Parser<'a> {
    #[inline]
    pub(crate) fn start(&mut self) -> Marker {
        let pos = self.events.len() as u32;
        self.events.push(Event::start(SyntaxKind::Tombstone));
        Marker { pos }
    }

    #[inline]
    pub(crate) fn tok(&self, n: usize) -> Option<Token> {
        self.sig.get(self.pos + n).map(|&i| self.tokens[i as usize])
    }

    #[inline]
    pub(crate) fn nth(&mut self, n: usize) -> Option<TokenKind> {
        #[cfg(debug_assertions)]
        {
            self.fuel += 1;
            debug_assert!(self.fuel < 100_000, "parser stopped consuming tokens");
        }
        self.tok(n).map(|t| t.kind)
    }

    #[inline]
    pub(crate) fn current(&mut self) -> Option<TokenKind> {
        self.nth(0)
    }

    #[inline]
    pub(crate) fn at(&mut self, k: TokenKind) -> bool {
        self.nth(0) == Some(k)
    }

    #[inline]
    pub(crate) fn nth_at(&mut self, n: usize, k: TokenKind) -> bool {
        self.nth(n) == Some(k)
    }

    #[inline]
    pub(crate) fn at_eof(&mut self) -> bool {
        self.pos >= self.sig.len()
    }

    /// The text of the `n`th significant token ahead (contextual keywords: `union`, `default`, `macro_rules`).
    #[inline]
    pub(crate) fn nth_text(&self, n: usize) -> &'a str {
        self.tok(n).map_or("", |t| t.text(self.src))
    }

    #[inline]
    pub(crate) fn nth_text_eq(&self, n: usize, kw: &str) -> bool {
        let Some(t) = self.tok(n) else { return false };
        (t.end - t.start) as usize == kw.len() && &self.src.as_bytes()[t.start as usize..t.end as usize] == kw.as_bytes()
    }

    #[inline]
    pub(crate) fn at_contextual(&mut self, kw: &str) -> bool {
        self.at(TokenKind::Ident) && self.nth_text_eq(0, kw)
    }

    /// Whether the punctuation tokens ahead spell `op` with nothing between them (`::`, `->`, `..=`).
    #[inline]
    pub(crate) fn at_op(&mut self, op: &str) -> bool {
        self.nth_at_op(0, op)
    }

    /// Whether the tokens from the `n`th ahead spell `op` with nothing between them.
    #[inline]
    pub(crate) fn nth_at_op(&mut self, n: usize, op: &str) -> bool {
        let bytes = op.as_bytes();
        let len = bytes.len();
        if self.pos + n + len > self.sig.len() {
            return false;
        }
        for (i, &b) in bytes.iter().enumerate() {
            let t = self.tokens[self.sig[self.pos + n + i] as usize];
            if t.kind.punct_byte() != Some(b) || (i + 1 < len && !t.is_joint()) {
                return false;
            }
        }
        true
    }

    #[inline]
    pub(crate) fn bump(&mut self) {
        if self.pos < self.sig.len() {
            self.pos += 1;
            #[cfg(debug_assertions)]
            {
                self.fuel = 0;
            }
            self.events.push(Event::TOKEN);
        }
    }

    #[inline]
    pub(crate) fn bump_n(&mut self, n: usize) {
        for _ in 0..n {
            self.bump();
        }
    }

    #[inline]
    pub(crate) fn eat(&mut self, k: TokenKind) -> bool {
        let hit = self.at(k);
        if hit {
            self.bump();
        }
        hit
    }

    pub(crate) fn eat_op(&mut self, op: &str) -> bool {
        let hit = self.at_op(op);
        if hit {
            self.bump_n(op.len());
        }
        hit
    }

    /// Consumes `k`, or leaves an empty `Error` node where it was expected.
    pub(crate) fn expect(&mut self, k: TokenKind) -> bool {
        if self.eat(k) {
            return true;
        }
        let m = self.start();
        m.complete(self, SyntaxKind::Error);
        false
    }

    /// Wraps the next token in an `Error` node.
    pub(crate) fn err_bump(&mut self) {
        let m = self.start();
        self.bump();
        m.complete(self, SyntaxKind::Error);
    }

    /// The token index of the current position, to tell whether a loop made progress.
    pub(crate) fn mark_pos(&self) -> usize {
        self.pos
    }
}

/// Parses a balanced token tree starting at an opening delimiter.
pub(crate) fn token_tree(p: &mut Parser<'_>) {
    let m = p.start();
    let mut depth = 0u32;
    loop {
        match p.current() {
            None => break,
            Some(TokenKind::LParen | TokenKind::LBrace | TokenKind::LBracket) => depth += 1,
            Some(TokenKind::RParen | TokenKind::RBrace | TokenKind::RBracket) => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    p.bump();
                    break;
                }
            }
            _ if depth == 0 => break,
            _ => {}
        }
        p.bump();
    }
    m.complete(p, SyntaxKind::TokenTree);
}

/// Whether the current token can name something: an identifier or a path keyword.
pub(crate) fn at_name_ref(p: &mut Parser<'_>) -> bool {
    matches!(p.current(), Some(TokenKind::Ident | TokenKind::RawIdent | TokenKind::SelfKw | TokenKind::SelfType | TokenKind::Super | TokenKind::Crate))
}

/// A declared name: `fn name`, `struct Name`.
pub(crate) fn name(p: &mut Parser<'_>) {
    if matches!(p.current(), Some(TokenKind::Ident | TokenKind::RawIdent)) {
        let m = p.start();
        p.bump();
        m.complete(p, SyntaxKind::Name);
    } else {
        p.expect(TokenKind::Ident);
    }
}
