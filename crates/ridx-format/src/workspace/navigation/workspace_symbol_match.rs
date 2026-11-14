/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! How workspace symbol search matches a query against a name: substring, subsequence, and rust-analyzer's
//! smart-case subsequence.

pub(super) fn contains_ascii_ci(haystack: &[u8], needle_lower: &[u8]) -> bool {
    if needle_lower.is_empty() {
        return true;
    }
    if haystack.len() < needle_lower.len() {
        return false;
    }
    let first = needle_lower[0];
    let first_upper = first.to_ascii_uppercase();
    let max_start = haystack.len() - needle_lower.len();
    for i in 0..=max_start {
        let b = haystack[i];
        if (b == first || b == first_upper) && haystack[i + 1..i + needle_lower.len()].eq_ignore_ascii_case(&needle_lower[1..]) {
            return true;
        }
    }
    false
}

pub(super) fn is_fuzzy_subseq(name: &str, query: &str) -> bool {
    if !(2..=12).contains(&query.len()) || name.len() < query.len() || !name.is_ascii() || !query.is_ascii() {
        return false;
    }
    let has_upper = query.bytes().any(|b| b.is_ascii_uppercase());
    let mut nb = name.bytes();
    for qb in query.bytes() {
        let found = if has_upper && qb.is_ascii_uppercase() { nb.any(|c| c == qb) } else { nb.any(|c| c.eq_ignore_ascii_case(&qb)) };
        if !found {
            return false;
        }
    }
    true
}

/// rust-analyzer's symbol match: the query is a subsequence of the name, compared case-insensitively except for the
/// query's own uppercase letters when it has any.
pub(super) fn ra_fuzzy_match(name: &str, query: &str, case_sensitive: bool) -> bool {
    if name.len() < query.len() {
        return false;
    }
    // Smart case: with an uppercase letter in the query its uppercase letters must match exactly; lowercase ones
    // always match either case (`oolVisito` finds `TagContentOtherFieldVisitor`, `Option` misses `option`).
    if name.is_ascii() && query.is_ascii() {
        let mut nb = name.bytes();
        return query.bytes().all(|q| nb.any(|n| if case_sensitive && q.is_ascii_uppercase() { n == q } else { n.eq_ignore_ascii_case(&q) }));
    }
    let mut name_chars = name.chars();
    query.chars().all(|q| name_chars.any(|n| if case_sensitive && q.is_uppercase() { n == q } else { n.to_lowercase().eq(q.to_lowercase()) }))
}
