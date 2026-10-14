/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! LSP editing handlers: rename, formatting, code actions, and commands.

pub mod actions;
pub mod apply_registry;
pub mod atomic_write;
pub mod commands;
pub mod extract;
pub mod format;
pub mod ident;
pub mod linked;
pub mod organize;
pub mod prepare_rename;
pub mod rename;

#[cfg(test)]
mod tests;

pub use actions::*;
pub use atomic_write::commit_closed_writes_atomic;
pub use commands::*;
pub use extract::*;
pub use format::*;
pub use ident::*;
pub use linked::*;
pub use organize::*;
pub use rename::*;
