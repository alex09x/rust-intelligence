/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Fast length-bucketed classification of strict Rust keywords vs identifiers.

use crate::token::TokenKind;

/// Classifies a bare identifier slice into either a strict keyword / [`TokenKind::Underscore`]
/// or [`TokenKind::Ident`]. Contextual keywords (`default`, `union`, `gen`, `macro_rules`,
/// `auto`, `raw`, `safe`, `try`) remain [`TokenKind::Ident`].
#[inline]
pub(crate) fn classify_ident(s: &str) -> TokenKind {
    match s.len() {
        1 => {
            if s.as_bytes()[0] == b'_' {
                TokenKind::Underscore
            } else {
                TokenKind::Ident
            }
        }
        2 => match s.as_bytes() {
            b"as" => TokenKind::As,
            b"fn" => TokenKind::Fn,
            b"if" => TokenKind::If,
            b"in" => TokenKind::In,
            _ => TokenKind::Ident,
        },
        3 => match s.as_bytes() {
            b"dyn" => TokenKind::Dyn,
            b"for" => TokenKind::For,
            b"let" => TokenKind::Let,
            b"mod" => TokenKind::Mod,
            b"mut" => TokenKind::Mut,
            b"pub" => TokenKind::Pub,
            b"ref" => TokenKind::Ref,
            b"use" => TokenKind::Use,
            _ => TokenKind::Ident,
        },
        4 => match s.as_bytes() {
            b"else" => TokenKind::Else,
            b"enum" => TokenKind::Enum,
            b"impl" => TokenKind::Impl,
            b"loop" => TokenKind::Loop,
            b"move" => TokenKind::Move,
            b"self" => TokenKind::SelfKw,
            b"Self" => TokenKind::SelfType,
            b"true" => TokenKind::True,
            b"type" => TokenKind::Type,
            _ => TokenKind::Ident,
        },
        5 => match s.as_bytes() {
            b"async" => TokenKind::Async,
            b"await" => TokenKind::Await,
            b"break" => TokenKind::Break,
            b"const" => TokenKind::Const,
            b"crate" => TokenKind::Crate,
            b"false" => TokenKind::False,
            b"match" => TokenKind::Match,
            b"super" => TokenKind::Super,
            b"trait" => TokenKind::Trait,
            b"where" => TokenKind::Where,
            b"while" => TokenKind::While,
            _ => TokenKind::Ident,
        },
        6 => match s.as_bytes() {
            b"extern" => TokenKind::Extern,
            b"return" => TokenKind::Return,
            b"static" => TokenKind::Static,
            b"struct" => TokenKind::Struct,
            b"unsafe" => TokenKind::Unsafe,
            _ => TokenKind::Ident,
        },
        8 => {
            if s.as_bytes() == b"continue" {
                TokenKind::Continue
            } else {
                TokenKind::Ident
            }
        }
        _ => TokenKind::Ident,
    }
}
