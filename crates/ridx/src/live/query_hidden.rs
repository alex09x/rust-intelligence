/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! `#[doc(hidden)]` on items of other crates: rust-analyzer does not complete them outside the crate that defines
//! them. The index records no such flag, so the attribute lines above the definition are read once per definition.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Per-definition answers plus the text of the file read last (a trait's methods share one file).
#[derive(Default)]
pub(crate) struct HiddenCache {
    answers: RefCell<HashMap<(PathBuf, u32), bool>>,
    last_file: RefCell<Option<(PathBuf, Arc<str>)>>,
}

impl HiddenCache {
    /// Whether the item declared at 1-based `line` of `file` carries `#[doc(hidden)]` among the attribute and doc
    /// lines directly above it.
    pub(crate) fn is_doc_hidden(&self, file: &Path, line: u32) -> bool {
        if let Some(&hit) = self.answers.borrow().get(&(file.to_path_buf(), line)) {
            return hit;
        }
        let text = {
            let mut last = self.last_file.borrow_mut();
            match last.as_ref().filter(|(p, _)| p == file) {
                Some((_, t)) => Arc::clone(t),
                None => {
                    let Ok(t) = std::fs::read_to_string(file) else { return false };
                    let t: Arc<str> = t.into();
                    *last = Some((file.to_path_buf(), Arc::clone(&t)));
                    t
                }
            }
        };
        let hidden = attrs_above_have_doc_hidden(&text, line);
        self.answers.borrow_mut().insert((file.to_path_buf(), line), hidden);
        hidden
    }
}

fn attrs_above_have_doc_hidden(text: &str, line: u32) -> bool {
    let decl = line.saturating_sub(1) as usize;
    let lines: Vec<&str> = text.lines().take(decl).collect();
    // The attribute and doc lines of the item run up to the end of the previous item, a blank line or the opening
    // brace of the enclosing block; multi-line attributes are spanned whole.
    for l in lines.iter().rev() {
        let t = l.trim();
        if t.is_empty() || t.ends_with('}') || t.ends_with(';') || t.ends_with('{') {
            return false;
        }
        if t.contains("doc(hidden)") {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::attrs_above_have_doc_hidden;

    #[test]
    fn test_doc_hidden_is_read_from_the_attribute_block_above() {
        let src = "impl X {\n    #[doc(hidden)]\n    #[cfg_attr(\n        feature = \"d\",\n        deprecated(since = \"4\")\n    )]\n    pub fn old(&self) {}\n\n    /// Docs.\n    pub fn new(&self) {}\n}\n";
        assert!(attrs_above_have_doc_hidden(src, 7));
        assert!(!attrs_above_have_doc_hidden(src, 10));
    }
}
