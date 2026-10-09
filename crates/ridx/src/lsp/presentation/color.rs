/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! LSP document color and color presentation handlers.

use serde_json::{json, Value};

/// Handles `textDocument/documentColor` requests.
pub fn handle_document_color(_msg: &Value) -> Value {
    json!([])
}

/// Handles `textDocument/colorPresentation` requests.
pub fn handle_color_presentation(_msg: &Value) -> Value {
    json!([])
}
