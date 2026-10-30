/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Reusable parameter validation helpers for Language Server Protocol requests.

use super::error::{LspError, LspResult};
use serde_json::Value;

/// Extracts and validates `(uri, line_0based, col_0based)` from a TextDocumentPositionParams payload.
pub fn text_document_position(msg: &Value) -> LspResult<(&str, u32, u32)> {
    let params = msg.get("params").ok_or_else(|| LspError::invalid_params("Missing params object"))?;
    if !params.is_object() {
        return Err(LspError::invalid_params("Params must be an object"));
    }

    let uri = params["textDocument"]["uri"].as_str().ok_or_else(|| LspError::invalid_params("Missing textDocument.uri"))?;
    if uri.is_empty() {
        return Err(LspError::invalid_params("Empty textDocument.uri"));
    }

    let pos = &params["position"];
    if !pos.is_object() {
        return Err(LspError::invalid_params("Missing position object"));
    }
    let line = pos["line"].as_u64().ok_or_else(|| LspError::invalid_params("Missing position.line"))? as u32;
    let col = pos["character"].as_u64().ok_or_else(|| LspError::invalid_params("Missing position.character"))? as u32;

    Ok((uri, line, col))
}

/// Extracts `(uri, Option<(line_0based, col_0based)>)` from a TextDocumentPositionParams payload,
/// requiring valid textDocument.uri and params object, but allowing missing position.
pub fn text_document_opt_position(msg: &Value) -> LspResult<(&str, Option<(u32, u32)>)> {
    let params = msg.get("params").ok_or_else(|| LspError::invalid_params("Missing params object"))?;
    if !params.is_object() {
        return Err(LspError::invalid_params("Params must be an object"));
    }

    let uri = params["textDocument"]["uri"].as_str().ok_or_else(|| LspError::invalid_params("Missing textDocument.uri"))?;
    if uri.is_empty() {
        return Err(LspError::invalid_params("Empty textDocument.uri"));
    }

    let pos = &params["position"];
    if !pos.is_object() {
        return Ok((uri, None));
    }
    let line = match pos["line"].as_u64() {
        Some(l) => l as u32,
        None => return Ok((uri, None)),
    };
    let col = match pos["character"].as_u64() {
        Some(c) => c as u32,
        None => return Ok((uri, None)),
    };

    Ok((uri, Some((line, col))))
}

/// Extracts and validates `(uri, start_line, start_col, end_line, end_col)` from a Range-based payload.
pub fn text_document_range(msg: &Value) -> LspResult<(&str, u32, u32, u32, u32)> {
    let params = msg.get("params").ok_or_else(|| LspError::invalid_params("Missing params object"))?;
    if !params.is_object() {
        return Err(LspError::invalid_params("Params must be an object"));
    }

    let uri = params["textDocument"]["uri"].as_str().ok_or_else(|| LspError::invalid_params("Missing textDocument.uri"))?;
    if uri.is_empty() {
        return Err(LspError::invalid_params("Empty textDocument.uri"));
    }

    let range = &params["range"];
    if !range.is_object() {
        return Err(LspError::invalid_params("Missing range object"));
    }
    let start_line = range["start"]["line"].as_u64().ok_or_else(|| LspError::invalid_params("Missing range.start.line"))? as u32;
    let start_col = range["start"]["character"].as_u64().unwrap_or(0) as u32;
    let end_line = range["end"]["line"].as_u64().ok_or_else(|| LspError::invalid_params("Missing range.end.line"))? as u32;
    let end_col = range["end"]["character"].as_u64().unwrap_or(0) as u32;

    Ok((uri, start_line, start_col, end_line, end_col))
}

/// Extracts and validates `uri` from a TextDocumentIdentifier payload.
pub fn text_document_uri(msg: &Value) -> LspResult<&str> {
    let params = msg.get("params").ok_or_else(|| LspError::invalid_params("Missing params object"))?;
    if !params.is_object() {
        return Err(LspError::invalid_params("Params must be an object"));
    }
    let uri = params["textDocument"]["uri"].as_str().ok_or_else(|| LspError::invalid_params("Missing textDocument.uri"))?;
    if uri.is_empty() {
        return Err(LspError::invalid_params("Empty textDocument.uri"));
    }
    Ok(uri)
}
