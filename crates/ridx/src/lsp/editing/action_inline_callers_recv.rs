/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! How a `&mut self` method's receiver is bound when the index gives no type for it: by its declaration in the
//! calling function. A binding that holds a `&mut` is reborrowed (`&mut *x`); a `mut` binding holding a value is
//! borrowed (`&mut x`), as the method call's auto-ref does; a field of a `&mut self` is borrowed (`&mut self.f`).

/// The receiver binding a `&mut self` call needs.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum MutRecv {
    /// `let mut this = &mut *x;`: `x` holds a `&mut`.
    Reborrow,
    /// `let mut this = &mut x;`: `x` is a `mut` binding of a value.
    Borrow,
}

fn is_id(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// Occurrences of `word` in `text` as a whole identifier.
fn word_at(text: &str, word: &str) -> Vec<usize> {
    let b = text.as_bytes();
    text.match_indices(word).map(|(p, _)| p).filter(|&p| (p == 0 || !is_id(b[p - 1])) && b.get(p + word.len()).is_none_or(|&c| !is_id(c))).collect()
}

/// The binding for receiver `recv` of a call at `call_start` in `masked` (the caller's document, comments and strings
/// masked), from the declaration closest before the call in the enclosing function; `None` when it cannot be told.
pub(crate) fn mut_receiver_binding(masked: &str, call_start: usize, recv: &str) -> Option<MutRecv> {
    let fn_at = word_at(&masked[..call_start], "fn").last().copied()?;
    let scope = &masked[fn_at..call_start];
    // `self.field` (a chain of fields) in a method taking `&mut self`.
    if let Some(rest) = recv.strip_prefix("self.") {
        let params = &scope[scope.find('(')?..];
        let takes_mut_self = params.trim_start_matches('(').trim_start().starts_with("&mut self") || params.contains("&'_ mut self");
        let fields = rest.split('.').all(|s| !s.is_empty() && s.bytes().all(is_id) && !s.as_bytes()[0].is_ascii_digit());
        return (takes_mut_self && fields).then_some(MutRecv::Borrow);
    }
    if recv.is_empty() || !recv.bytes().all(is_id) || recv == "self" {
        return None;
    }
    let decl = word_at(scope, recv).into_iter().rev().find_map(|p| {
        let before = scope[..p].trim_end();
        let after = scope[p + recv.len()..].trim_start();
        let mut_kw = before.ends_with("mut") && !before[..before.len() - 3].ends_with(|c: char| is_id(c as u8));
        let lead = if mut_kw { before[..before.len() - 3].trim_end() } else { before };
        let is_let = lead.ends_with("let") && !lead[..lead.len() - 3].ends_with(|c: char| is_id(c as u8));
        let is_param = lead.ends_with('(') || lead.ends_with(',');
        if !(is_let || is_param) {
            return None;
        }
        // The declared type (`&mut T`, `&'a mut T`), or for a `let` the start of its value (`= &mut v`).
        let holds_mut_ref = if let Some(ty) = after.strip_prefix(':') {
            let ty = ty[..ty.find([',', ')', '=', ';', '{']).unwrap_or(ty.len())].trim_start();
            ty.starts_with('&') && !word_at(ty, "mut").is_empty()
        } else if let Some(val) = after.strip_prefix('=') {
            val.trim_start().starts_with("&mut ")
        } else {
            false
        };
        Some(if holds_mut_ref {
            Some(MutRecv::Reborrow)
        } else if mut_kw && (is_let || after.starts_with(':')) {
            Some(MutRecv::Borrow)
        } else {
            None
        })
    })?;
    decl
}

#[cfg(test)]
mod tests {
    use super::{mut_receiver_binding, MutRecv};

    #[test]
    fn binds_a_receiver_by_its_declaration() {
        let src = "fn run(out: &mut Vec<u8>, mut buf: String, g: Gizmos) { let mut acc = Acc::new(); let r = &mut acc; X; }\n\
                   impl S { fn m(&mut self) { X; } fn n(&self) { X; } }";
        let call = src.find("X;").unwrap();
        assert_eq!(mut_receiver_binding(src, call, "out"), Some(MutRecv::Reborrow));
        assert_eq!(mut_receiver_binding(src, call, "buf"), Some(MutRecv::Borrow));
        assert_eq!(mut_receiver_binding(src, call, "acc"), Some(MutRecv::Borrow));
        assert_eq!(mut_receiver_binding(src, call, "r"), Some(MutRecv::Reborrow));
        assert_eq!(mut_receiver_binding(src, call, "g"), None, "neither `mut` nor a `&mut`");
        let in_m = src.match_indices("X;").nth(1).unwrap().0;
        assert_eq!(mut_receiver_binding(src, in_m, "self.commands"), Some(MutRecv::Borrow));
        let in_n = src.match_indices("X;").nth(2).unwrap().0;
        assert_eq!(mut_receiver_binding(src, in_n, "self.commands"), None, "a field of `&self`");
    }
}
