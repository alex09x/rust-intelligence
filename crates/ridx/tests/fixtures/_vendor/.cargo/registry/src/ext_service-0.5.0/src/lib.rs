/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

pub fn external_compute() -> u32 {
    999
}

pub fn external_closure_builder(offset: i32) -> impl Fn(i32) -> i32 {
    let base = offset + 1;
    move |x| x + base
}

