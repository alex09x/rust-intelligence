/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Which workspace symbol a name completion offers with an import, and by which path. Only what the `use` it writes
//! surely compiles: an item `use` can name (no impl block, associated item, field or import of another module), by a
//! public path (`std::collections::HashSet`, not the definition path through the private `hash::set`; `ahash::AHashSet`,
//! with its crate), declared `pub` unless it is a std item, and no name the file already imports from elsewhere.

use crate::lsp::editing::public_path::{crate_path, public_path};
use crate::lsp::navigation::type_definition_parse::{on_disk, sysroot_rust_src};
use ridx_format::{DefKind, WorkspaceSymbol};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Kinds of items a `use` can bring in by name.
pub(super) fn importable(s: &WorkspaceSymbol<'_>) -> bool {
    matches!(
        s.kind,
        DefKind::Struct | DefKind::Enum | DefKind::Union | DefKind::Trait | DefKind::TyAlias | DefKind::Fn | DefKind::Const | DefKind::Static | DefKind::Macro | DefKind::Mod
    ) && !s.name.is_empty()
        && s.name.chars().all(|c| c.is_alphanumeric() || c == '_')
        && !s.name.starts_with(|c: char| c.is_ascii_digit())
}

/// The path a `use` of `s` writes from a file of crate `current_crate`; `None` when no path is known to reach it.
/// `roots` caches the public path found per definition path.
pub(super) fn import_path(s: &WorkspaceSymbol<'_>, current_crate: Option<&str>, current_file: &str, roots: &mut HashMap<String, Option<String>>) -> Option<String> {
    let first = s.path.split("::").next().unwrap_or("");
    let full = if first == s.crate_name || matches!(first, "std" | "core" | "alloc") { s.path.to_string() } else { format!("{}::{}", s.crate_name, s.path) };
    let (krate, _) = full.split_once("::")?;
    let in_crate = Some(krate) == current_crate;
    let key = format!("{in_crate}:{full}");
    if let Some(found) = roots.get(&key) {
        return found.clone();
    }
    let found = match krate {
        "std" | "core" | "alloc" if !in_crate => std_path(krate, &full, current_file),
        _ => {
            let vis = visibility(s);
            let root = crate_root(&defining_file(s));
            // An item of the crate root is reached by `krate::item` with no module on the way to read.
            let via_root = |find: fn(&Path, &str) -> Option<String>| match &root {
                Some(r) => find(r, &full),
                None => (full.matches("::").count() == 1).then(|| full.clone()),
            };
            let path = match vis {
                // `#[macro_export]` puts a `macro_rules!` at the crate root, whatever module defines it.
                Vis::ExportedMacro => Some(format!("{krate}::{}", s.name)),
                Vis::Pub if in_crate => via_root(crate_path),
                Vis::Pub => via_root(public_path),
                Vis::Crate if in_crate => via_root(crate_path),
                _ => None,
            };
            path.map(|p| if in_crate { p.replacen(&format!("{krate}::"), "crate::", 1) } else { p })
        }
    };
    roots.insert(key, found.clone());
    found
}

/// The path a `use` of a std, core or alloc item `full` writes from `current_file`: by `std::` where std re-exports
/// the module (`core::cmp` is `std::cmp`), else by the crate that defines it; in a `#![no_std]` crate, never `std::`.
fn std_path(krate: &str, full: &str, current_file: &str) -> Option<String> {
    let library = sysroot_rust_src()?.join("library");
    let path = public_path(&library.join(krate).join("src/lib.rs"), full)?;
    if krate == "std" || is_no_std(Path::new(current_file)) {
        return Some(path);
    }
    let module = path.split("::").nth(1)?;
    let std_lib = std::fs::read_to_string(library.join("std/src/lib.rs")).ok()?;
    let reexported = super::use_tree::level_uses(&std_lib, (0, std_lib.len()))
        .iter()
        .any(|(_, tree)| super::use_tree::use_leaves(tree).iter().any(|l| l.path == format!("core::{module}") || l.path == format!("alloc_crate::{module}")));
    Some(if reexported { path.replacen(&format!("{krate}::"), "std::", 1) } else { path })
}

/// Whether the crate of `file` is `#![no_std]`.
fn is_no_std(file: &Path) -> bool {
    crate_root(file).and_then(|r| std::fs::read_to_string(r).ok()).is_some_and(|t| t.contains("#![no_std]"))
}

/// The library root of the package above `file`: its `Cargo.toml`'s `[lib] path`, else `src/lib.rs`, else `src/main.rs`.
fn crate_root(file: &Path) -> Option<PathBuf> {
    let dir = file.ancestors().skip(1).find(|d| d.join("Cargo.toml").is_file())?;
    let manifest = std::fs::read_to_string(dir.join("Cargo.toml")).unwrap_or_default();
    let lib_path = manifest.split("[lib]").nth(1).and_then(|sec| {
        let sec = sec.split("\n[").next().unwrap_or(sec);
        sec.lines().find_map(|l| l.trim().strip_prefix("path").map(str::trim).and_then(|v| v.strip_prefix('=')).map(|v| v.trim().trim_matches('"').to_string()))
    });
    [lib_path.map(|p| dir.join(p)), Some(dir.join("src/lib.rs")), Some(dir.join("src/main.rs"))].into_iter().flatten().find(|p| p.is_file())
}

/// The file `s` is declared in, on disk (a crate's source root and its file path overlap: `ws/native` + `native/src/lib.rs`).
fn defining_file(s: &WorkspaceSymbol<'_>) -> PathBuf {
    on_disk(s.resolved_path())
}

