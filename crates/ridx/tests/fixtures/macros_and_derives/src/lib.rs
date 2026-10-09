/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! L2 fixture for macro expansion, derives, and file inclusion.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedModel {
    pub id: u64,
    pub title: String,
}

macro_rules! make_multiplier {
    ($name:ident, $factor:expr) => {
        pub fn $name(val: i32) -> i32 {
            val * $factor
        }
    };
}

make_multiplier!(triple, 3);
make_multiplier!(quadruple, 4);

include!("included.rs");

pub fn execute_macro_code() -> i32 {
    let m = DerivedModel {
        id: 1,
        title: "Test".to_string(),
    };
    let _ = format!("Model: {:?}", m);
    triple(10) + quadruple(5) + included_helper()
}
