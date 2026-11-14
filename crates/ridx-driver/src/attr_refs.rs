/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! An attribute or derive macro's name where it is written (`#[tokio::test]`, `#[derive(Deserialize)]`) is a reference
//! to the macro. Expansion removes the attribute from the HIR; the code it produced still carries the call site and
//! the macro's definition in its expansion data.

use super::*;
use rustc_span::hygiene::{ExpnKind, MacroKind};
use rustc_span::{BytePos, Span};

impl<'tcx> HirVisitor<'tcx> {
    /// Records the macro named at the call site of the attribute or derive expansion that produced `span`. Macros of
    /// the standard library (`derive(Clone)`) are left out: their paths are the paths of their traits.
    pub(crate) fn note_macro_call_site(&mut self, span: Span) {
        if !span.from_expansion() {
            return;
        }
        let data = span.ctxt().outer_expn_data();
        if matches!(data.kind, ExpnKind::Macro(MacroKind::Bang, _)) {
            self.note_bang_call_site(data);
            return;
        }
        let (ExpnKind::Macro(MacroKind::Attr | MacroKind::Derive, _), Some(did)) = (data.kind, data.macro_def_id) else {
            return;
        };
        if matches!(self.tcx.crate_name(did.krate).as_str(), "core" | "std" | "alloc") || data.call_site.from_expansion() {
            return;
        }
        let site = data.call_site;
        let Ok(text) = self.tcx.sess.source_map().span_to_snippet(site) else { return };
        let Some((start, len)) = macro_name_in(&text) else { return };
        // The name written may be an alias (`use serde::Serialize as Ser`); only the macro's own name is matched.
        if self.tcx.opt_item_name(did).is_none_or(|n| n.as_str() != &text[start..start + len]) {
            return;
        }
        let lo = site.lo() + BytePos(start as u32);
        self.add_ref_from_res(site.with_lo(lo).with_hi(lo + BytePos(len as u32)), Res::Def(self.tcx.def_kind(did), did));
    }

    /// Records the function-like macro written in the source (`tracing::info_span!(..)`, `pin!(x)`) whose expansion
    /// produced the code. Macros the expansion itself calls (`info_span!` → `span!`) are not written there: the chain
    /// of call sites is followed out to the first one in the source. Standard-library macros are left out.
    fn note_bang_call_site(&mut self, mut data: rustc_span::hygiene::ExpnData) {
        while data.call_site.from_expansion() {
            data = data.call_site.ctxt().outer_expn_data();
        }
        let (ExpnKind::Macro(MacroKind::Bang, _), Some(did)) = (data.kind, data.macro_def_id) else {
            return;
        };
        if matches!(self.tcx.crate_name(did.krate).as_str(), "core" | "std" | "alloc") {
            return;
        }
        let site = data.call_site;
        let Ok(text) = self.tcx.sess.source_map().span_to_snippet(site) else { return };
        let Some((start, len)) = bang_macro_name_in(&text) else { return };
        if self.tcx.opt_item_name(did).is_none_or(|n| n.as_str() != &text[start..start + len]) {
            return;
        }
        let lo = site.lo() + BytePos(start as u32);
        self.add_ref_from_res(site.with_lo(lo).with_hi(lo + BytePos(len as u32)), Res::Def(self.tcx.def_kind(did), did));
    }
}

/// Byte offset and length of the macro's name in the text of a function-like macro call
/// (`tracing::info_span!("x")` → `info_span`, `pin! (fut)` → `pin`).
pub(crate) fn bang_macro_name_in(text: &str) -> Option<(usize, usize)> {
    let path = text[..text.find('!')?].trim_end();
    let name_start = path.rfind("::").map_or(0, |p| p + 2);
    let name = path[name_start..].strip_prefix("r#").unwrap_or(&path[name_start..]);
    let name_start = path.len() - name.len();
    (!name.is_empty() && name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')).then_some((name_start, name.len()))
}

/// Byte offset and length of the macro's name in the text of an attribute or derive path
/// (`#[tokio::test(flavor = "x")]` → `test`, `serde::Deserialize` → `Deserialize`).
pub(crate) fn macro_name_in(text: &str) -> Option<(usize, usize)> {
    let body_start = if text.starts_with("#[") { 2 } else { 0 };
    let body = &text[body_start..];
    let end = body.find(|c: char| c == '(' || c == ']' || c == '=' || c.is_whitespace()).unwrap_or(body.len());
    let path = &body[..end];
    let name_start = path.rfind("::").map_or(0, |p| p + 2);
    let name = &path[name_start..];
    (!name.is_empty() && name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')).then_some((body_start + name_start, name.len()))
}

#[cfg(test)]
mod tests {
    use super::{bang_macro_name_in, macro_name_in};

    #[test]
    fn finds_the_macro_name_in_function_like_calls() {
        assert_eq!(bang_macro_name_in("tracing::info_span!(\"x\")"), Some((9, 9)));
        assert_eq!(bang_macro_name_in("pin! (fut)"), Some((0, 3)));
        assert_eq!(bang_macro_name_in("snapbox::file![\"a.rs\"]"), Some((9, 4)));
        assert_eq!(bang_macro_name_in("no_bang(x)"), None);
    }

    #[test]
    fn finds_the_macro_name_in_attribute_and_derive_paths() {
        assert_eq!(macro_name_in("#[tokio::test]"), Some((9, 4)));
        assert_eq!(macro_name_in("#[tokio::test(flavor = \"multi_thread\")]"), Some((9, 4)));
        assert_eq!(macro_name_in("Deserialize"), Some((0, 11)));
        assert_eq!(macro_name_in("serde::Serialize"), Some((7, 9)));
    }
}