#[derive(Debug, PartialEq, Eq)]
enum Vis {
    Pub,
    /// `pub(crate)`, `pub(super)`, `pub(in …)`.
    Crate,
    ExportedMacro,
    Private,
}

/// How `s` is declared: the text before its name, attributes on its line or the lines above (`#[inline] pub fn`,
/// `#[macro_export]`) and a `pub` alone on the line before (`pub\nfn`) taken into account.
fn visibility(s: &WorkspaceSymbol<'_>) -> Vis {
    let Ok(text) = std::fs::read_to_string(defining_file(s)) else { return Vis::Private };
    let lines: Vec<&str> = text.lines().collect();
    let at = s.span.start_line().saturating_sub(1) as usize;
    let Some(line) = lines.get(at) else { return Vis::Private };
    let mut head = line.split(s.name).next().unwrap_or("").trim_start();
    while let Some(rest) = head.strip_prefix("#[") {
        head = rest.split_once(']').map_or("", |(_, after)| after).trim_start();
    }
    let attrs_above = lines[..at].iter().rev().map(|l| l.trim()).take_while(|l| l.starts_with("#[") || l.starts_with("///") || *l == "pub").collect::<Vec<_>>();
    if head.starts_with("macro_rules!") {
        return if attrs_above.iter().any(|l| l.starts_with("#[macro_export")) { Vis::ExportedMacro } else { Vis::Private };
    }
    if head.starts_with("pub(") {
        Vis::Crate
    } else if head.starts_with("pub ") || attrs_above.first() == Some(&"pub") {
        Vis::Pub
    } else {
        Vis::Private
    }
}

/// Whether the file's own `use` items already bring a name `name` into scope, by whatever path: importing another
/// `name` would clash with it (E0252). A `use` inside `mod tests { … }` or a function brings nothing in elsewhere.
pub(crate) fn name_imported(doc: &str, name: &str) -> bool {
    super::use_tree::level_imports(doc, (0, doc.len()), name)
}

/// Whether the module level at `cursor` already sees an imported `name`: its own `use`s, and the file's through a
/// `use super::*;` of an inline module.
pub(super) fn name_imported_at(doc: &str, cursor: usize, name: &str) -> bool {
    let level = super::use_tree::level_at(doc, cursor);
    super::use_tree::level_imports(doc, level, name) || level.0 > 0 && super::use_tree::level_globs_super(doc, level) && name_imported(doc, name)
}

/// The byte ranges of the inline modules of `text` (`mod tests { … }`, nested ones too), braces included: what is
/// declared in one is seen inside it only (a `use` of `mod tests` brings nothing into the functions above it).
pub(super) fn inline_mod_ranges(text: &str) -> Vec<(usize, usize)> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    for (p, _) in text.match_indices("mod ") {
        if p > 0 && (b[p - 1].is_ascii_alphanumeric() || b[p - 1] == b'_') {
            continue;
        }
        let line_start = text[..p].rfind('\n').map_or(0, |i| i + 1);
        if text[line_start..p].contains("//") {
            continue;
        }
        let rest = &text[p + 4..];
        let name_len = rest.find(|c: char| !(c.is_alphanumeric() || c == '_')).unwrap_or(rest.len());
        let after = &rest[name_len..];
        if name_len == 0 || !after.trim_start().starts_with('{') {
            continue;
        }
        let open = p + 4 + name_len + after.find('{').unwrap_or(0);
        let mut depth = 0usize;
        for (i, c) in text[open..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        out.push((open, open + i));
                        break;
                    }
                }
                _ => {}
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_inline_module_ranges() {
        let text = "use a::B;\nfn f() {}\n#[cfg(test)]\nmod tests {\n    use crate::x::Flat;\n    mod inner { fn g() {} }\n}\n// mod commented {\nmod file_mod;\n";
        let ranges = inline_mod_ranges(text);
        assert_eq!(ranges.len(), 2, "{ranges:?}");
        let flat = text.find("Flat").unwrap();
        assert!(ranges.iter().any(|&(s, e)| s < flat && flat < e));
        assert!(!ranges.iter().any(|&(s, e)| s < text.find("fn f").unwrap() && text.find("fn f").unwrap() < e));
    }

    #[test]
    fn test_a_name_imported_by_any_path_is_in_scope() {
        let doc = "use hashbrown::HashMap;\nuse std::{fmt, collections::{BTreeMap as Tree, VecDeque}};\nfn f() {}\nmod tests {\n    use std::collections::HashSet;\n}\n";
        assert!(name_imported(doc, "HashMap"));
        assert!(name_imported(doc, "VecDeque"));
        assert!(name_imported(doc, "Tree"), "an alias");
        assert!(!name_imported(doc, "HashSet"), "imported inside `mod tests` only");
        assert!(!name_imported(doc, "collections"), "a path segment is no import of that name");
        assert!(!name_imported(doc, "f"), "a function of the file is no import");
        assert!(!name_imported("use a::Foo as Bar;\n", "Foo"), "an alias brings in the alias only");
        assert!(name_imported("#[cfg(test)]\nuse a::Foo;\n", "Foo"));
        let tests = "use std::collections::HashSet;\nmod tests {\n    use super::*;\n    use crate::x::Flat;\n    fn t() {}\n}\n";
        let at = tests.find("fn t").unwrap();
        assert!(name_imported_at(tests, at, "Flat"));
        assert!(name_imported_at(tests, at, "HashSet"), "the file's import, through `use super::*`");
        assert!(!name_imported_at(tests, 0, "Flat"));
    }
}
