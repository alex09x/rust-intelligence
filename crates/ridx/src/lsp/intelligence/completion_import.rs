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

use crate::lsp::editing::public_path::public_path;
use crate::lsp::navigation::type_definition_parse::{on_disk, sysroot_rust_src};
use ridx_format::{DefKind, WorkspaceSymbol};
use std::collections::HashMap;
use std::path::PathBuf;

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
pub(super) fn import_path(s: &WorkspaceSymbol<'_>, current_crate: Option<&str>, roots: &mut HashMap<String, Option<String>>) -> Option<String> {
    let first = s.path.split("::").next().unwrap_or("");
    let full = if first == s.crate_name || matches!(first, "std" | "core" | "alloc") { s.path.to_string() } else { format!("{}::{}", s.crate_name, s.path) };
    let (krate, rest) = full.split_once("::")?;
    if Some(krate) == current_crate {
        return declared_pub(s, true).then(|| format!("crate::{rest}"));
    }
    if let Some(found) = roots.get(&full) {
        return found.clone();
    }
    let found = match krate {
        "std" | "core" | "alloc" => {
            let root = sysroot_rust_src()?.join("library").join(krate).join("src/lib.rs");
            // `std` re-exports the public modules of `core` and `alloc` under its own name.
            public_path(&root, &full).map(|p| p.replacen(&format!("{krate}::"), "std::", 1))
        }
        _ => {
            // The crate root above the defining file: the nearest directory with a `Cargo.toml` and a `src/lib.rs`.
            let def_file = defining_file(s);
            let root = def_file.ancestors().skip(1).find(|d| d.join("Cargo.toml").is_file()).map(|d| d.join("src/lib.rs")).filter(|r| r.is_file())?;
            public_path(&root, &full).filter(|_| declared_pub(s, false))
        }
    };
    roots.insert(full, found.clone());
    found
}

/// The file `s` is declared in, on disk (a crate's source root and its file path overlap: `ws/native` + `native/src/lib.rs`).
fn defining_file(s: &WorkspaceSymbol<'_>) -> PathBuf {
    on_disk(s.resolved_path())
}

/// Whether `s` is declared `pub` (`pub(crate)` too when `in_crate`): the text before its name on its line.
fn declared_pub(s: &WorkspaceSymbol<'_>, in_crate: bool) -> bool {
    let file = defining_file(s);
    let Some(line) = std::fs::read_to_string(file).ok().and_then(|t| t.lines().nth(s.span.start_line().saturating_sub(1) as usize).map(str::to_string)) else {
        return false;
    };
    let head = line.split(s.name).next().unwrap_or("").trim_start();
    head.starts_with("pub ") || head.starts_with("pub(crate)") && in_crate
}

/// Whether the file's `use` items already bring a name `name` into scope, by whatever path: importing another
/// `name` would clash with it (E0252).
pub(super) fn name_imported(doc: &str, name: &str) -> bool {
    let mut in_use = false;
    for line in doc.lines() {
        let t = line.trim_start();
        // The file's own level only: a `use` inside `mod tests { … }` or a function brings nothing in elsewhere.
        if line.starts_with("use ") || line.starts_with("pub use ") || line.starts_with("pub(crate) use ") {
            in_use = true;
        }
        if in_use {
            let words: Vec<&str> = t.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == ':')).collect();
            if words.iter().any(|w| w.rsplit("::").next() == Some(name) && !w.ends_with("::")) {
                return true;
            }
            if t.contains(';') {
                in_use = false;
            }
        }
    }
    false
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
    }
}
