/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Token representation, kinds, flags, and binary-search offset lookup.

use core::ops::Range;

/// Flag set on a single-character punctuation [`Token`] when the immediately following
/// token is also a single-character punctuation token with zero intervening trivia
/// (`self.end == next.start`).
pub const JOINT_NEXT: u8 = 1 << 0;

/// Flag set on a [`Token`] when lexical recovery occurred (e.g. unclosed literal or
/// block comment, empty/overlong char literal, missing digits after base/exponent prefix,
/// invalid raw identifier, or unrecognized source character).
pub const HAS_ERROR: u8 = 1 << 1;

/// Compact 12-byte lossless lexical token referencing a byte span `[start, end)` in `src`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Token {
    pub kind: TokenKind,
    pub flags: u8,
    pub start: u32,
    pub end: u32,
}

const _: () = assert!(core::mem::size_of::<Token>() == 12);

impl Token {
    #[inline]
    pub const fn new(kind: TokenKind, flags: u8, start: u32, end: u32) -> Self {
        Self { kind, flags, start, end }
    }

    #[inline]
    pub const fn len(self) -> u32 {
        self.end - self.start
    }

    #[inline]
    pub const fn is_empty(self) -> bool {
        self.start == self.end
    }

    #[inline]
    pub const fn is_trivia(self) -> bool {
        self.kind.is_trivia()
    }

    #[inline]
    pub const fn is_joint(self) -> bool {
        (self.flags & JOINT_NEXT) != 0
    }

    #[inline]
    pub const fn has_error(self) -> bool {
        (self.flags & HAS_ERROR) != 0
    }

    #[inline]
    pub const fn range(self) -> Range<usize> {
        (self.start as usize)..(self.end as usize)
    }

    #[inline]
    pub fn text(self, src: &str) -> &str {
        &src[self.range()]
    }
}

/// Finds the index of the token covering `byte` (`token.start <= byte < token.end`)
/// using `O(log N)` binary search over a contiguous or monotonic slice of tokens.
#[inline]
pub fn token_at(tokens: &[Token], byte: u32) -> Option<usize> {
    let idx = tokens.partition_point(|t| t.end <= byte);
    if idx < tokens.len() && tokens[idx].start <= byte {
        Some(idx)
    } else {
        None
    }
}

/// Lexical kind of a Rust source token (edition-independent, context-free).
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TokenKind {
    // Trivia
    Whitespace,
    LineComment,
    BlockComment,
    Shebang,

    // Identifiers & lifetimes
    Ident,
    RawIdent,
    Lifetime,

    // Literals
    Int,
    Float,
    Char,
    Byte,
    Str,
    RawStr,
    ByteStr,
    RawByteStr,
    CStr,
    RawCStr,

    // Strict keywords (Rust 2018 / 2021) + underscore
    As,
    Async,
    Await,
    Break,
    Const,
    Continue,
    Crate,
    Dyn,
    Else,
    Enum,
    Extern,
    False,
    Fn,
    For,
    If,
    Impl,
    In,
    Let,
    Loop,
    Match,
    Mod,
    Move,
    Mut,
    Pub,
    Ref,
    Return,
    SelfKw,
    SelfType,
    Static,
    Struct,
    Super,
    Trait,
    True,
    Type,
    Unsafe,
    Use,
    Where,
    While,
    Underscore,

    // Delimiters
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,

    // Atomic single-character punctuation
    Bang,
    Hash,
    Dollar,
    Percent,
    Amp,
    Star,
    Plus,
    Comma,
    Minus,
    Dot,
    Slash,
    Colon,
    Semi,
    Lt,
    Eq,
    Gt,
    Question,
    At,
    Caret,
    Pipe,
    Tilde,

    // Unrecognized character or stray byte sequence
    Unknown,
}

impl TokenKind {
    /// Returns `true` for whitespace, comments (including doc comments), and file shebangs.
    #[inline]
    pub const fn is_trivia(self) -> bool {
        (self as u8) <= (Self::Shebang as u8)
    }

    /// Returns `true` for non-delimiter atomic punctuation characters eligible for [`JOINT_NEXT`].
    #[inline]
    pub const fn is_punct(self) -> bool {
        let v = self as u8;
        v >= (Self::Bang as u8) && v <= (Self::Tilde as u8)
    }

    /// Returns `true` for `(`, `)`, `{`, `}`, `[`, `]`.
    #[inline]
    pub const fn is_delimiter(self) -> bool {
        let v = self as u8;
        v >= (Self::LParen as u8) && v <= (Self::RBracket as u8)
    }

    /// Returns `true` for strict keywords (excluding [`Self::Underscore`]).
    #[inline]
    pub const fn is_keyword(self) -> bool {
        let v = self as u8;
        v >= (Self::As as u8) && v <= (Self::While as u8)
    }

    /// Returns `true` for numeric, character, byte, string, and C-string literals.
    #[inline]
    pub const fn is_literal(self) -> bool {
        let v = self as u8;
        v >= (Self::Int as u8) && v <= (Self::RawCStr as u8)
    }

    /// Maps an atomic punctuation [`TokenKind`] back to its ASCII byte, enabling fast
    /// multi-character operator checks (`at_op("::")`, `eat_op("->")`, `at_op(">>")`) in the parser.
    #[inline]
    pub const fn punct_byte(self) -> Option<u8> {
        const BYTES: [u8; 21] = *b"!#$%&*+,-./:;<=>?@^|~";
        let idx = (self as u8).wrapping_sub(Self::Bang as u8) as usize;
        if idx < BYTES.len() {
            Some(BYTES[idx])
        } else {
            None
        }
    }
}
