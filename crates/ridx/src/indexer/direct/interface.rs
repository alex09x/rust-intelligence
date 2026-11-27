/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! What a crate shows the crates depending on it, read from its text: every token but the comments (doc comments are
//! attributes, and stay) and the bodies of ordinary functions, without positions. Two texts with the same interface give a dependent the same index, but for
//! the positions of the definitions it references, and the tokens kept in both, in the same order, are where those
//! definitions start (a definition is recorded at its name, or at the start of an item without one).
//!
//! A body stays in the interface where a dependent can see into it: a `const fn` (evaluated in a dependent's types),
//! an `async fn` or one returning `impl Trait` (the hidden type and its auto traits leak), a body holding an `impl` or
//! a call of a macro other than the standard library's (which may expand to one), and every item under an attribute
//! that may be a procedural macro, which may make anything of a body. A macro call is kept whole, its input being anything. An item under `#[cfg(test)]` is no part of the library
//! its dependents compile against, and is left out.

use ridx_syntax::{lex, Token, TokenKind, JOINT_NEXT};

/// The interface of one file: the hash of its tokens' kinds and texts, and the line and column (1-based, the column
/// in characters, as the driver records a definition) of each, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileInterface {
    pub hash: u64,
    pub positions: Vec<(u32, u32)>,
}

/// Attributes that leave a function's body to the compiler: built in, or a tool's.
const PLAIN_ATTRIBUTES: &[&str] = &[
    "allow",
    "automatically_derived",
    "clippy",
    "cold",
    "coverage",
    "deny",
    "deprecated",
    "derive",
    "doc",
    "expect",
    "export_name",
    "forbid",
    "ignore",
    "inline",
    "link_section",
    "must_use",
    "no_mangle",
    "non_exhaustive",
    "optimize",
    "repr",
    "rustfmt",
    "should_panic",
    "target_feature",
    "test",
    "track_caller",
    "warn",
];

/// The interface of the Rust source `src`, or `None` when its delimiters do not balance (a text being typed): no part
/// of it can then be told from the rest.
pub(crate) fn file_interface(src: &str) -> Option<FileInterface> {
    // A doc comment is an attribute (`#[doc = ".."]`) a macro or a derive is given: kept, but in a body left out
    // (review of 2a030462).
    let tokens: Vec<Token> = lex(src).into_iter().filter(|t| !t.is_trivia() || is_doc_comment(&src[t.start as usize..t.end as usize])).collect();
    let close = matching_delimiters(&tokens)?;
    let mut scan = Scan { src, t: &tokens, close: &close, kept: Vec::with_capacity(tokens.len()) };
    scan.walk(0, tokens.len());
    Some(scan.finish())
}

/// A token of the interface: one of the text, or the stand-in of a function body left out (at its `{`).
#[derive(Clone, Copy)]
enum Kept {
    Token(usize),
    Body(usize),
}

struct Scan<'a> {
    src: &'a str,
    t: &'a [Token],
    close: &'a [usize],
    kept: Vec<Kept>,
}

