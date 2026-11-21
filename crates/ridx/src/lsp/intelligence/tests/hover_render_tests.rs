/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Hover assembly helpers: associated return types, doc link resolution and doc links.

use super::{associated_return, imported_path, join_url};

#[test]
fn test_associated_return_type_is_named_at_the_call() {
    assert_eq!(associated_return("fn extract(&self) -> Self::Item", Some("i32")).as_deref(), Some("`Self::Item` = `i32`"));
    assert_eq!(associated_return("fn len(&self) -> usize", Some("usize")), None);
    assert_eq!(associated_return("fn extract(&self) -> Self::Item", Some("fn(&Container) -> i32 {Container::extract}")), None);
}

#[test]
fn test_imports_and_relative_links_resolve() {
    let src = "use core::ops::Index;\nuse crate::raw::{RawVec, Cap};\nfn f() {}\n";
    assert_eq!(imported_path(src, "Index").as_deref(), Some("core::ops::Index"));
    assert_eq!(imported_path(src, "RawVec::new").as_deref(), Some("crate::raw::RawVec::new"));
    assert_eq!(imported_path(src, "Missing"), None);
    let base = "https://doc.rust-lang.org/stable/core/option/enum.Option.html#method.map";
    assert_eq!(join_url(base, "../../std/string/struct.String.html").as_deref(), Some("https://doc.rust-lang.org/stable/std/string/struct.String.html"));
}
