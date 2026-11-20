/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Timing-sensitive and environment-mutating flycheck integration tests.

pub static PATH_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

mod ntm_flycheck_all_roots;
mod ntm_flycheck_changed_file;
mod ntm_flycheck_nonblocking;
mod ntm_flycheck_saturation;