impl Scan<'_> {
    fn text(&self, i: usize) -> &str {
        &self.src[self.t[i].start as usize..self.t[i].end as usize]
    }

    fn kind(&self, i: usize) -> Option<TokenKind> {
        self.t.get(i).map(|t| t.kind)
    }

    fn keep(&mut self, from: usize, to: usize) {
        self.kept.extend((from..to).map(Kept::Token));
    }

    fn walk(&mut self, mut i: usize, end: usize) {
        while i < end {
            match self.kind(i) {
                Some(TokenKind::Hash) if self.attribute_end(i).is_some() => i = self.attributed(i, end),
                Some(TokenKind::Ident | TokenKind::RawIdent) if self.kind(i + 1) == Some(TokenKind::Bang) => {
                    // `name!(..)`, `macro_rules! name { .. }`: whole.
                    let group = if matches!(self.kind(i + 2), Some(TokenKind::Ident | TokenKind::RawIdent)) { i + 3 } else { i + 2 };
                    let to = if is_open(self.kind(group)) { self.close[group] + 1 } else { i + 2 };
                    self.keep(i, to.min(end));
                    i = to;
                }
                Some(TokenKind::Fn) if matches!(self.kind(i + 1), Some(TokenKind::Ident | TokenKind::RawIdent)) => i = self.function(i, end),
                _ => {
                    self.kept.push(Kept::Token(i));
                    i += 1;
                }
            }
        }
    }

    /// The tokens after the attribute starting at `i` (`#[..]` or `#![..]`).
    fn attribute_end(&self, i: usize) -> Option<usize> {
        let open = if self.kind(i + 1) == Some(TokenKind::Bang) { i + 2 } else { i + 1 };
        (self.kind(open) == Some(TokenKind::LBracket)).then(|| self.close[open] + 1)
    }

    /// Whether the attribute in the `[` at `open` may be a procedural macro: not built in, nor a tool's.
    fn is_macro_like(&self, open: usize) -> bool {
        let inner: Vec<&str> = (open + 1..self.close[open]).take(3).map(|k| self.text(k)).collect();
        // `#[unsafe(no_mangle)]` names the attribute inside.
        let name = if inner.first() == Some(&"unsafe") { inner.get(2) } else { inner.first() };
        !name.is_some_and(|n| PLAIN_ATTRIBUTES.contains(n) || *n == "cfg")
    }

    /// The outer attributes from `i` and the item they are on.
    fn attributed(&mut self, i: usize, end: usize) -> usize {
        let first = self.attribute_end(i).unwrap_or(i + 1);
        if self.kind(i + 1) == Some(TokenKind::Bang) {
            self.keep(i, first);
            return first;
        }
        let (mut j, mut cfg_test, mut macro_like) = (i, false, false);
        while self.kind(j) == Some(TokenKind::Hash) && self.kind(j + 1) == Some(TokenKind::LBracket) {
            let close = self.close[j + 1];
            cfg_test |= (j + 2..close).map(|k| self.text(k)).eq(["cfg", "(", "test", ")"]);
            macro_like |= self.is_macro_like(j + 1);
            j = close + 1;
        }
        if cfg_test {
            // Left out: an end found early leaves the rest of the item in, never the item after it out.
            return self.item_end(j, end, false);
        }
        if macro_like {
            // Kept whole: an end found late keeps more, never less, of what the macro is given (review of 2a030462:
            // `impl<T, U>` ended at its first `,`, and the bodies inside were left out).
            let item_end = self.item_end(j, end, true);
            self.keep(i, item_end);
            return item_end;
        }
        self.keep(i, j);
        j
    }

    /// Where the item, field or variant starting at `k` ends: its block or its `;`, `,` or the group holding it closing.
    /// Without `generics` it may end early (a `,` of generic arguments); with them, a `,` inside `<..>` is no end, and
    /// it may end late (a `<` that is a comparison). Never past the group holding it.
    fn item_end(&self, mut k: usize, end: usize, generics: bool) -> usize {
        let mut angle = 0i32;
        while k < end {
            match self.kind(k) {
                Some(TokenKind::LParen | TokenKind::LBracket) => k = self.close[k] + 1,
                Some(TokenKind::LBrace) => return self.close[k] + 1,
                Some(TokenKind::Semi) => return k + 1,
                Some(TokenKind::Comma) if !generics || angle <= 0 => return k + 1,
                Some(TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace) => return k,
                Some(TokenKind::Lt) => {
                    angle += 1;
                    k += 1;
                }
                // Not the `>` of `->` or `=>`.
                Some(TokenKind::Gt) if !(k > 0 && matches!(self.kind(k - 1), Some(TokenKind::Minus | TokenKind::Eq)) && self.t[k - 1].flags & JOINT_NEXT != 0) => {
                    angle -= 1;
                    k += 1;
                }
                _ => k += 1,
            }
        }
        end
    }

    /// A function item at `i` (`fn name`): its signature, and its body unless it is an ordinary one.
    fn function(&mut self, i: usize, end: usize) -> usize {
        let mut sees_body = false;
        let mut b = i;
        while b > 0 {
            b -= 1;
            match self.kind(b) {
                Some(TokenKind::Const | TokenKind::Async) => sees_body = true,
                Some(TokenKind::Unsafe | TokenKind::Extern | TokenKind::Str | TokenKind::RawStr) => {}
                Some(TokenKind::Ident) if matches!(self.text(b), "default" | "safe") => {}
                _ => break,
            }
        }
        let (mut j, mut angle, mut returns) = (i + 2, 0i32, false);
        let body = loop {
            match self.kind(j) {
                None => break None,
                _ if j >= end => break None,
                Some(TokenKind::Minus) if self.kind(j + 1) == Some(TokenKind::Gt) && self.t[j].flags & JOINT_NEXT != 0 => {
                    returns = true;
                    j += 2;
                    continue;
                }
                Some(TokenKind::LParen | TokenKind::LBracket) => {
                    sees_body |= returns && (j..self.close[j]).any(|k| self.kind(k) == Some(TokenKind::Impl));
                    j = self.close[j] + 1;
                    continue;
                }
                Some(TokenKind::LBrace) if angle > 0 => {
                    j = self.close[j] + 1;
                    continue;
                }
                Some(TokenKind::LBrace) => break Some(j),
                Some(TokenKind::Semi) if angle <= 0 => break None,
                Some(TokenKind::Lt) => angle += 1,
                Some(TokenKind::Gt) => angle -= 1,
                Some(TokenKind::Impl) if returns => sees_body = true,
                _ => {}
            }
            j += 1;
        };
        let Some(open) = body else {
            let to = j.min(end);
            self.keep(i, to);
            return to;
        };
        let close = self.close[open];
        // A macro called in the body may expand to an item seen outside it (`add!(u8)` to `impl T for u8 {}`, review
        // of 2a030462), but for the standard library's, which never do; so may an attribute macro on an item or a
        // statement in it (`#[make_impl(u8)] fn local() {}`, review of 6b27649c).
        sees_body |= (open + 1..close).any(|k| match self.kind(k) {
            Some(TokenKind::Impl) => true,
            Some(TokenKind::Ident) => {
                self.kind(k + 1) == Some(TokenKind::Bang) && (is_open(self.kind(k + 2)) || self.kind(k + 2) == Some(TokenKind::Ident)) && !STD_MACROS.contains(&self.text(k))
            }
            Some(TokenKind::Hash) => {
                let open = if self.kind(k + 1) == Some(TokenKind::Bang) { k + 2 } else { k + 1 };
                self.kind(open) == Some(TokenKind::LBracket) && self.is_macro_like(open)
            }
            _ => false,
        });
        self.keep(i, open);
        if sees_body {
            self.walk(open, close + 1);
        } else {
            self.kept.push(Kept::Body(open));
        }
        close + 1
    }

    fn finish(self) -> FileInterface {
        let mut hash = Fnv::new();
        let mut positions = Vec::with_capacity(self.kept.len());
        let mut lines = LineCols::new(self.src);
        // `line!()` and `column!()` kept give values that move with the text: its positions are its interface then
        // (`pub const N: usize = line!() as usize;`, review of 6b27649c).
        let located = self.kept.iter().any(
            |k| matches!(*k, Kept::Token(i) if self.kind(i) == Some(TokenKind::Ident) && matches!(self.text(i), "line" | "column") && self.kind(i + 1) == Some(TokenKind::Bang)),
        );
        for k in &self.kept {
            let i = match *k {
                Kept::Token(i) => {
                    // Whether a punctuation is joint to the next (`>>`, not `> >`): a procedural macro sees it.
                    hash.write(&[self.t[i].kind as u8, self.t[i].flags & JOINT_NEXT]);
                    hash.write(self.text(i).as_bytes());
                    hash.write(&[0]);
                    i
                }
                Kept::Body(i) => {
                    hash.write(&[0xff, 0]);
                    i
                }
            };
            let at = lines.at(self.t[i].start as usize);
            if located {
                hash.write(&at.0.to_le_bytes());
                hash.write(&at.1.to_le_bytes());
            }
            positions.push(at);
        }
        FileInterface { hash: hash.finish(), positions }
    }
}

