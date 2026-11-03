/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Crate-level lint, coherence, and scope-isolation checks for compile-safe code actions.

use super::action_parse::{is_ident_byte, mask_comments_and_strings};
use super::action_parse_decl::{find_header_terminator, next_ident_token};
use super::action_parse_items::{contains_word, skip_balanced};
use std::path::{Path, PathBuf};

pub(crate) fn crate_denies_missing_docs(masked_doc: &str, file_path: &str) -> bool {
    if masked_denies_missing_docs(masked_doc) {
        return true;
    }
    if file_path.is_empty() {
        return false;
    }
    if let Some(crate_dir) = find_enclosing_crate_dir(Path::new(file_path)) {
        for entry in ["src/lib.rs", "src/main.rs"] {
            if let Ok(raw) = std::fs::read_to_string(crate_dir.join(entry)) {
                if raw.contains("missing_docs") && masked_denies_missing_docs(&mask_comments_and_strings(&raw)) {
                    return true;
                }
            }
        }
        if let Ok(toml_raw) = std::fs::read_to_string(crate_dir.join("Cargo.toml")) {
            if toml_raw.lines().any(|l| l.contains("missing_docs") && (l.contains("deny") || l.contains("forbid"))) {
                return true;
            }
        }
    }
    false
}

fn masked_denies_missing_docs(masked: &str) -> bool {
    let mb = masked.as_bytes();
    let (mut warns_md, mut denies_warn) = (false, false);
    let mut i = 0usize;
    while i + 2 < mb.len() {
        if &mb[i..i + 2] == b"#!" {
            let mut k = i + 2;
            while k < mb.len() && mb[k].is_ascii_whitespace() {
                k += 1;
            }
            if let Some(end_a) = skip_balanced(mb, k, b'[', b']') {
                let inner = &masked[k + 1..end_a - 1];
                if (contains_word(inner, "deny") || contains_word(inner, "forbid")) && contains_word(inner, "missing_docs") {
                    return true;
                }
                if contains_word(inner, "warn") && contains_word(inner, "missing_docs") {
                    warns_md = true;
                }
                if (contains_word(inner, "deny") || contains_word(inner, "forbid")) && contains_word(inner, "warnings") {
                    denies_warn = true;
                }
                i = end_a;
                continue;
            }
        }
        i += 1;
    }
    warns_md && denies_warn
}

pub(crate) fn crate_already_implements_conversion_for(masked_doc: &str, file_path: &str, type_name: &str) -> bool {
    if doc_has_conversion_for(masked_doc, type_name) {
        return true;
    }
    if file_path.is_empty() {
        return false;
    }
    let Some(crate_dir) = find_enclosing_crate_dir(Path::new(file_path)) else {
        return false;
    };
    let mut stack = vec![crate_dir.join("src")];
    let mut visited = 0usize;
    let cur_path = Path::new(file_path);
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for entry in rd.flatten() {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().and_then(|e| e.to_str()) == Some("rs") && p != cur_path {
                visited += 1;
                if visited > 128 {
                    return false;
                }
                let Ok(raw) = std::fs::read_to_string(&p) else { continue };
                if !raw.contains(type_name) || !(raw.contains("From") || raw.contains("from")) {
                    continue;
                }
                let m = mask_comments_and_strings(&raw);
                if contains_word(&m, type_name) && doc_has_conversion_for(&m, type_name) {
                    return true;
                }
            }
        }
    }
    false
}

fn find_enclosing_crate_dir(start: &Path) -> Option<PathBuf> {
    let mut cur = start.parent()?;
    for _ in 0..6 {
        if cur.join("Cargo.toml").is_file() {
            return Some(cur.to_path_buf());
        }
        cur = cur.parent()?;
    }
    None
}

fn doc_has_conversion_for(masked: &str, type_name: &str) -> bool {
    if doc_already_implements_trait_for(masked, "From", type_name) || doc_already_implements_trait_for(masked, "TryFrom", type_name) {
        return true;
    }
    let file_defines_from_macro = masked.contains("macro_rules!") && (contains_word(masked, "From") || contains_word(masked, "TryFrom"));
    let mb = masked.as_bytes();
    let mut i = 0usize;
    while i < mb.len() {
        if mb[i] == b'!' && mb.get(i + 1).copied() != Some(b'=') {
            let prev = masked[..i].trim_end();
            let mac_name: String = prev.chars().rev().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect::<String>().chars().rev().collect();
            let mut k = i + 1;
            while k < mb.len() && mb[k].is_ascii_whitespace() {
                k += 1;
            }
            let end_m = match mb.get(k).copied() {
                Some(b'(') => skip_balanced(mb, k, b'(', b')'),
                Some(b'[') => skip_balanced(mb, k, b'[', b']'),
                Some(b'{') => skip_balanced(mb, k, b'{', b'}'),
                _ => None,
            };
            if let Some(em) = end_m {
                let lower = mac_name.to_ascii_lowercase();
                if (file_defines_from_macro || lower.contains("from") || lower.contains("newtype")) && contains_word(&masked[k..em], type_name) {
                    return true;
                }
                i = em;
                continue;
            }
        }
        i += 1;
    }
    false
}

