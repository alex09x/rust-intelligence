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

pub mod inner {
    pub struct Thing {
        pub value: u32,
    }

    impl Thing {
        pub fn new(value: u32) -> Self {
            Self { value }
        }

        pub fn get_value(&self) -> u32 {
            self.value
        }
    }
}

pub use inner::Thing;

pub mod other {
    pub struct Thing {
        pub count: usize,
    }

    impl Thing {
        pub fn other_fn(&self) -> usize {
            self.count
        }
    }
}

pub fn use_other_thing() -> usize {
    let t = other::Thing { count: 5 };
    t.other_fn()
}
