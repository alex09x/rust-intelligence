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

/// What `text` brings into scope by name: every `(name, path)` of its `use` items at any depth (a function's own
/// `use` too), `extern crate` and `mod name;` declarations, and the paths of its globs (`super` for `use super::*;`),
/// whose names cannot be listed. An anonymous import (`use a::Trait as _;`) is named `_`: it changes what methods
/// resolve to. Each path carries the block it is declared in, keyed by the text of the line opening that block
/// (`mod tests {`, `fn f() {`): the same import in two blocks counts twice, and an edit elsewhere moves no key.
/// Comments and strings name nothing.
pub fn imported_names(text: &str) -> (HashSet<(String, String)>, HashSet<String>) {
    use crate::lsp::intelligence::use_tree;
    let masked = crate::lsp::editing::action_parse::mask_comments_and_strings(text);
    let b = masked.as_bytes();
    let (mut names, mut globs) = (HashSet::new(), HashSet::new());
    let mut blocks: Vec<String> = Vec::new();
    let word_at = |p: usize, w: &str| {
        masked[p..].starts_with(w) && (p == 0 || !(b[p - 1].is_ascii_alphanumeric() || b[p - 1] == b'_')) && b.get(p + w.len()).is_some_and(u8::is_ascii_whitespace)
    };
    let mut p = 0;
    while p < b.len() {
        match b[p] {
            b'{' => {
                let line_start = masked[..p].rfind('\n').map_or(0, |i| i + 1);
                blocks.push(masked[line_start..p].trim().to_string());
            }
            b'}' => {
                blocks.pop();
            }
            _ => {}
        }
        let statement_end = || masked[p..].find(';').map(|i| p + i);
        if word_at(p, "use") {
            let block = blocks.join("/");
            if let Some(end) = statement_end() {
                for leaf in use_tree::use_leaves(&masked[p + 3..end]) {
                    let path = format!("{}@{block}", leaf.path);
                    if leaf.glob {
                        globs.insert(path);
                    } else {
                        names.insert((leaf.name.unwrap_or_else(|| "_".to_string()), path));
                    }
                }
                p = end;
                continue;
            }
        } else if word_at(p, "extern") && masked[p + 6..].trim_start().starts_with("crate") {
            let block = blocks.join("/");
            if let Some(end) = statement_end() {
                let decl: Vec<&str> = masked[p..end].split_whitespace().collect();
                let name = if decl.len() >= 5 && decl[3] == "as" { decl[4] } else { decl.get(2).copied().unwrap_or_default() };
                names.insert((name.to_string(), format!("extern crate {}@{block}", decl.get(2).copied().unwrap_or_default())));
                p = end;
                continue;
            }
        } else if word_at(p, "mod") {
            let rest = &masked[p + 3..];
            // `#` too: `mod r#type;` declares a module named `r#type`.
            let name_len = rest.trim_start().find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '#')).unwrap_or(0);
            let name = rest.trim_start()[..name_len].to_string();
            if rest.trim_start()[name_len..].trim_start().starts_with(';') {
                names.insert((name.clone(), format!("self::{name}@{}", blocks.join("/"))));
            }
        }
        p += masked[p..].chars().next().map_or(1, char::len_utf8);
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
