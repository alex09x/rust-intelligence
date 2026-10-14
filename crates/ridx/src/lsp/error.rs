/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Standard JSON-RPC and Language Server Protocol error types and codes.

use serde_json::{json, Value};
use std::fmt;

// JSON-RPC 2.0 error codes
pub const PARSE_ERROR: i32 = -32700;
pub const INVALID_REQUEST: i32 = -32600;
pub const METHOD_NOT_FOUND: i32 = -32601;
pub const INVALID_PARAMS: i32 = -32602;
pub const INTERNAL_ERROR: i32 = -32603;

// Language Server Protocol error codes (LSP 3.17)
pub const SERVER_NOT_INITIALIZED: i32 = -32002;
pub const UNKNOWN_ERROR_CODE: i32 = -32001;
pub const REQUEST_FAILED: i32 = -32803;
pub const SERVER_CANCELLED: i32 = -32802;
pub const CONTENT_MODIFIED: i32 = -32801;
pub const REQUEST_CANCELLED: i32 = -32800;

/// Structured LSP ResponseError payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LspError {
    pub code: i32,
    pub message: String,
    pub data: Option<Value>,
}

pub type LspResult<T = Value> = Result<T, LspError>;

impl LspError {
    pub fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    pub fn with_data(code: i32, message: impl Into<String>, data: Value) -> Self {
        Self {
            code,
            message: message.into(),
            data: Some(data),
        }
    }

    pub fn request_failed(message: impl Into<String>) -> Self {
        Self::new(REQUEST_FAILED, message)
    }

    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self::new(INVALID_PARAMS, message)
    }

    pub fn method_not_found(message: impl Into<String>) -> Self {
        Self::new(METHOD_NOT_FOUND, message)
    }

    pub fn request_cancelled() -> Self {
        Self::new(REQUEST_CANCELLED, "Request cancelled")
    }

    pub fn content_modified(message: impl Into<String>) -> Self {
        Self::new(CONTENT_MODIFIED, message)
    }

    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self::new(INVALID_REQUEST, message)
    }

    pub fn server_not_initialized() -> Self {
        Self::new(SERVER_NOT_INITIALIZED, "Server not initialized")
    }

    pub fn internal_error(message: impl Into<String>) -> Self {
        Self::new(INTERNAL_ERROR, message)
    }

    pub fn to_json(&self) -> Value {
        if let Some(ref data) = self.data {
            json!({
                "code": self.code,
                "message": self.message,
                "data": data,
            })
        } else {
            json!({
                "code": self.code,
                "message": self.message,
            })
        }
    }
}

impl fmt::Display for LspError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "LSP error {}: {}", self.code, self.message)
    }
}

impl std::error::Error for LspError {}
