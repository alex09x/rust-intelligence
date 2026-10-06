/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Cargo indexing orchestration, variant hashing, driver invocation, and manifest publishing.

pub mod cargo;
pub mod driver;
pub mod manifest;
pub mod orchestrate;
pub mod types;
pub mod variant;

#[cfg(test)]
pub mod tests;

pub use cargo::*;
pub use driver::*;
pub use manifest::*;
pub use orchestrate::*;
pub use types::*;
pub use variant::*;
