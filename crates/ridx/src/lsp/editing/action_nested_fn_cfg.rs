/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! `Convert nested function to closure` of a cfg-gated nested fn: its `#[cfg]` stays on the `let`, and the
//! same-named twins under the other cfgs (`#[cfg(not(feature = "x"))] fn imp`) are not taken for its uses.

use super::action_parse_decl::next_ident_token;
use super::action_parse_items::{contains_word, skip_balanced};

/// Whether the attributes between `lo` and `hi` all stay valid on the `let` that replaces the fn: `#[cfg(..)]` and
/// the lint levels. `#[inline]`, `#[must_use]`, `#[track_caller]` .. are rejected on a statement, and a `cfg_attr`
/// may add any of them.
pub(crate) fn only_statement_attrs(masked: &str, lo: usize, hi: usize) -> bool {
    const KEPT: &[&str] = &["#[cfg(", "#[allow(", "#[warn(", "#[deny(", "#[expect(", "#[forbid("];
    let b = masked.as_bytes();
    let mut i = lo;
    while i < hi {
        if b[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if !KEPT.iter().any(|k| masked[i..hi].starts_with(k)) {
            return false;
        }
        let Some(end) = skip_balanced(b, i + 1, b'[', b']') else { return false };
        if end > hi {
            return false;
        }
        i = end;
    }
    true
}

/// The masked text with every other `#[cfg(..)] fn NAME` item of the block `(open, close)` blanked out, so that only
/// real uses of NAME remain. `own` is the converted fn's span. `None` when a twin cannot be delimited.
pub(crate) fn blank_cfg_twins(masked: &str, open: usize, close: usize, own: (usize, usize), name: &str) -> Option<String> {
    let b = masked.as_bytes();
    let mut out = masked.as_bytes().to_vec();
    let mut i = open + 1;
    while i < close {
        if (own.0..own.1).contains(&i) {
            i = own.1;
            continue;
        }
        let word_start = b[i].is_ascii_alphabetic() && (i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_'));
        if !word_start || !masked[i..].starts_with("fn") {
            i += 1;
            continue;
        }
        let Some((kw, after_kw)) = next_ident_token(masked, i) else { break };
        if kw != "fn" || next_ident_token(masked, after_kw).map(|(n, _)| n) != Some(name) {
            i = after_kw;
            continue;
        }
        // The twin's single `#[cfg(..)]` right above it, and its body.
        let pre = masked[open + 1..i].trim_end();
        let attr_at = pre.rfind("#[")? + open + 1;
        if !masked[attr_at..].starts_with("#[cfg(") || skip_balanced(b, attr_at + 1, b'[', b']')? != open + 1 + pre.len() {
            return None;
        }
        let body_open = i + masked[i..close].find('{')?;
        let body_close = skip_balanced(b, body_open, b'{', b'}')?;
        for c in &mut out[attr_at..body_close] {
            if *c != b'\n' {
                *c = b' ';
            }
        }
        i = body_close;
    }
    let text = String::from_utf8(out).ok()?;
    // Some use of NAME other than a twin's own text must remain for the conversion to mean anything.
    contains_word(&text[own.1..close], name).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::{blank_cfg_twins, only_statement_attrs};

    #[test]
    fn cfg_twins_are_not_uses() {
        let m = "{ #[cfg(not(f))] fn imp() -> u8 { 0 } #[cfg(f)] fn imp() -> u8 { 1 } imp() }";
        let own_lo = m.find("#[cfg(f)]").unwrap() + "#[cfg(f)] ".len();
        let own_hi = m.rfind("imp()").unwrap() - 1;
        let blanked = blank_cfg_twins(m, 0, m.len() - 1, (own_lo, own_hi), "imp").unwrap();
        assert!(!blanked[..own_lo].contains("imp"), "{blanked}");
        assert!(only_statement_attrs(m, m.find("#[cfg(f)]").unwrap(), own_lo) && only_statement_attrs("#[allow(unused)] ", 0, 17));
        assert!(!only_statement_attrs("#[inline] ", 0, 10) && !only_statement_attrs("#[cfg_attr(all(), inline)] ", 0, 27));
        // A twin without its own `#[cfg]` is a real duplicate: no conversion.
        assert!(blank_cfg_twins("{ fn imp() {} fn imp() {} imp() }", 0, 32, (14, 24), "imp").is_none());
    }
}
