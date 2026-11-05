/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Doc comments rendered as rustdoc and rust-analyzer show them.

use super::*;

fn resolver(t: &str) -> Option<String> {
    match link_path(t)?.as_str() {
        "Circle" => Some("https://docs.rs/x/1.0.0/x/struct.Circle.html".into()),
        "Shape::area" => Some("https://docs.rs/x/1.0.0/x/trait.Shape.html#tymethod.area".into()),
        "crate::shapes" => Some("https://docs.rs/x/1.0.0/x/shapes/index.html".into()),
        "core::option::Option::Some" => Some("https://doc.rust-lang.org/stable/core/option/enum.Option.html#variant.Some".into()),
        _ => None,
    }
}

#[test]
fn test_rust_blocks_are_tagged_and_hidden_lines_dropped() {
    assert!(render("```compile_fail,E0277\nlet x: u8 = \"a\";\n```", &resolver).starts_with("```rust\n"));
    let doc = "Adds.\n\n```edition2021\n# use x::add;\n#\nassert_eq!(add(1, 2), 3);\n## not hidden\n# fn main() {}\n```\n\n```text\n# kept in text\n```\n\n```\n    # indented hidden\nlet a = 1;\n```";
    let out = render(doc, &resolver);
    assert_eq!(out, "Adds.\n\n```rust\nassert_eq!(add(1, 2), 3);\n# not hidden\n```\n\n```text\n# kept in text\n```\n\n```rust\nlet a = 1;\n```");
}

#[test]
fn test_intra_doc_links_point_at_docs() {
    let doc = "See [`Circle`] and [`Shape::area`], or the [shapes module][crate::shapes].\n[`unknown`] stays, `[Circle]` in code stays.";
    let out = render(doc, &resolver);
    assert_eq!(
        out,
        "See [`Circle`](https://docs.rs/x/1.0.0/x/struct.Circle.html) and [`Shape::area`](https://docs.rs/x/1.0.0/x/trait.Shape.html#tymethod.area), or the [shapes module](https://docs.rs/x/1.0.0/x/shapes/index.html).\n[`unknown`] stays, `[Circle]` in code stays."
    );
}

#[test]
fn test_reference_definitions_resolve_and_their_uses_stay() {
    let doc = "Serialize a [`Some(T)`] value, see [here].\n\n[`Some(T)`]: core::option::Option::Some\n[here]: https://serde.rs/";
    let out = render(doc, &resolver);
    assert_eq!(
        out,
        "Serialize a [`Some(T)`] value, see [here].\n\n[`Some(T)`]: https://doc.rust-lang.org/stable/core/option/enum.Option.html#variant.Some\n[here]: https://serde.rs/"
    );
}

#[test]
fn test_definitions_move_to_the_end_and_full_references_inline() {
    let doc = "First [a circle][`C`] here.\n\n[`C`]: Circle\n\nMore text with [`C`].\n\n```\n[not]: a definition in code\n```";
    let out = render(doc, &resolver);
    assert_eq!(
        out,
        "First [a circle](https://docs.rs/x/1.0.0/x/struct.Circle.html) here.\n\nMore text with [`C`].\n\n```rust\n[not]: a definition in code\n```\n\n[`C`]: https://docs.rs/x/1.0.0/x/struct.Circle.html"
    );
}

#[test]
fn test_underscore_emphasis_becomes_stars_outside_code() {
    assert_eq!(render("exceeds `isize::MAX` _bytes_, __very__ snake_case_name `_x_`", &resolver), "exceeds `isize::MAX` *bytes*, **very** snake_case_name `_x_`");
}

#[test]
fn test_inline_path_links_resolve() {
    assert_eq!(render("Use [the circle](Circle) or [web](https://x.y).", &resolver), "Use [the circle](https://docs.rs/x/1.0.0/x/struct.Circle.html) or [web](https://x.y).");
}

#[test]
fn test_link_path_strips_decorations() {
    assert_eq!(link_path("`Vec::new()`").as_deref(), Some("Vec::new"));
    assert_eq!(link_path("struct@Circle").as_deref(), Some("Circle"));
    assert_eq!(link_path("vec!").as_deref(), Some("vec"));
    assert_eq!(link_path("Vec<T>").as_deref(), Some("Vec"));
    assert_eq!(link_path("two words"), None);
}
