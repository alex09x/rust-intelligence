/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration test suite for LSP streaming server.

pub mod common;
pub mod foreign_shard_tests;
mod fresh_protocol_tests;
mod index_refresh_tests;
pub mod lifecycle_tests;
pub mod reload_tests;
pub mod stage8_dispatch_tests;
pub mod stages_early_tests;
pub mod stages_late_tests;
pub mod symbol_tests;
pub mod sync_tests;
