/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Type name sanitization for CAS cache reproducibility and path leak prevention.

/// Sanitizes compiler-inferred type names to strip local file paths,
/// guaranteeing bit-for-bit CAS shard reproducibility and privacy.
///
/// Specifically:
/// 1. Strips span locations from unnameable closure/coroutine types:
///    `{closure@/Users/alex09x/project/src/lib.rs:42:15}` -> `{closure}`
///    `{async fn body@/home/runner/...:10:20}` -> `{async fn body}`
/// 2. Redacts any accidental user home directory path fragments (`/Users/<user>/` or `/home/<user>/`).
pub fn sanitize_type_name(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let mut i = 0;

    while i < n {
        if chars[i] == '{' {
            out.push('{');
            i += 1;
            let start = i;
            let mut has_at = false;
            let mut at_pos = 0;
            while i < n && chars[i] != '}' && chars[i] != '{' {
                if chars[i] == '@' {
                    has_at = true;
                    at_pos = i;
                }
                i += 1;
            }
            if has_at && i < n && chars[i] == '}' {
                // We found `{tag@path}`. Keep `tag`, drop `@path`.
                for c in &chars[start..at_pos] {
                    out.push(*c);
                }
                out.push('}');
                i += 1;
            } else {
                for c in &chars[start..i] {
                    out.push(*c);
                }
            }
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }

    if out.contains("/Users/") || out.contains("/home/") {
        redact_home_paths(&out)
    } else {
        out
    }
}

/// Fallback helper to redact `/Users/<user>` or `/home/<user>` prefixes.
fn redact_home_paths(s: &str) -> String {
    let mut result = s.to_string();
    for prefix in &["/Users/", "/home/"] {
        while let Some(pos) = result.find(prefix) {
            let after = pos + prefix.len();
            let end = result[after..]
                .find(|c: char| {
                    c == '/' || c == '>' || c == ',' || c == ')' || c == '}' || c.is_whitespace()
                })
                .map(|p| after + p)
                .unwrap_or(result.len());
            result.replace_range(pos..end, "<user_dir>");
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_closure_simple() {
        let input = "{closure@/Users/alex09x/rust-intelligence/src/lib.rs:12:34}";
        assert_eq!(sanitize_type_name(input), "{closure}");
    }

    #[test]
    fn test_sanitize_async_fn_body() {
        let input = "{async fn body@/home/runner/work/repo/src/main.rs:10:20}";
        assert_eq!(sanitize_type_name(input), "{async fn body}");
    }

    #[test]
    fn test_sanitize_coroutine() {
        let input = "{coroutine@src/gen.rs:1:1}";
        assert_eq!(sanitize_type_name(input), "{coroutine}");
    }

    #[test]
    fn test_sanitize_nested_iterators() {
        let input = "Map<Filter<Chars<'_>, {closure@/Users/alex/foo.rs:1:2}>, {closure@/home/ci/bar.rs:3:4}>";
        assert_eq!(
            sanitize_type_name(input),
            "Map<Filter<Chars<'_>, {closure}>, {closure}>"
        );
    }

    #[test]
    fn test_sanitize_standard_types_untouched() {
        let input = "alloc::vec::Vec<u8>";
        assert_eq!(sanitize_type_name(input), "alloc::vec::Vec<u8>");

        let input2 = "&'a mut [core::option::Option<i32>]";
        assert_eq!(sanitize_type_name(input2), input2);
    }

    #[test]
    fn test_sanitize_fallback_home_paths() {
        let input = "CustomType</Users/alex09x/some/path>";
        assert_eq!(sanitize_type_name(input), "CustomType<<user_dir>/some/path>");
    }
}
