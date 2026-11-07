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
        matches!(self, Self::Whitespace | Self::LineComment | Self::BlockComment | Self::Shebang)
    }

    /// Returns `true` for non-delimiter atomic punctuation characters eligible for [`JOINT_NEXT`].
    #[inline]
    pub const fn is_punct(self) -> bool {
        self.punct_byte().is_some()
    }

    /// Returns `true` for `(`, `)`, `{`, `}`, `[`, `]`.
    #[inline]
    pub const fn is_delimiter(self) -> bool {
        matches!(self, Self::LParen | Self::RParen | Self::LBrace | Self::RBrace | Self::LBracket | Self::RBracket)
    }

    /// Returns `true` for strict keywords (excluding [`Self::Underscore`]).
    #[inline]
    pub const fn is_keyword(self) -> bool {
        matches!(
            self,
            Self::As
                | Self::Async
                | Self::Await
                | Self::Break
                | Self::Const
                | Self::Continue
                | Self::Crate
                | Self::Dyn
                | Self::Else
                | Self::Enum
                | Self::Extern
                | Self::False
                | Self::Fn
                | Self::For
                | Self::If
                | Self::Impl
                | Self::In
                | Self::Let
                | Self::Loop
                | Self::Match
                | Self::Mod
                | Self::Move
                | Self::Mut
                | Self::Pub
                | Self::Ref
                | Self::Return
                | Self::SelfKw
                | Self::SelfType
                | Self::Static
                | Self::Struct
                | Self::Super
                | Self::Trait
                | Self::True
                | Self::Type
                | Self::Unsafe
                | Self::Use
                | Self::Where
                | Self::While
        )
    }

    /// Returns `true` for numeric, character, byte, string, and C-string literals.
    #[inline]
    pub const fn is_literal(self) -> bool {
        matches!(self, Self::Int | Self::Float | Self::Char | Self::Byte | Self::Str | Self::RawStr | Self::ByteStr | Self::RawByteStr | Self::CStr | Self::RawCStr)
    }

    /// Maps an atomic punctuation [`TokenKind`] back to its ASCII byte, enabling fast
    /// multi-character operator checks (`at_op("::")`, `eat_op("->")`, `at_op(">>")`) in the parser.
    #[inline]
    pub const fn punct_byte(self) -> Option<u8> {
        match self {
            Self::Bang => Some(b'!'),
            Self::Hash => Some(b'#'),
            Self::Dollar => Some(b'$'),
            Self::Percent => Some(b'%'),
            Self::Amp => Some(b'&'),
            Self::Star => Some(b'*'),
            Self::Plus => Some(b'+'),
            Self::Comma => Some(b','),
            Self::Minus => Some(b'-'),
            Self::Dot => Some(b'.'),
            Self::Slash => Some(b'/'),
            Self::Colon => Some(b':'),
            Self::Semi => Some(b';'),
            Self::Lt => Some(b'<'),
            Self::Eq => Some(b'='),
            Self::Gt => Some(b'>'),
            Self::Question => Some(b'?'),
            Self::At => Some(b'@'),
            Self::Caret => Some(b'^'),
            Self::Pipe => Some(b'|'),
            Self::Tilde => Some(b'~'),
            _ => None,
        }
    }
}
