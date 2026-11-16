/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Import scanner and normalization for dirty buffer dependency tracking.

use std::collections::HashSet;

/// What the `use` items and file module declarations of `text` bring in (`mod tests { … }` bodies are not imports):
/// every `(name, path)`, and the paths of its globs (`super` for `use super::*;`), whose names cannot be listed.
pub fn imported_names(text: &str) -> (HashSet<(String, String)>, HashSet<String>) {
    use crate::lsp::intelligence::{completion_import::inline_mod_ranges, use_tree};
    let (mut names, mut globs) = (HashSet::new(), HashSet::new());
    // The file's own level and that of each inline module (`mod tests { use super::*; … }`), the module's ordinal kept
    // in the path so that the same import in two modules counts twice (an ordinal, unlike an offset, survives edits).
    let levels = std::iter::once((0, text.len())).chain(inline_mod_ranges(text).into_iter().map(|(s, e)| (s + 1, e)));
    for (ordinal, level) in levels.enumerate() {
        for (_, tree) in use_tree::level_uses(text, level) {
            for leaf in use_tree::use_leaves(&tree) {
                let path = format!("{}@{ordinal}", leaf.path);
                match leaf.name {
                    _ if leaf.glob => {
                        globs.insert(path);
                    }
                    Some(name) => {
                        names.insert((name, path));
                    }
                    None => {}
                }
            }
        }
    }
    // File modules (`mod util;`) bring their names in too.
    for line in text.lines() {
        let t = line.trim();
        let t = t.strip_prefix("pub ").or_else(|| t.strip_prefix("pub(crate) ")).unwrap_or(t);
        if let Some(m) = t.strip_prefix("mod ").and_then(|r| r.strip_suffix(';')) {
            names.insert((m.trim().to_string(), format!("self::{}", m.trim())));
        }
    }
    (names, globs)
}

/// Extracts and normalizes import statements from Rust source text,
/// safely respecting UTF-8 character boundaries and comment structures.
pub fn extract_normalized_imports(text: &str) -> Vec<String> {
    let mut imports = Vec::new();
    let bytes = text.as_bytes();
    let n = bytes.len();
    let mut i = 0;

    while i < n {
        let b = bytes[i];

        if b == b'/' && i + 1 < n && bytes[i + 1] == b'/' {
            i += 2;
            while i < n && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }

        if b == b'/' && i + 1 < n && bytes[i + 1] == b'*' {
            i += 2;
            let mut depth = 1;
            while i + 1 < n && depth > 0 {
                if bytes[i] == b'/' && bytes[i + 1] == b'*' {
                    depth += 1;
                    i += 2;
                } else if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            if depth > 0 {
                i = n;
            }
            continue;
        }

        if b == b'"' {
            i += 1;
            let mut closed = false;
            while i < n {
                if bytes[i] == b'\\' && i + 1 < n {
                    i += 2;
                    continue;
                }
                if bytes[i] == b'"' {
                    i += 1;
                    closed = true;
                    break;
                }
                i += 1;
            }
            if !closed {
                i = n;
            }
            continue;
        }

        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }

        while i < n && !text.is_char_boundary(i) {
            i += 1;
        }
        if i >= n {
            break;
        }

        let rem = &text[i..];
        let is_import_start = rem.starts_with("use ")
            || rem.starts_with("pub use ")
            || (rem.starts_with("pub(") && rem.contains(") use "))
            || rem.starts_with("mod ")
            || rem.starts_with("pub mod ")
            || rem.starts_with("extern crate ");

        if is_import_start {
            let start = i;
            let mut end = i;
            let mut depth: usize = 0;
            while end < n {
                let eb = bytes[end];
                if eb == b'{' {
                    depth += 1;
                    end += 1;
                } else if eb == b'}' {
                    depth = depth.saturating_sub(1);
                    end += 1;
                } else if eb == b';' && depth == 0 {
                    end += 1;
                    break;
                } else if let Some(ch) = text[end..].chars().next() {
                    end += ch.len_utf8();
                } else {
                    end = n;
                    break;
                }
            }
            while end < n && !text.is_char_boundary(end) {
                end += 1;
            }
            let raw_stmt = &text[start..end];
            let normalized = raw_stmt.split_whitespace().collect::<Vec<_>>().join(" ");
            imports.push(normalized);
            i = end;
            continue;
        }

        if let Some(ch) = text[i..].chars().next() {
            i += ch.len_utf8();
        } else {
            break;
        }
    }

    imports
}
