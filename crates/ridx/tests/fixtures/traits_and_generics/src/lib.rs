/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! L2 fixture for traits, associated types, and generic method dispatch.

pub trait Container {
    type Item;
    fn extract(&self) -> Self::Item;
}

pub struct BoxContainer(pub i32);

impl Container for BoxContainer {
    type Item = i32;
    fn extract(&self) -> Self::Item {
        self.0
    }
}

pub fn process_container<C: Container<Item = i32>>(c: &C) -> i32 {
    c.extract()
}

pub trait Greeter {
    fn greet(&self) -> &'static str;
    fn default_greet(&self) -> &'static str {
        "hello default"
    }
}

pub struct Robot;

impl Greeter for Robot {
    fn greet(&self) -> &'static str {
        "beep boop"
    }
}

pub fn call_concrete(r: &Robot) -> &'static str {
    r.greet()
}

pub fn call_generic<G: Greeter>(g: &G) -> &'static str {
    let _ = G::default_greet;
    g.greet()
}

pub fn call_default(r: &Robot) -> &'static str {
    r.default_greet()
}

pub fn process_bytes(b: [u8; 4]) -> i32 {
    let _closing = "}";
    // }
    let c = BoxContainer(b[0] as i32);
    c.extract()
}

pub struct Wrapper<W>(pub W);

impl<W: Greeter> Wrapper<W> {
    pub fn wrap_greet(&self) -> &'static str {
        let _ = W::default_greet;
        self.0.greet()
    }
}

pub fn labels_of(items: &[u32]) -> Vec<String> {
    let mut labels_out = Vec::new();
    for item in items {
        let mut loop_local: Vec<String> = Vec::new();
        loop_local.push(item.to_string());
        labels_out.extend(loop_local);
    }
    labels_out
}

pub fn failure_text(n: usize) -> String {
    format!("failed to download ({}) // not a comment", n.min(9))
}

pub async fn async_count() -> usize {
    let mut async_local: Vec<u8> = Vec::new();
    async_local.push(1);
    async_local.len()
}

