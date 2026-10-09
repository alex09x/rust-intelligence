/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Core library in multi-crate fixture workspace.

pub struct CoreService {
    pub id: u32,
}

impl CoreService {
    pub fn new(id: u32) -> Self {
        Self { id }
    }

    pub fn execute(&self) -> u32 {
        self.id * 10
    }
}

pub trait Worker {
    fn work(&self) -> u32;
}
