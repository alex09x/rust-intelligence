/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Which imports count as workspace symbols: re-exports, decided from the `use` statement's source.

use super::use_is_pub;

#[test]
fn test_use_is_pub_reads_the_statement_visibility() {
    let src = "use std::fmt::Display;\npub use crate::a::B;\npub(crate) use self::c::D;\nfn f() {}\nuse x::{\n    y::{Z},\n    W,\n};\npub use q::{\n    R,\n};\n";
    assert!(!use_is_pub(src, 1, 15), "private import");
    assert!(use_is_pub(src, 2, 19), "pub re-export");
    assert!(use_is_pub(src, 3, 25), "pub(crate) re-export");
    assert!(!use_is_pub(src, 6, 9), "nested group of a private import");
    assert!(!use_is_pub(src, 7, 5), "after a nested group, still private");
    assert!(use_is_pub(src, 10, 5), "multi-line pub group");
}
