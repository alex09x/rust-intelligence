/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Auto-import intelligence: computes `use` insertion edits for external and workspace symbols.

use serde_json::{json, Value};

/// Standard Rust prelude full paths and primitive types that do not require explicit `use`.
const RUST_PRELUDE_PATHS: &[&str] = &[
    "core::option::Option",
    "std::option::Option",
    "core::result::Result",
    "std::result::Result",
    "core::result::Result::Ok",
    "core::result::Result::Err",
    "alloc::string::String",
    "std::string::String",
    "alloc::vec::Vec",
    "std::vec::Vec",
    "alloc::boxed::Box",
    "std::boxed::Box",
    "core::clone::Clone",
    "core::marker::Copy",
    "core::marker::Send",
    "core::marker::Sync",
    "core::marker::Sized",
    "core::default::Default",
    "core::convert::Into",
    "core::convert::From",
    "core::convert::TryInto",
    "core::convert::TryFrom",
    "core::convert::AsRef",
    "core::convert::AsMut",
    "core::iter::Iterator",
    "core::iter::IntoIterator",
    "core::iter::FromIterator",
    "core::cmp::PartialEq",
    "core::cmp::PartialOrd",
    "core::cmp::Eq",
    "core::cmp::Ord",
    "core::ops::Drop",
];

const PRIMITIVE_TYPES: &[&str] = &[
    "bool", "char", "str", "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64",
    "i128", "isize", "f32", "f64",
];

pub fn is_prelude_symbol(symbol_path: &str) -> bool {
    if !symbol_path.contains("::") {
        return PRIMITIVE_TYPES.contains(&symbol_path);
    }
    if symbol_path.starts_with("core::prelude::") || symbol_path.starts_with("std::prelude::") {
        return true;
    }
    RUST_PRELUDE_PATHS.contains(&symbol_path)
}

pub fn compute_auto_import_edit(doc_text: &str, symbol_path: &str) -> Option<Value> {
    if !symbol_path.contains("::") {
        return None;
    }

    if is_prelude_symbol(symbol_path) {
        return None;
    }

    let symbol_name = symbol_path.rsplit("::").next().unwrap_or(symbol_path);

    // Check if the symbol or path is already imported
    if is_already_imported(doc_text, symbol_path, symbol_name) {
        return None;
    }

    let insert_line = find_use_insert_line(doc_text);
    let new_text = format!("use {symbol_path};\n");

    Some(json!({
        "range": {
            "start": { "line": insert_line, "character": 0 },
            "end": { "line": insert_line, "character": 0 }
        },
        "newText": new_text
    }))
}

pub fn is_already_imported(doc_text: &str, symbol_path: &str, symbol_name: &str) -> bool {
    let parent_mod = symbol_path.rsplit_once("::").map(|(p, _)| p);

    for stmt in extract_use_statements(doc_text) {
        let trimmed = stmt.trim();
        // Exact full import: use foo::bar::Baz;
        if trimmed == symbol_path {
            return true;
        }

        // Glob import: use foo::bar::*;
        if let Some(parent) = parent_mod {
            if trimmed == format!("{parent}::*")
                || (trimmed.starts_with(parent) && trimmed.ends_with("::*"))
            {
                return true;
            }
        }

        // Braced groups: use foo::bar::{Baz, Qux}; or use foo::bar::{ Baz }; or multiline
        if let Some(parent) = parent_mod {
            if trimmed.starts_with(parent) && trimmed.contains('{') && trimmed.contains('}') {
                if let Some(open) = trimmed.find('{') {
                    if let Some(close) = trimmed.rfind('}') {
                        let inner = &trimmed[open + 1..close];
                        for item in inner.split(',') {
                            let item_trimmed = item.trim();
                            if item_trimmed == "*" {
                                return true;
                            }
                            if item_trimmed == symbol_name {
                                return true;
                            }
                            // Check alias: use foo::bar::{Baz as Alias} vs use foo::bar::{Qux as Baz}
                            if let Some((orig, alias)) = item_trimmed.split_once(" as ") {
                                if orig.trim() == symbol_name || alias.trim() == symbol_name {
                                    return true;
                                }
                            }
                        }
                    }
                }
            }
        }

        // Top-level alias: use foo::bar::Qux as Baz;
        if let Some((orig, alias)) = trimmed.split_once(" as ") {
            if orig.trim() == symbol_path || alias.trim() == symbol_name {
                return true;
            }
        }
    }

    false
}

fn extract_use_statements(doc_text: &str) -> Vec<String> {
    let mut stmts = Vec::new();
    let mut current_stmt = String::new();
    let mut in_use = false;

    for line in doc_text.lines() {
        let trimmed = line.trim();
        if !in_use {
            if trimmed.starts_with("use ") || trimmed.starts_with("pub use ") {
                let start_idx = if let Some(idx) = trimmed.find("use ") {
                    idx + 4
                } else {
                    0
                };
                current_stmt.push_str(&trimmed[start_idx..]);
                if trimmed.ends_with(';') {
                    let cleaned = current_stmt.trim_end_matches(';').trim().to_string();
                    stmts.push(cleaned);
                    current_stmt.clear();
                } else {
                    in_use = true;
                    current_stmt.push(' ');
                }
            }
        } else {
            current_stmt.push_str(trimmed);
            if trimmed.ends_with(';') {
                let cleaned = current_stmt.trim_end_matches(';').trim().to_string();
                stmts.push(cleaned);
                current_stmt.clear();
                in_use = false;
            } else {
                current_stmt.push(' ');
            }
        }
    }
    stmts
}

pub fn find_use_insert_line(doc_text: &str) -> u32 {
    let lines: Vec<&str> = doc_text.lines().collect();
    let mut last_use_line: Option<usize> = None;
    let mut header_end_line = 0usize;
    let mut in_multiline_use = false;
    let mut brace_depth = 0i32;

    for (idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim();

        if in_multiline_use {
            last_use_line = Some(idx);
            for ch in trimmed.chars() {
                if ch == '{' {
                    brace_depth += 1;
                } else if ch == '}' {
                    brace_depth -= 1;
                }
            }
            if brace_depth <= 0 && trimmed.contains(';') {
                in_multiline_use = false;
                brace_depth = 0;
            }
            continue;
        }

        // Track header comments / inner attributes
        if trimmed.starts_with("//!") || trimmed.starts_with("#![") || trimmed.is_empty() {
            if last_use_line.is_none() {
                header_end_line = idx + 1;
            }
            continue;
        }

        if trimmed.starts_with("use ") || trimmed.starts_with("pub use ") {
            last_use_line = Some(idx);
            for ch in trimmed.chars() {
                if ch == '{' {
                    brace_depth += 1;
                } else if ch == '}' {
                    brace_depth -= 1;
                }
            }
            if brace_depth > 0 || !trimmed.ends_with(';') {
                in_multiline_use = true;
            }
            continue;
        }

        // Hit first non-header, non-use code
        if last_use_line.is_some() {
            break;
        }
    }

    if let Some(last_use) = last_use_line {
        (last_use + 1) as u32
    } else {
        header_end_line as u32
    }
}
