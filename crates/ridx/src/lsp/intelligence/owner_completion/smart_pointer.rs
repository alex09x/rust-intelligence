/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Smart pointer identification and inner type extraction for scoped completion.

use super::trait_filter::base_type_path;
use super::ufcs::peel_type_modifiers;

/// Checks whether the owner represents a known standard smart pointer (`Box`, `Arc`, `Rc`).
pub fn is_smart_pointer(owner: &str) -> bool {
    let b = base_type_path(owner);
    let short = b.rsplit("::").next().unwrap_or(b);
    matches!(short, "Box" | "Arc" | "Rc")
}

/// Checks whether the owner is a smart pointer or generic `Deref<Target = T>` guard/wrapper.
pub fn is_generic_deref_wrapper(owner: &str) -> bool {
    let b = base_type_path(owner);
    let short = b.rsplit("::").next().unwrap_or(b);
    matches!(
        short,
        "Box"
            | "Arc"
            | "Rc"
            | "Ref"
            | "RefMut"
            | "Cow"
            | "MutexGuard"
            | "RwLockReadGuard"
            | "RwLockWriteGuard"
            | "MappedMutexGuard"
            | "MappedRwLockReadGuard"
            | "MappedRwLockWriteGuard"
            | "ManuallyDrop"
            | "Pin"
            | "LazyLock"
            | "OnceLock"
            | "AssertUnwindSafe"
            | "Res"
            | "ResMut"
            | "Mut"
            | "NonSend"
            | "NonSendMut"
            | "Local"
            | "Single"
            | "PoolGuard"
            | "Spawn"
    )
}

/// Extracts the `index`-th non-lifetime type argument from a generic type expression.
pub fn extract_nth_non_lifetime_type_arg(ty_name: &str, index: usize) -> Option<&str> {
    let s = peel_type_modifiers(ty_name);
    let start = s.find('<')?;
    let end = s.rfind('>')?;
    if start >= end {
        return None;
    }
    let inner = s[start + 1..end].trim();
    if inner.is_empty() {
        return None;
    }
    let bytes = inner.as_bytes();
    let mut depth = 0usize;
    let mut seg_start = 0usize;
    let mut non_lt_idx = 0usize;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'<' | b'(' | b'[' => depth += 1,
            b'>' | b')' | b']' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => {
                let seg = inner[seg_start..i].trim();
                if !seg.is_empty() && !seg.starts_with('\'') && !seg.contains('=') {
                    if non_lt_idx == index {
                        return Some(peel_type_modifiers(seg));
                    }
                    non_lt_idx += 1;
                }
                seg_start = i + 1;
            }
            _ => {}
        }
    }
    let seg = inner[seg_start..].trim();
    if !seg.is_empty() && !seg.starts_with('\'') && !seg.contains('=') && non_lt_idx == index {
        return Some(peel_type_modifiers(seg));
    }
    None
}

/// Extracts the inner type argument from a smart pointer type expression (e.g. `Box<dyn Trait>` -> `Trait`).
pub fn extract_smart_pointer_inner_type(ty_name: &str) -> Option<&str> {
    let start = ty_name.find('<')?;
    let end = ty_name.rfind('>')?;
    if start >= end {
        return None;
    }
    let mut inner = ty_name[start + 1..end].trim();
    if let Some(rest) = inner.strip_prefix("dyn ") {
        inner = rest.trim();
    }
    let bytes = inner.as_bytes();
    let mut depth = 0usize;
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'<' {
            depth += 1;
        } else if b == b'>' {
            depth = depth.saturating_sub(1);
        } else if b == b',' && depth == 0 {
            inner = inner[..i].trim();
            break;
        }
    }
    if inner.is_empty() {
        None
    } else {
        Some(inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_smart_pointer() {
        assert!(is_smart_pointer("Box"));
        assert!(is_smart_pointer("alloc::boxed::Box"));
        assert!(is_smart_pointer("Arc"));
        assert!(is_smart_pointer("alloc::sync::Arc"));
        assert!(is_smart_pointer("Rc"));
        assert!(is_smart_pointer("alloc::rc::Rc"));
        assert!(!is_smart_pointer("Vec"));
        assert!(!is_smart_pointer("Option"));
    }

    #[test]
    fn test_extract_smart_pointer_inner_type() {
        assert_eq!(extract_smart_pointer_inner_type("Box<dyn MyTrait>"), Some("MyTrait"));
        assert_eq!(extract_smart_pointer_inner_type("alloc::sync::Arc<Foo, Global>"), Some("Foo"));
        assert_eq!(extract_smart_pointer_inner_type("Rc<Bar<i32>>"), Some("Bar<i32>"));
        assert_eq!(extract_smart_pointer_inner_type("Box<>"), None);
        assert_eq!(extract_smart_pointer_inner_type("NotGeneric"), None);
    }
}