pub(crate) fn doc_already_implements_trait_for(masked_doc: &str, trait_kw: &str, type_name: &str) -> bool {
    let mb = masked_doc.as_bytes();
    let mut i = 0usize;
    while i + 4 < mb.len() {
        if &mb[i..i + 4] == b"impl" && (i == 0 || !is_ident_byte(mb[i - 1])) && !is_ident_byte(mb[i + 4]) {
            if let Some(bo) = masked_doc[i..].find('{') {
                let hdr = &masked_doc[i..i + bo];
                if contains_word(hdr, trait_kw) && contains_word(hdr, "for") && contains_word(hdr, type_name) {
                    return true;
                }
            }
        }
        i += 1;
    }
    false
}

pub(crate) fn nested_fn_has_non_runtime_ref(after_masked: &str, name: &str) -> bool {
    let mb = after_masked.as_bytes();
    let mut i = 0usize;
    while i < mb.len() {
        if mb[i] == b'#' {
            let mut k = i + 1;
            if mb.get(k).copied() == Some(b'!') {
                k += 1;
            }
            while k < mb.len() && mb[k].is_ascii_whitespace() {
                k += 1;
            }
            if let Some(end_a) = skip_balanced(mb, k, b'[', b']') {
                if contains_word(&after_masked[k..end_a], name) {
                    return true;
                }
                i = end_a;
                continue;
            }
        }
        if mb[i] == b'!' && mb.get(i + 1).copied() != Some(b'=') {
            let mut k = i + 1;
            while k < mb.len() && mb[k].is_ascii_whitespace() {
                k += 1;
            }
            let end_m = match mb.get(k).copied() {
                Some(b'(') => skip_balanced(mb, k, b'(', b')'),
                Some(b'[') => skip_balanced(mb, k, b'[', b']'),
                Some(b'{') => skip_balanced(mb, k, b'{', b'}'),
                _ => None,
            };
            if let Some(em) = end_m {
                if contains_word(&after_masked[k..em], name) {
                    return true;
                }
                i = em;
                continue;
            }
        }
        if mb[i].is_ascii_alphabetic() && (i == 0 || !is_ident_byte(mb[i - 1])) {
            if let Some((kw, after_kw)) = next_ident_token(after_masked, i) {
                if matches!(kw, "fn" | "const" | "static" | "struct" | "enum" | "union" | "trait" | "impl" | "type" | "mod") {
                    if let Some(term) = find_header_terminator(mb, after_kw) {
                        let end_item = if mb[term] == b'{' { skip_balanced(mb, term, b'{', b'}').unwrap_or(term + 1) } else { term + 1 };
                        if contains_word(&after_masked[i..end_item], name) {
                            return true;
                        }
                        i = end_item;
                        continue;
                    }
                }
                i = after_kw;
                continue;
            }
        }
        i += 1;
    }
    false
}

pub(crate) fn is_leaf_scope_without_macros(scope_masked: &str, name: &str) -> bool {
    if contains_word(scope_masked, "include") {
        return false;
    }
    let mb = scope_masked.as_bytes();
    let mut i = 0usize;
    while i < mb.len() {
        if mb[i] == b'!' && mb.get(i + 1).copied() != Some(b'=') {
            let mut k = i + 1;
            while k < mb.len() && mb[k].is_ascii_whitespace() {
                k += 1;
            }
            let end_m = match mb.get(k).copied() {
                Some(b'(') => skip_balanced(mb, k, b'(', b')'),
                Some(b'[') => skip_balanced(mb, k, b'[', b']'),
                Some(b'{') => skip_balanced(mb, k, b'{', b'}'),
                _ => None,
            };
            if let Some(em) = end_m {
                if contains_word(&scope_masked[k..em], name) {
                    return false;
                }
                i = em;
                continue;
            }
        }
        if i + 3 < mb.len() && &mb[i..i + 3] == b"mod" && (i == 0 || !is_ident_byte(mb[i - 1])) && !is_ident_byte(mb[i + 3]) {
            if let Some((_, after_n)) = next_ident_token(scope_masked, i + 3) {
                if scope_masked[after_n..].trim_start().starts_with(';') {
                    return false;
                }
            }
        }
        i += 1;
    }
    true
}
