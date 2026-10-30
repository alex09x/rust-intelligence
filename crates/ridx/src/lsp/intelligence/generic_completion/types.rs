/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Data types for generic parameter bounds resolution in completion.

/// A generic parameter and its resolved trait bounds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenericParam {
    /// Parameter identifier (e.g. `F`, `B`, `P`, `T`).
    pub name: String,
    /// Trait bounds on this parameter (e.g. `["SystemParamFunction"]`, `["System"]`).
    pub bounds: Vec<String>,
}

impl GenericParam {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), bounds: Vec::new() }
    }

    pub fn with_bounds(name: impl Into<String>, bounds: Vec<String>) -> Self {
        Self { name: name.into(), bounds }
    }
}
