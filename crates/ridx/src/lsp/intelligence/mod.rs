/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! LSP intelligence handlers: hover, completion, signature help, and inlay hints.

pub mod completion;
pub mod hints;
pub mod hover;
pub mod signature;

#[cfg(test)]
pub mod tests;

pub use completion::*;
pub use hints::*;
pub use hover::*;
pub use signature::*;
