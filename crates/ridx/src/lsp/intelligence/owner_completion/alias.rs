/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! `#[doc(alias = …)]` names of a definition, which rust-analyzer shows in a completion label:
//! `take(alias limit) (as Iterator)`.

use ridx_format::ResolvedDefinition;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

thread_local! {
    static SOURCE_LINES: RefCell<HashMap<PathBuf, Option<Rc<Vec<String>>>>> = RefCell::new(HashMap::new());
}

/// The `(alias a, b)` note for `d`, read from the attributes above its declaration, or `None` without aliases.
pub fn alias_note(d: &ResolvedDefinition<'_>) -> Option<String> {
    let mut aliases = Vec::new();
    for t in attribute_lines(d)? {
        aliases.splice(0..0, aliases_in(&t));
    }
    (!aliases.is_empty()).then(|| format!("(alias {})", aliases.join(", ")))
}

/// Whether `d` is `#[doc(hidden)]` in a library (the standard library or a registry crate); rust-analyzer does not
/// complete such items outside their crate.
pub fn is_hidden_library_item(d: &ResolvedDefinition<'_>) -> bool {
    let path = crate::lsp::intelligence::hover_render::def_file(d);
    let p = path.to_string_lossy();
    (p.contains("/library/") || p.contains("/registry/src/")) && attribute_lines(d).is_some_and(|ls| ls.iter().any(|l| l.contains("doc(hidden)")))
}

/// Whether the declaration of a method or field `d` lacks a plain `pub` (`pub(crate) fn`, `fn`, `field: T`), so it
/// is not visible from another crate. Declarations that cannot be read, or whose line does not declare `d` (macro
/// output), count as visible.
pub fn is_crate_private(d: &ResolvedDefinition<'_>) -> bool {
    let Some(lines) = source_lines(d) else { return false };
    let Some(line) = (d.start_line as usize).checked_sub(1).and_then(|i| lines.get(i)) else { return false };
    let t = line.trim_start();
    let declares = t.contains(&format!("fn {}", d.name)) || t.split(':').next().is_some_and(|head| head.split_whitespace().last() == Some(d.name));
    declares && !(t.starts_with("pub ") || t.starts_with("pub\t"))
}

fn source_lines(d: &ResolvedDefinition<'_>) -> Option<Rc<Vec<String>>> {
    let path = crate::lsp::intelligence::hover_render::def_file(d);
    SOURCE_LINES.with(|cache| {
        cache.borrow_mut().entry(path.clone()).or_insert_with(|| std::fs::read_to_string(&path).ok().map(|t| Rc::new(t.lines().map(str::to_string).collect()))).clone()
    })
}

/// The attribute and doc-comment lines directly above `d`'s declaration, nearest first.
fn attribute_lines(d: &ResolvedDefinition<'_>) -> Option<Vec<String>> {
    let lines = source_lines(d)?;
    let decl = (d.start_line as usize).checked_sub(1)?;
    let mut out = Vec::new();
    for line in lines[..decl.min(lines.len())].iter().rev() {
        let t = line.trim();
        if !(t.starts_with("#[") || t.starts_with("///") || t.starts_with("//") || t.ends_with(")]") || t.ends_with(',')) {
            break;
        }
        out.push(t.to_string());
    }
    Some(out)
}

/// The alias names one attribute line declares: `#[doc(alias = "x")]` or `#[doc(alias("a", "b"))]`.
fn aliases_in(attr: &str) -> Vec<String> {
    let Some(at) = attr.find("alias") else { return Vec::new() };
    if !attr.contains("doc") {
        return Vec::new();
    }
    attr[at + "alias".len()..].split('"').skip(1).step_by(2).map(str::to_string).collect()
}

#[cfg(test)]
mod tests {
    use super::aliases_in;

    #[test]
    fn test_aliases_in_both_attribute_forms() {
        assert_eq!(aliases_in(r#"#[doc(alias = "limit")]"#), ["limit"]);
        assert_eq!(aliases_in(r#"#[doc(alias("inject", "foldl"))]"#), ["inject", "foldl"]);
        assert!(aliases_in("#[inline]").is_empty());
    }
}
