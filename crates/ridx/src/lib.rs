/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

pub mod cli;
pub mod compare;
pub mod indexer;
pub mod lsp;
pub mod overlay;

#[path = "../../ridx-syntax/src/char_class.rs"]
mod char_class;
#[path = "../../ridx-syntax/src/keyword.rs"]
mod keyword;
#[path = "../../ridx-syntax/src/lexer.rs"]
mod lexer;
#[path = "../../ridx-syntax/src/name_res/mod.rs"]
pub mod name_res;
#[path = "../../ridx-syntax/src/parser/mod.rs"]
pub mod parser;
#[path = "../../ridx-syntax/src/scan_char.rs"]
mod scan_char;
#[path = "../../ridx-syntax/src/scan_lit.rs"]
mod scan_lit;
#[path = "../../ridx-syntax/src/scan_trivia.rs"]
mod scan_trivia;
#[path = "../../ridx-syntax/src/sema_fn.rs"]
pub mod sema_fn;
#[path = "../../ridx-syntax/src/sema_macro.rs"]
mod sema_macro;
#[path = "../../ridx-syntax/src/sema_pat.rs"]
mod sema_pat;
#[path = "../../ridx-syntax/src/sema_scope.rs"]
mod sema_scope;
#[path = "../../ridx-syntax/src/syntax_kind.rs"]
pub mod syntax_kind;
#[path = "../../ridx-syntax/src/token.rs"]
pub mod token;
#[path = "../../ridx-syntax/src/tree.rs"]
pub mod tree;
#[path = "../../ridx-syntax/src/tree_dump.rs"]
mod tree_dump;
#[path = "../../ridx-syntax/src/ty_infer/mod.rs"]
pub mod ty_infer;

pub use lexer::lex;
pub use parser::parse;
pub use syntax_kind::SyntaxKind;
pub use token::{token_at, Token, TokenKind, HAS_ERROR, JOINT_NEXT};
pub use tree::{NodeId, SyntaxTree};
