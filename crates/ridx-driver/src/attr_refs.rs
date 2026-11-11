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
    use super::macro_name_in;

    #[test]
    fn finds_the_macro_name_in_attribute_and_derive_paths() {
        assert_eq!(macro_name_in("#[tokio::test]"), Some((9, 4)));
        assert_eq!(macro_name_in("#[tokio::test(flavor = \"multi_thread\")]"), Some((9, 4)));
        assert_eq!(macro_name_in("Deserialize"), Some((0, 11)));
        assert_eq!(macro_name_in("serde::Serialize"), Some((7, 9)));
    }
}
