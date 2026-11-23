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

pub mod builder;
pub mod cargo;
pub mod direct;
pub mod driver;
pub mod feature_graph;
mod feature_seed;
mod last_good;
pub mod manifest;
pub mod no_build;
pub mod orchestrate;
pub mod required_features;
pub mod session;
pub mod session_report;
mod session_units;
pub mod shards;
pub mod source_snapshots;
pub mod types;
pub mod variant;

#[cfg(test)]
pub mod tests;

pub use builder::*;
pub use cargo::*;
pub use direct::*;
pub use driver::*;
pub use feature_graph::*;
pub use manifest::*;
pub use orchestrate::*;
pub use shards::*;
pub use types::*;
pub use variant::*;
