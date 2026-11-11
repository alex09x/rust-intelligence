/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Parameter and receiver formatting helpers for `Inline into all callers`.

pub(crate) use super::action_inline_callers_format::format_inlined_call;
pub(crate) use super::action_inline_callers_params::{is_primitive_scalar, is_writable_binding_type, normalize_param_type_for_binding, parse_fn_params, FnParamInfo, SelfKind};

/// Where the receiver of the method call whose `.` is at `dot_byte` starts: the whole chain before it (`a.b()?.c()`
/// for `.d()`), which the inlined block binds once, as the call evaluated it once; and whether a call follows another
/// link in it (`a.b().c()`): such a chain may return a reference into a temporary of the chain, which only a `match`
/// keeps alive for the whole inlined block (a `let` drops it at its `;`, E0716).
pub(crate) fn receiver_chain_start(src: &str, masked: &str, dot_byte: usize) -> Option<(usize, bool)> {
    let mb = masked.as_bytes();
    let mut chained = false;
    // A `?` ends the link before it (`a.b()?.c()`).
    let link_end = |mut e: usize| {
        while e > 0 && mb[e - 1].is_ascii_whitespace() {
            e -= 1;
        }
        if e > 0 && mb[e - 1] == b'?' {
            e -= 1;
        }
        e
    };
    let first_end = link_end(dot_byte);
    let mut q = receiver_link_start(src, masked, first_end)?;
    let mut later_call = mb[q..first_end].contains(&b'(') || first_end < dot_byte;
    loop {
        let mut r = q;
        while r > 0 && mb[r - 1].is_ascii_whitespace() {
            r -= 1;
        }
        if r > 1 && mb[r - 1] == b'.' && mb[r - 2] != b'.' {
            chained |= later_call;
            let end = link_end(r - 1);
            let head = receiver_link_start(src, masked, end)?;
            later_call = mb[head..end].contains(&b'(') || end < r - 1;
            q = head;
            continue;
        }
        return Some((q, chained));
    }
}

/// Where one link of a receiver chain ending before byte `dot_byte` starts: a call or macro call with its name, an
/// index with its base, a block, or a path of identifiers and fields.
fn receiver_link_start(_src: &str, masked: &str, dot_byte: usize) -> Option<usize> {
    let mb = masked.as_bytes();
    let mut p = dot_byte;
    while p > 0 && mb[p - 1].is_ascii_whitespace() {
        p -= 1;
    }
    if p == 0 {
        return None;
    }
    // A turbofish ending a path (`add::<i32>`, `f::<T>(x)`): the path before its `::<`.
    let before_turbofish = |q: usize| -> usize {
        if q > 0 && mb[q - 1] == b'>' {
            if let Some(open) = skip_balanced_backwards(mb, q - 1, b'>', b'<') {
                if open >= 2 && &mb[open - 2..open] == b"::" {
                    return open - 2;
                }
            }
        }
        q
    };
    let p = before_turbofish(p);
    if p == 0 {
        return None;
    }
    let last = mb[p - 1];
    if last == b')' {
        let open = skip_balanced_backwards(mb, p - 1, b')', b'(')?;
        let mut q = open;
        while q > 0 && mb[q - 1].is_ascii_whitespace() {
            q -= 1;
        }
        // A macro call (`bytes::regex!("foo")`) is the receiver with its name.
        if q > 0 && mb[q - 1] == b'!' {
            q -= 1;
        }
        // The called path, through its turbofishes (`Vec::<u8>::new`, `f::<T>`).
        loop {
            q = before_turbofish(q);
            let seg_end = q;
            while q > 0 && (mb[q - 1].is_ascii_alphanumeric() || mb[q - 1] == b'_' || mb[q - 1] == b':') {
                q -= 1;
            }
            if q == seg_end || !(q > 0 && mb[q - 1] == b'>' && masked[q..].starts_with("::")) {
                break;
            }
        }
        return Some(q);
    }
    if last == b']' {
        let open = skip_balanced_backwards(mb, p - 1, b']', b'[')?;
        let mut q = open;
        while q > 0 && (mb[q - 1].is_ascii_alphanumeric() || mb[q - 1] == b'_' || mb[q - 1] == b'.') {
            q -= 1;
        }
        return Some(q);
    }
    if last == b'}' {
        return skip_balanced_backwards(mb, p - 1, b'}', b'{');
    }
    let mut q = p;
    while q > 0 {
        let b = mb[q - 1];
        if b.is_ascii_alphanumeric() || b == b'_' || (b == b'.' && q > 1 && (mb[q - 2].is_ascii_alphanumeric() || mb[q - 2] == b'_')) {
            q -= 1;
        } else {
            break;
        }
    }
    if q < p && mb[q] != b'.' {
        return Some(q);
    }
    None
}

