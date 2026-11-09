/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Stack-inlined 24-byte identifier key (`IdentKey`) avoiding per-entry heap allocations in
//! [`super::def_map::ModuleScope`] and [`super::ext_scope::ExtModuleScope`] hash tables.

use std::borrow::Borrow;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::Deref;

const INLINE_CAP: usize = 62;

/// 24-byte immutable UTF-8 identifier storing identifiers up to 22 bytes inline without heap allocation.
#[derive(Clone)]
pub enum IdentKey {
    Inline { len: u8, buf: [u8; INLINE_CAP] },
    Heap(Box<str>),
}

impl IdentKey {
    #[inline]
    pub fn new(s: &str) -> Self {
        let bytes = s.as_bytes();
        if bytes.len() <= INLINE_CAP {
            let mut buf = [0u8; INLINE_CAP];
            buf[..bytes.len()].copy_from_slice(bytes);
            Self::Inline { len: bytes.len() as u8, buf }
        } else {
            Self::Heap(s.into())
        }
    }

    #[inline]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Inline { len, buf } => std::str::from_utf8(&buf[..*len as usize]).unwrap_or(""),
            Self::Heap(b) => b,
        }
    }
}

impl From<&str> for IdentKey {
    #[inline]
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

impl From<String> for IdentKey {
    #[inline]
    fn from(s: String) -> Self {
        if s.len() <= INLINE_CAP {
            Self::new(&s)
        } else {
            Self::Heap(s.into_boxed_str())
        }
    }
}

impl Deref for IdentKey {
    type Target = str;

    #[inline]
    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<str> for IdentKey {
    #[inline]
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Borrow<str> for IdentKey {
    #[inline]
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl PartialEq for IdentKey {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl Eq for IdentKey {}

impl PartialEq<str> for IdentKey {
    #[inline]
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<&str> for IdentKey {
    #[inline]
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl Hash for IdentKey {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_str().hash(state);
    }
}

impl fmt::Debug for IdentKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), f)
    }
}

impl fmt::Display for IdentKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self.as_str(), f)
    }
}
