/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Data types and path cleaning helpers for Self:: completion.

/// Lexical scope context for `Self::` resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnclosingScope {
    Impl {
        trait_name: Option<String>,
        type_name: String,
    },
    Trait {
        trait_name: String,
        supertraits: Vec<String>,
    },
    Other,
}

/// Strips generic parameters `<...>` from a type or trait path, handling nested brackets.
pub fn strip_generics(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut depth: usize = 0;
    for c in raw.chars() {
        if c == '<' {
            depth += 1;
        } else if c == '>' {
            depth = depth.saturating_sub(1);
        } else if depth == 0 {
            out.push(c);
        }
    }
    out.trim().to_string()
}

/// Normalizes and cleans a trait name or path.
pub fn clean_trait_name(raw: &str) -> String {
    strip_generics(raw.trim())
}

/// Normalizes and cleans a type name or path from an `impl` header.
pub fn clean_type_name(raw: &str) -> String {
    let mut s = raw.trim();
    if let Some(rest) = s.strip_prefix('&') {
        s = rest.trim_start();
        if s.starts_with('\'') {
            if let Some(sp) = s.find(|c: char| c.is_whitespace() || c == ':') {
                s = s[sp..].trim_start();
            }
        }
        if let Some(rest_mut) = s.strip_prefix("mut") {
            if rest_mut.starts_with(' ') || rest_mut.starts_with('\t') {
                s = rest_mut.trim_start();
            }
        }
    }
    strip_generics(s)
}

/// Finds the index of a whole keyword with word boundaries outside angle brackets.
pub fn find_keyword(s: &str, kw: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let kw_bytes = kw.as_bytes();
    let mut depth: usize = 0;
    let mut i = 0;

    while i + kw_bytes.len() <= bytes.len() {
        let b = bytes[i];
        if b == b'<' {
            depth += 1;
            i += 1;
            continue;
        } else if b == b'>' {
            depth = depth.saturating_sub(1);
            i += 1;
            continue;
        }

        if depth == 0 && &bytes[i..i + kw_bytes.len()] == kw_bytes {
            let prev_ok = i == 0 || (!bytes[i - 1].is_ascii_alphanumeric() && bytes[i - 1] != b'_');
            let next_pos = i + kw_bytes.len();
            let next_ok = next_pos == bytes.len()
                || (!bytes[next_pos].is_ascii_alphanumeric() && bytes[next_pos] != b'_');
            if prev_ok && next_ok {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}
