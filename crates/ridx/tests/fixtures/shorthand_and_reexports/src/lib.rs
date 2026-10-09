/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! L2 fixture for struct field shorthands, re-exports, and raw identifiers.

pub mod inner {
    pub struct InnerWidget {
        pub name: &'static str,
    }

    pub fn construct_widget(name: &'static str) -> InnerWidget {
        InnerWidget { name }
    }
}

pub use inner::construct_widget;
pub use inner::InnerWidget as AliasedWidget;

pub struct Point {
    pub x: i32,
    pub y: i32,
}

pub fn make_point(x: i32, y: i32) -> Point {
    Point { x, y }
}

pub fn unpack_point(p: Point) -> i32 {
    let Point { x, y } = p;
    x + y
}

pub fn r#match(r#type: i32) -> i32 {
    r#type * 2
}

pub fn invoke_match() -> i32 {
    r#match(42)
}

pub fn invoke_aliased_widget() -> AliasedWidget {
    construct_widget("example")
}

pub fn invoke_inferred_vec() {
    /* 🦀 */ let p = make_point(1, 2);
    let v = vec![1u8];
    let s = String::from("hello");
    let mut it = v.iter().map(|x| x + 1);
    let _ = (p, &v, s, it.next());
}

pub fn invoke_println(x: i32) {
    println!("{x}");
}

pub mod range {
    pub struct RangeVisitor;
    pub const FIELDS: &[&str] = &["start", "end"];
}

pub fn invoke_local_range() {
    let _ = range::RangeVisitor;
}

pub mod remote_ops {
    use core::ops;

    pub fn invoke_sysroot_ops() {
        let _ = ops::Range { start: 0, end: 1 };
    }
}

pub fn invoke_prelude_types() {
    let _: Vec<i32> = Vec::new();
    let _ = String::from("hello");
}
