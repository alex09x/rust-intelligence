/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! LSP call hierarchy and type hierarchy navigation handlers.

pub mod cache;
pub mod call;
pub mod def_cache;
pub mod fallback_cache;
pub mod impl_table;
pub mod scan;
pub mod type_index;
pub mod types;
pub mod value_use;

#[cfg(test)]
pub mod tests;

pub use call::*;
pub use def_cache::*;
pub use fallback_cache::*;
pub use impl_table::*;
pub use type_index::*;
pub use types::*;
