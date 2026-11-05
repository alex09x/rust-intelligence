/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Hover assembly helpers: associated return types, doc link resolution, `let` bindings and their layouts.

use super::{associated_return, imported_path, join_url, let_binding_hover, let_binding_hover_with_layout};

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

#[test]
fn test_let_binding_hover_names_the_binding_with_a_short_type() {
    let line = "    let mut v: Vec<String> = Vec::new();";
    assert_eq!(let_binding_hover(line, 13, "std::vec::Vec<std::string::String>").as_deref(), Some("```rust\nlet mut v: Vec<String>\n```"));
    assert_eq!(let_binding_hover("    let c = Circle::new(2.0);", 9, "shapes::Circle").as_deref(), Some("```rust\nlet c: Circle\n```"));
    assert_eq!(let_binding_hover("    outlet(c);", 12, "u8"), None, "no let binding");
}

#[test]
fn test_let_binding_layout_is_found_at_the_pattern_start() {
    let line = "    let mut v: Vec<String> = Vec::new();";
    let layout_at = |col: u32| (col == 9).then(|| "size = 24 (0x18), align = 0x8, needs Drop".to_string());
    assert_eq!(
        let_binding_hover_with_layout(line, 13, "Vec<String>", &layout_at).as_deref(),
        Some("```rust\nlet mut v: Vec<String>\n```\n\n---\n\nsize = 24 (0x18), align = 0x8, needs Drop"),
        "the pattern of `let mut v` starts at `mut`"
    );
}