fn skip_balanced_backwards(bytes: &[u8], close_idx: usize, close_ch: u8, open_ch: u8) -> Option<usize> {
    if bytes.get(close_idx)? != &close_ch {
        return None;
    }
    let mut depth = 0usize;
    let mut i = close_idx;
    loop {
        let b = bytes[i];
        if b == close_ch {
            depth += 1;
        } else if b == open_ch {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
        if i == 0 {
            break;
        }
        i -= 1;
    }
    None
}

pub(crate) fn find_path_start(masked: &str, ref_byte: usize) -> usize {
    let before = &masked[..ref_byte];
    let trimmed = before.trim_end();
    if !trimmed.ends_with("::") {
        return ref_byte;
    }
    let mb = masked.as_bytes();
    let mut p = trimmed.len() - 2;
    loop {
        while p > 0 && mb[p - 1].is_ascii_whitespace() {
            p -= 1;
        }
        if p == 0 {
            return 0;
        }
        if mb[p - 1] == b'>' {
            if let Some(open) = skip_balanced_backwards(mb, p - 1, b'>', b'<') {
                p = open;
                while p > 0 && mb[p - 1].is_ascii_whitespace() {
                    p -= 1;
                }
                if p >= 2 && &mb[p - 2..p] == b"::" {
                    p -= 2;
                    while p > 0 && mb[p - 1].is_ascii_whitespace() {
                        p -= 1;
                    }
                }
            } else {
                break;
            }
        }
        let id_end = p;
        while p > 0 && (mb[p - 1].is_ascii_alphanumeric() || mb[p - 1] == b'_') {
            p -= 1;
        }
        if p == id_end {
            break;
        }
        let mut q = p;
        while q > 0 && mb[q - 1].is_ascii_whitespace() {
            q -= 1;
        }
        if q >= 2 && &mb[q - 2..q] == b"::" {
            p = q - 2;
        } else {
            break;
        }
    }
    p
}

pub(crate) fn contains_recursive_call(masked_body: &str, fn_name: &str) -> bool {
    let mb = masked_body.as_bytes();
    // A local of the same name (`let new = |r| …;`) is what a bare `new(…)` calls.
    let shadowed = super::action_parse_items::contains_word(masked_body, "let")
        && masked_body.match_indices("let ").any(|(p, _)| {
            let rest = masked_body[p + 4..].trim_start();
            let rest = rest.strip_prefix("mut ").map_or(rest, str::trim_start);
            rest.starts_with(fn_name) && !rest[fn_name.len()..].starts_with(|c: char| c.is_alphanumeric() || c == '_')
        });
    let mut start = 0;
    while let Some(idx) = masked_body[start..].find(fn_name) {
        let abs_idx = start + idx;
        let before_ok = abs_idx == 0 || (!mb[abs_idx - 1].is_ascii_alphanumeric() && mb[abs_idx - 1] != b'_');
        let after_idx = abs_idx + fn_name.len();
        let after_ok = after_idx == mb.len() || (!mb[after_idx].is_ascii_alphanumeric() && mb[after_idx] != b'_');
        if before_ok && after_ok {
            let mut p = after_idx;
            while p < mb.len() && mb[p].is_ascii_whitespace() {
                p += 1;
            }
            if p < mb.len() && mb[p] == b'(' {
                if abs_idx > 0 && mb[abs_idx - 1] == b'.' {
                    let before_dot = masked_body[..abs_idx - 1].trim_end();
                    if before_dot.ends_with("self") {
                        let self_start = before_dot.len() - 4;
                        let self_before_ok = self_start == 0 || (!before_dot.as_bytes()[self_start - 1].is_ascii_alphanumeric() && before_dot.as_bytes()[self_start - 1] != b'_');
                        if self_before_ok {
                            return true;
                        }
                    }
                } else if abs_idx >= 2 && &mb[abs_idx - 2..abs_idx] == b"::" {
                    let before_colons = masked_body[..abs_idx - 2].trim_end();
                    if before_colons.ends_with("Self") {
                        return true;
                    }
                } else if !shadowed {
                    return true;
                }
            }
        }
        start = abs_idx + fn_name.len();
    }
    false
}

pub(crate) fn is_caller_return_compatible(src: &str, masked: &str, starts: &[usize], ref_byte: usize, callee_ret: &str) -> bool {
    let callee_is_result = callee_ret.contains("Result");
    let callee_is_option = callee_ret.contains("Option");
    if !callee_is_result && !callee_is_option {
        return false;
    }
    let Some(caller_fn) = super::action_parse_enclosing::find_enclosing_fn_at_cursor(src, masked, starts, ref_byte) else {
        return false;
    };
    let super::action_parse::ItemKindInfo::Fn { ret_src, .. } = &caller_fn.kind else {
        return false;
    };
    let Some(r) = ret_src.as_deref() else {
        return false;
    };
    if callee_is_result {
        r.contains("Result")
    } else if callee_is_option {
        r.contains("Option")
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::receiver_chain_start;

    #[test]
    fn a_macro_call_receiver_keeps_its_name() {
        let src = "assert!(bytes::regex!(\"foo\").is_match(b\"foo\"));";
        let dot = src.find(").is_match").unwrap() + 1;
        assert_eq!(receiver_chain_start(src, src, dot), src.find("bytes::regex!").map(|p| (p, false)));
        // A chain is the receiver as a whole; one where a call follows another link (or a `?`) may borrow from a
        // temporary of the chain, which is said so that a `match` binds it.
        let headed = "x = Builder::new().cfg.items.is_match(1);";
        assert_eq!(receiver_chain_start(headed, headed, headed.find(".is_match").unwrap()), headed.find("Builder").map(|p| (p, false)));
        let plain = "x = a.b(c).is_match(1);";
        assert_eq!(receiver_chain_start(plain, plain, plain.find(").is").unwrap() + 1), Some((4, true)));
        let tried = "let y = self.cfg.get(k)?.value().is_match(1);";
        assert_eq!(receiver_chain_start(tried, tried, tried.find(".is_match").unwrap()), Some((8, true)));
        let fished = "let f = add::<i32>.into_function_mut().with_overload(g);";
        assert_eq!(receiver_chain_start(fished, fished, fished.find(".with_overload").unwrap()), fished.find("add::").map(|p| (p, true)));
        let built = "x = Vec::<u8>::new().is_empty();";
        assert_eq!(receiver_chain_start(built, built, built.find(".is_empty").unwrap()), built.find("Vec").map(|p| (p, false)));
        let range = "for i in 0..n.len() {}";
        assert_eq!(receiver_chain_start(range, range, range.find(".len").unwrap()), range.find("n.len").map(|p| (p, false)));
        let body = "for i in 0..self.n { x.self_y(self) }";
        assert_eq!(super::super::action_inline_callers_transform::rewrite_self_to_this(body, body), "for i in 0..this.n { x.self_y(this) }");
    }
}
