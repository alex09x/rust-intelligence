/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Test harness, DSL, and fixture helpers for Rust Intelligence (ridx).

pub mod cursor_dsl;
pub mod edit;
pub mod fixture;
pub mod framing;

pub use cursor_dsl::{
    parse_cursor, parse_fixture, parse_spans, AnnotatedSpan, CursorPos, ParsedFixture,
};
pub use edit::{
    apply_text_edits, apply_workspace_edit, apply_workspace_edit_with_versions,
    position_to_byte_offset, utf16_col_to_char_slice,
};
pub use fixture::{FixtureIndexBuilder, SyntheticWorkspaceFixture};
pub use framing::{make_lsp_msg, parse_lsp_responses, parse_lsp_responses_strict, TestLspSession};
