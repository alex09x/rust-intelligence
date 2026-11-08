/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Zero-dependency, lossless Rust lexer and CST foundation (`ridx-syntax`).
//!
//! Guarantees for [`lex`]:
//! - Lossless round-trip: concatenating `&src[t.start as usize..t.end as usize]` across
//!   all returned tokens reproduces `src` byte-for-byte on arbitrary UTF-8 input.
//! - Panic-free and infallible: malformed or truncated overlay code (unclosed strings,
//!   raw strings, block comments, char literals, invalid escapes, stray bytes) produces
//!   tokens marked with [`HAS_ERROR`] rather than failing.
//! - Context-free atomic punctuation: multi-character operators are represented as
//!   single-character punctuation tokens with [`JOINT_NEXT`] set when adjacent with no
//!   intervening trivia.

mod char_class;
mod keyword;
mod lexer;
pub mod name_res;
pub mod parser;
mod scan_char;
mod scan_lit;
mod scan_trivia;
pub mod sema_fn;
mod sema_macro;
mod sema_pat;
mod sema_scope;
pub mod syntax_kind;
pub mod token;
pub mod tree;
mod tree_dump;
pub mod ty_infer;

pub use lexer::lex;
pub use parser::parse;
pub use syntax_kind::SyntaxKind;
pub use token::{token_at, Token, TokenKind, HAS_ERROR, JOINT_NEXT};
pub use tree::{NodeId, SyntaxTree};