/// The standard library's macros callable in a body, none of which expands to an item.
const STD_MACROS: &[&str] = &[
    "assert",
    "assert_eq",
    "assert_matches",
    "assert_ne",
    "cfg",
    "column",
    "compile_error",
    "concat",
    "dbg",
    "debug_assert",
    "debug_assert_eq",
    "debug_assert_ne",
    "env",
    "eprint",
    "eprintln",
    "file",
    "format",
    "format_args",
    "include_bytes",
    "include_str",
    "line",
    "matches",
    "module_path",
    "option_env",
    "panic",
    "print",
    "println",
    "stringify",
    "todo",
    "unimplemented",
    "unreachable",
    "vec",
    "write",
    "writeln",
];

/// `///`, `//!`, `/** */` and `/*! */`, not `////` or `/***/`.
fn is_doc_comment(text: &str) -> bool {
    (text.starts_with("///") && !text.starts_with("////"))
        || text.starts_with("//!")
        || (text.starts_with("/**") && !text.starts_with("/***") && text != "/**/")
        || text.starts_with("/*!")
}

fn is_open(kind: Option<TokenKind>) -> bool {
    matches!(kind, Some(TokenKind::LParen | TokenKind::LBrace | TokenKind::LBracket))
}

/// For each opening delimiter, the index of the one closing it (`usize::MAX` elsewhere); `None` if they do not balance.
fn matching_delimiters(t: &[Token]) -> Option<Vec<usize>> {
    let mut close = vec![usize::MAX; t.len()];
    let mut open: Vec<usize> = Vec::new();
    for (i, tok) in t.iter().enumerate() {
        let opener = match tok.kind {
            TokenKind::LParen | TokenKind::LBrace | TokenKind::LBracket => {
                open.push(i);
                continue;
            }
            TokenKind::RParen => TokenKind::LParen,
            TokenKind::RBrace => TokenKind::LBrace,
            TokenKind::RBracket => TokenKind::LBracket,
            _ => continue,
        };
        let o = open.pop().filter(|&o| t[o].kind == opener)?;
        close[o] = i;
    }
    open.is_empty().then_some(close)
}

/// Line and column of byte offsets visited in increasing order.
struct LineCols<'a> {
    src: &'a str,
    at: usize,
    line: u32,
    col: u32,
}

impl<'a> LineCols<'a> {
    fn new(src: &'a str) -> Self {
        Self { src, at: 0, line: 1, col: 1 }
    }

    fn at(&mut self, offset: usize) -> (u32, u32) {
        for c in self.src[self.at..offset].chars() {
            if c == '\n' {
                self.line += 1;
                self.col = 1;
            } else {
                self.col += 1;
            }
        }
        self.at = offset;
        (self.line, self.col)
    }
}

/// FNV-1a, 64 bits, as `ridx_format::fnv1a_64`, fed in parts.
pub(crate) struct Fnv(u64);

impl Fnv {
    pub(crate) fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    pub(crate) fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= u64::from(b);
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }

    pub(crate) fn finish(&self) -> u64 {
        self.0
    }
}

#[cfg(test)]
#[path = "interface_tests.rs"]
mod tests;
