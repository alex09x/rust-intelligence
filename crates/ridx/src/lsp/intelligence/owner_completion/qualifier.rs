/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Path qualifier extraction and turbofish stripping for scoped completion.

/// Strips turbofish generic arguments `::<...>` from a path or code line.
///
/// Balanced angle brackets `<...>` immediately preceded by `::` are removed,
/// accounting for nested generics (e.g. `Foo::<Vec<u8>, B<C>>::`), string literals
/// (e.g. `Foo::<"hello", T>::`), char literals (e.g. `Foo::<'c', T>::`), and
/// lifetime arguments (e.g. `Foo::<'a, T>::`).
///
/// Comparison operators like `a < b::` or UFCS paths like `<T as Tr>::` are left untouched.
pub fn strip_turbofish(s: &str) -> String {
    if !s.contains("::<") {
        return s.to_string();
    }

    let mut res = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let n = bytes.len();
    let mut i = 0;
    let mut last_copied = 0;

    while i < n {
        if i + 2 < n && bytes[i] == b':' && bytes[i + 1] == b':' && bytes[i + 2] == b'<' {
            let mut j = i + 3;
            let mut depth = 1usize;

            while j < n && depth > 0 {
                let b = bytes[j];
                if b == b'"' {
                    j += 1;
                    while j < n && bytes[j] != b'"' {
                        if bytes[j] == b'\\' && j + 1 < n {
                            j += 2;
                        } else {
                            j += 1;
                        }
                    }
                    if j < n {
                        j += 1;
                    }
                } else if b == b'\'' {
                    if j + 1 < n && bytes[j + 1] == b'\\' {
                        if let Some(pos) = s[j + 2..].find('\'') {
                            if pos <= 10 {
                                j = j + 2 + pos + 1;
                                continue;
                            }
                        }
                        j += 1;
                    } else if j + 2 < n && bytes[j + 2] == b'\'' {
                        j += 3;
                    } else if j + 3 < n && &bytes[j + 1..j + 3] == b"u{" {
                        if let Some(pos) = s[j + 3..].find('\'') {
                            if pos <= 10 {
                                j = j + 3 + pos + 1;
                                continue;
                            }
                        }
                        j += 1;
                    } else {
                        j += 1;
                        while j < n && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
                            j += 1;
                        }
                    }
                } else if (b == b'-' || b == b'=') && j + 1 < n && bytes[j + 1] == b'>' {
                    j += 2;
                } else if b == b'<' {
                    depth += 1;
                    j += 1;
                } else if b == b'>' {
                    depth -= 1;
                    j += 1;
                } else {
                    j += 1;
                }
            }

            if depth == 0 {
                res.push_str(&s[last_copied..i]);
                i = j;
                last_copied = j;
                continue;
            }
        }
        i += 1;
    }

    res.push_str(&s[last_copied..]);
    res
}

use super::ufcs::extract_ufcs_qualifier;

