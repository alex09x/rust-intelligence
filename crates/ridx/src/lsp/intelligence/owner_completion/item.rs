/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Completion items in rust-analyzer's presentation: a trait method carries `(as Trait)`, a postfix snippet whose
//! type is not imported carries `(use path)`. The note goes to `labelDetails` when the client supports it and into
//! the label otherwise, so the label a client shows is the one rust-analyzer would show.

use serde_json::{json, Value};
use std::cell::Cell;

thread_local! {
    // The server loop answers every request on the thread that handled `initialize`.
    static LABEL_DETAILS: Cell<bool> = const { Cell::new(true) };
}

/// Records whether the client declared `completionItem.labelDetailsSupport` at initialize.
pub fn set_label_details_support(capabilities: &Value) {
    let on = capabilities["textDocument"]["completion"]["completionItem"]["labelDetailsSupport"].as_bool().unwrap_or(false);
    LABEL_DETAILS.with(|c| c.set(on));
}

/// A completion item named `name`, with `note` shown after it the way rust-analyzer shows it.
pub fn item(name: &str, note: Option<&str>, kind: u32, detail: &str) -> Value {
    let mut it = json!({ "label": name, "kind": kind, "detail": detail });
    if let Some(note) = note {
        if LABEL_DETAILS.with(Cell::get) {
            it["labelDetails"] = json!({ "detail": note });
        } else {
            it["label"] = json!(format!("{name}{note}"));
        }
        it["filterText"] = json!(name);
        it["insertText"] = json!(name);
    }
    it
}

/// An `iter().name` item: filtered by the method name, inserting the call through `iter()`.
pub fn through_iter_item(name: &str, note: &str, kind: u32, detail: &str) -> Value {
    let mut it = item(&format!("iter().{name}"), Some(note), kind, detail);
    it["filterText"] = json!(name);
    it
}

/// What deduplicates an item: its label with its note and kind, so a method found through several owners appears
/// once while the `deref` snippet and the `deref(as Deref)` method both stay.
pub fn item_key(item: &Value) -> String {
    format!("{}{}#{}", item["label"].as_str().unwrap_or_default(), item["labelDetails"]["detail"].as_str().unwrap_or_default(), item["kind"])
}

/// Whether `trait_path` names `Iterator`.
pub fn is_iterator_trait(trait_path: &str) -> bool {
    super::base_trait_path(trait_path).rsplit("::").next() == Some("Iterator")
}

/// Whether a method signature takes nothing but its receiver (`fn(&self) -> Iter<'_, T>`).
pub fn takes_only_receiver(sig: &str) -> bool {
    let Some(open) = sig.find('(') else { return false };
    let mut depth = 0i32;
    for c in sig[open + 1..].chars() {
        match c {
            '(' | '<' | '[' => depth += 1,
            ')' if depth == 0 => return true,
            ')' | '>' | ']' => depth -= 1,
            ',' if depth == 0 => return false,
            _ => {}
        }
    }
    false
}

/// The `(as Trait)` note for a method a trait provides, with the trait's generics dropped.
pub fn trait_note(trait_path: &str) -> String {
    format!("(as {})", super::trait_short_name(trait_path))
}

/// The note rust-analyzer shows after a definition's name: its doc aliases, then the trait providing it
/// (`(alias limit) (as Iterator)`).
pub fn def_note(d: &ridx_format::ResolvedDefinition<'_>, trait_path: Option<&str>) -> Option<String> {
    match (super::alias::alias_note(d), trait_path.map(trait_note)) {
        (Some(a), Some(t)) => Some(format!("{a} {t}")),
        (a, t) => a.or(t),
    }
}

/// The trait a method definition belongs to: the trait of an impl item (`<T as Trait>::m`) or the parent of a
/// trait's own item (`core::clone::Clone::clone`).
pub fn trait_of_def(def_path: &str) -> Option<&str> {
    super::extract_trait_from_def_path(def_path).or_else(|| def_path.rsplit_once("::").map(|(p, _)| p))
}

/// Gives every item a `sortText` in the order the list was built (fields, inherent, traits, snippets), so clients
/// keep the relevance order instead of sorting alphabetically.
pub fn number_sort_text(items: &mut [Value]) {
    for (i, it) in items.iter_mut().enumerate() {
        it["sortText"] = json!(format!("{i:04}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trait_methods_carry_the_trait_in_label_without_label_details() {
        set_label_details_support(&json!({}));
        let it = item("clone", Some(&trait_note("core::clone::Clone")), 2, "fn(&self) -> Self");
        assert_eq!(it["label"], "clone(as Clone)");
        assert_eq!(it["filterText"], "clone");
        set_label_details_support(&json!({ "textDocument": { "completion": { "completionItem": { "labelDetailsSupport": true } } } }));
        let it = item("into", Some(&trait_note("core::convert::Into<T>")), 2, "fn(self) -> T");
        assert_eq!(it["label"], "into");
        assert_eq!(it["labelDetails"]["detail"], "(as Into)");
    }

    #[test]
    fn test_helpers() {
        assert!(takes_only_receiver("fn(&self) -> Iter<'_, T>"));
        assert!(!takes_only_receiver("fn(&self, n: usize) -> Option<&T>"));
        assert!(is_iterator_trait("core::iter::traits::iterator::Iterator"));
        assert_ne!(item_key(&item("deref", None, 15, "*expr")), item_key(&item("deref", Some("(as Deref)"), 2, "")));
    }
}
