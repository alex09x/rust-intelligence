/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! UFCS (Universal Function Call Syntax) qualifier extraction and modifier peeling.

/// Finds the starting index of a trailing UFCS `<...>` qualifier in `before_colon`.
///
/// Returns `Some(start_idx)` if `before_colon` ends with a balanced `<...>` construct
/// that is not attached to an identifier.
pub fn find_ufcs_start(before_colon: &str) -> Option<usize> {
    if !before_colon.ends_with('>') || before_colon.ends_with("->") || before_colon.ends_with("=>")
    {
        return None;
    }
    let bytes = before_colon.as_bytes();
    let mut depth = 0i32;
    let mut start = None;
    let mut j = bytes.len();

    while j > 0 {
        j -= 1;
        let b = bytes[j];
        if b == b'>' {
            if j > 0 && (bytes[j - 1] == b'-' || bytes[j - 1] == b'=') {
                j -= 1;
                continue;
            }
            depth += 1;
        } else if b == b'<' {
            depth -= 1;
            if depth == 0 {
                start = Some(j);
                break;
            }
        }
    }

    let start_idx = start?;
    if start_idx > 0 {
        if let Some(prev) = before_colon[..start_idx].chars().last() {
            if prev.is_alphanumeric() || prev == '_' {
                return None;
            }
        }
    }
    Some(start_idx)
}

/// Peels reference, pointer, lifetime, and dyn modifiers from a type string.
pub fn peel_type_modifiers(mut s: &str) -> &str {
    s = s.trim();
    loop {
        if let Some(rest) = s.strip_prefix('&') {
            s = rest.trim_start();
            if s.starts_with('\'') {
                if let Some(end_lt) =
                    s.find(|c: char| !c.is_alphanumeric() && c != '_' && c != '\'')
                {
                    s = s[end_lt..].trim_start();
                } else {
                    s = "";
                }
            }
            if let Some(rest_mut) = s.strip_prefix("mut") {
                if rest_mut.starts_with(char::is_whitespace) || rest_mut.is_empty() {
                    s = rest_mut.trim_start();
                }
            }
            continue;
        }
        if let Some(rest) = s.strip_prefix("*const") {
            if rest.starts_with(char::is_whitespace) {
                s = rest.trim_start();
                continue;
            }
        }
        if let Some(rest) = s.strip_prefix("*mut") {
            if rest.starts_with(char::is_whitespace) {
                s = rest.trim_start();
                continue;
            }
        }
        if let Some(rest) = s.strip_prefix("dyn") {
            if rest.starts_with(char::is_whitespace) {
                s = rest.trim_start();
                continue;
            }
        }
        break;
    }
    s
}

/// Strips generic arguments (`<...>`) from a type path string.
pub fn strip_type_generics(s: &str) -> String {
    let mut res = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    let mut depth = 0i32;

    while let Some(c) = chars.next() {
        if (c == '-' || c == '=') && chars.peek() == Some(&'>') {
            if depth == 0 {
                res.push(c);
                res.push('>');
            }
            chars.next();
            continue;
        }
        if c == '<' {
            depth += 1;
        } else if c == '>' {
            if depth > 0 {
                depth -= 1;
            }
        } else if depth == 0 {
            res.push(c);
        }
    }
    res.trim().trim_end_matches(':').to_string()
}

/// Extracts the target qualifier from a UFCS construct preceding `::`.
///
/// Supports:
/// - `<Type as Trait>::` -> `Trait` (candidates: items from trait)
/// - `<Type>::` / `<&Type>::` -> `Type` (peeling references, pointers, and generic args)
pub fn extract_ufcs_qualifier(before_colon: &str) -> Option<String> {
    let start_idx = find_ufcs_start(before_colon)?;
    let inner = before_colon[start_idx + 1..before_colon.len() - 1].trim();
    if inner.is_empty() {
        return None;
    }

    // 1. Check for explicit trait qualification: `<Type as Trait>`
    let mut depth = 0i32;
    let mut as_idx = None;
    let mut chars = inner.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if (c == '-' || c == '=') && chars.peek().map(|&(_, next_c)| next_c) == Some('>') {
            chars.next();
            continue;
        }
        if c == '<' {
            depth += 1;
        } else if c == '>' {
            if depth > 0 {
                depth -= 1;
            }
        } else if depth == 0 && inner[i..].starts_with(" as ") {
            as_idx = Some(i);
            break;
        }
    }

    if let Some(idx) = as_idx {
        let trait_part = inner[idx + 4..].trim();
        let stripped = strip_type_generics(trait_part);
        if !stripped.is_empty() {
            return Some(stripped);
        }
    }

    // 2. Inherent / type qualification: `<Type>` or `<&Type>`
    let peeled = peel_type_modifiers(inner);
    let stripped = strip_type_generics(peeled);
    if !stripped.is_empty() {
        return Some(stripped);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_ufcs_start() {
        assert_eq!(find_ufcs_start("<T as Tr>"), Some(0));
        assert_eq!(
            find_ufcs_start("unsafe { Slice::from_slice_unchecked(<&set::Slice<K>>"),
            Some(37)
        );
        assert_eq!(find_ufcs_start("foo<T>"), None);
        assert_eq!(find_ufcs_start("a < b"), None);
        assert_eq!(find_ufcs_start("a ->"), None);
    }

    #[test]
    fn test_peel_type_modifiers() {
        assert_eq!(peel_type_modifiers("&set::Slice<K>"), "set::Slice<K>");
        assert_eq!(peel_type_modifiers("&'a mut Slice<K>"), "Slice<K>");
        assert_eq!(peel_type_modifiers("&'static str"), "str");
        assert_eq!(peel_type_modifiers("*const u8"), "u8");
        assert_eq!(peel_type_modifiers("*mut u8"), "u8");
        assert_eq!(peel_type_modifiers("dyn MyTrait"), "MyTrait");
    }

    #[test]
    fn test_extract_ufcs_qualifier() {
        assert_eq!(extract_ufcs_qualifier("<T as Tr>"), Some("Tr".to_string()));
        assert_eq!(
            extract_ufcs_qualifier("<T as my::Trait<U>>"),
            Some("my::Trait".to_string())
        );
        assert_eq!(
            extract_ufcs_qualifier("<Option<<T as Other>::Item> as Trait>"),
            Some("Trait".to_string())
        );
        assert_eq!(
            extract_ufcs_qualifier("<&set::Slice<K>>"),
            Some("set::Slice".to_string())
        );
        assert_eq!(
            extract_ufcs_qualifier("<&'a mut Slice<K>>"),
            Some("Slice".to_string())
        );
        assert_eq!(
            extract_ufcs_qualifier("<*const u8>"),
            Some("u8".to_string())
        );
        assert_eq!(extract_ufcs_qualifier("<*mut u8>"), Some("u8".to_string()));
        assert_eq!(extract_ufcs_qualifier("<T>"), Some("T".to_string()));
        assert_eq!(extract_ufcs_qualifier("<Grüße>"), Some("Grüße".to_string()));
        assert_eq!(
            extract_ufcs_qualifier("<T as Grüße>"),
            Some("Grüße".to_string())
        );
    }

    #[test]
    fn test_strip_type_generics() {
        assert_eq!(strip_type_generics("Option<T>"), "Option");
        assert_eq!(strip_type_generics("Map<K, V>"), "Map");
        assert_eq!(strip_type_generics("Grüße<T>"), "Grüße");
        assert_eq!(strip_type_generics("Fn(A) -> B"), "Fn(A) -> B");
    }
}