/// Extracts the path qualifier preceding `::`, accounting for turbofish generic arguments,
/// UFCS expressions (`<T as Tr>::`, `<&Type>::`), and nested `use` groups like `use alloc::{string::`.
pub fn extract_colon_qualifier(trimmed_before: &str) -> String {
    let stripped;
    let s = if trimmed_before.contains("::<") {
        stripped = strip_turbofish(trimmed_before);
        &stripped
    } else {
        trimmed_before
    };

    let colon_idx = match s.rfind("::") {
        Some(idx) => idx,
        None => return String::new(),
    };
    let before_colon = s[..colon_idx].trim_end();

    if before_colon.ends_with('>') {
        if let Some(ufcs_qual) = extract_ufcs_qualifier(before_colon) {
            return ufcs_qual;
        }
    }

    if let Some(brace_idx) = before_colon.rfind('{') {
        let before_brace = before_colon[..brace_idx].trim_end();
        if let Some(prefix_base) = before_brace.strip_suffix("::") {
            let after_brace = &before_colon[brace_idx + 1..];
            if !after_brace.contains('}') {
                let last_item = after_brace.rsplit(',').next().unwrap_or(after_brace).trim();
                let sub_qual: String = last_item
                    .chars()
                    .rev()
                    .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == ':')
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();
                let base_qual: String = prefix_base
                    .chars()
                    .rev()
                    .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == ':')
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();
                let base_trimmed = base_qual.trim_end_matches(':');
                if !base_trimmed.is_empty() && !sub_qual.is_empty() {
                    return format!("{base_trimmed}::{sub_qual}");
                } else if !base_trimmed.is_empty() {
                    return base_trimmed.to_string();
                } else if !sub_qual.is_empty() {
                    return sub_qual;
                }
            }
        }
    }

    before_colon
        .chars()
        .rev()
        .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == ':')
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_turbofish() {
        assert_eq!(strip_turbofish("Foo::<T>::new"), "Foo::new");
        assert_eq!(strip_turbofish("Foo::<A, B<C>>::"), "Foo::");
        assert_eq!(strip_turbofish("path::Foo::<T>::"), "path::Foo::");
        assert_eq!(
            strip_turbofish("Vec::<u8>::with_capacity"),
            "Vec::with_capacity"
        );
        assert_eq!(strip_turbofish("a < b::"), "a < b::");
        assert_eq!(strip_turbofish("<T as Tr>::"), "<T as Tr>::");
        assert_eq!(strip_turbofish("Foo::<'a, T>::"), "Foo::");
        assert_eq!(strip_turbofish("Foo::<'static, T>::"), "Foo::");
        assert_eq!(strip_turbofish("Foo::<\"hello\", T>::"), "Foo::");
        assert_eq!(strip_turbofish("Foo::<'c', T>::"), "Foo::");
        assert_eq!(strip_turbofish("Foo::<fn() -> u8>::new"), "Foo::new");
        assert_eq!(
            strip_turbofish("Foo::<fn(i32) -> Result<u8, ()>>::"),
            "Foo::"
        );
        assert_eq!(strip_turbofish(r"Foo::<'\u{10FFFF}', T>::"), "Foo::");
        assert_eq!(strip_turbofish("Foo::<'🦀', T>::"), "Foo::");
        assert_eq!(
            strip_turbofish("QueryState::<&A, Without<C>>::"),
            "QueryState::"
        );
        assert_eq!(
            strip_turbofish("EventTriggerState::<E>::"),
            "EventTriggerState::"
        );
    }

    #[test]
    fn test_extract_colon_qualifier() {
        assert_eq!(extract_colon_qualifier("use core::cell::"), "core::cell");
        assert_eq!(
            extract_colon_qualifier("    use alloc::{string::"),
            "alloc::string"
        );
        assert_eq!(
            extract_colon_qualifier("    use alloc::{vec::Vec, string::"),
            "alloc::string"
        );
        assert_eq!(
            extract_colon_qualifier("unsafe { UniqueEntityEquivalentSlice::"),
            "UniqueEntityEquivalentSlice"
        );
        assert_eq!(extract_colon_qualifier("match x { Some(v) => Foo::"), "Foo");

        // Turbofish test cases
        assert_eq!(extract_colon_qualifier("Foo::<T>::new"), "Foo");
        assert_eq!(extract_colon_qualifier("Foo::<A, B<C>>::"), "Foo");
        assert_eq!(extract_colon_qualifier("path::Foo::<T>::"), "path::Foo");
        assert_eq!(extract_colon_qualifier("Vec::<u8>::with_capacity"), "Vec");
        assert_eq!(
            extract_colon_qualifier("QueryState::<&A, Without<C>>::"),
            "QueryState"
        );
        assert_eq!(
            extract_colon_qualifier("EventTriggerState::<E>::"),
            "EventTriggerState"
        );

        // UFCS test cases
        assert_eq!(extract_colon_qualifier("<T as Tr>::"), "Tr");
        assert_eq!(
            extract_colon_qualifier("<T as my::Trait<U>>::"),
            "my::Trait"
        );
        assert_eq!(
            extract_colon_qualifier("<Option<<T as Other>::Item> as Trait>::"),
            "Trait"
        );
        assert_eq!(extract_colon_qualifier("<&set::Slice<K>>::"), "set::Slice");
        assert_eq!(extract_colon_qualifier("<&'a mut Slice<K>>::"), "Slice");
        assert_eq!(extract_colon_qualifier("<*const u8>::"), "u8");
        assert_eq!(extract_colon_qualifier("<*mut u8>::"), "u8");
        assert_eq!(extract_colon_qualifier("<T>::"), "T");
        assert_eq!(extract_colon_qualifier("<Grüße>::"), "Grüße");
        assert_eq!(extract_colon_qualifier("<T as Grüße>::"), "Grüße");
        assert_eq!(
            extract_colon_qualifier("unsafe { Slice::from_slice_unchecked(<&set::Slice<K>>::"),
            "set::Slice"
        );

        // Non-turbofish and negative tests
        assert_eq!(extract_colon_qualifier("a < b::"), "b");
    }
}
