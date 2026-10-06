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

const RUST_PRELUDE_ITEMS: &[&str] = &[
    "bool",
    "char",
    "str",
    "u8",
    "u16",
    "u32",
    "u64",
    "u128",
    "usize",
    "i8",
    "i16",
    "i32",
    "i64",
    "i128",
    "isize",
    "f32",
    "f64",
    "String",
    "Vec",
    "Option",
    "Some",
    "None",
    "Result",
    "Ok",
    "Err",
    "Box",
    "Drop",
    "Clone",
    "Copy",
    "Default",
    "Send",
    "Sync",
    "Into",
    "From",
    "Iterator",
    "IntoIterator",
    "AsRef",
    "AsMut",
    "PartialEq",
    "PartialOrd",
    "Eq",
    "Ord",
    "Hash",
];

pub fn compute_auto_import_edit(doc_text: &str, symbol_path: &str) -> Option<Value> {
    if !symbol_path.contains("::") {
        return None;
    }

    let symbol_name = symbol_path.rsplit("::").next().unwrap_or(symbol_path);
    if RUST_PRELUDE_ITEMS.contains(&symbol_name) {
        return None;
    }

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
    let exact_use = format!("use {symbol_path};");
    if doc_text.contains(&exact_use) {
        return true;
    }

    for line in doc_text.lines() {
        let trimmed = line.trim();
        let is_use = trimmed.starts_with("use ") && trimmed.ends_with(';');
        if is_use
            && (trimmed.contains(&format!("::{symbol_name};"))
                || trimmed.contains(&format!("::{symbol_name} as "))
                || trimmed.contains(&format!(", {symbol_name},"))
                || trimmed.contains(&format!("{{{symbol_name},"))
                || trimmed.contains(&format!(", {symbol_name}}}"))
                || trimmed.contains(&format!("{{{symbol_name}}}")))
        {
            return true;
        }
    }

    false
}

pub fn find_use_insert_line(doc_text: &str) -> u32 {
    let lines: Vec<&str> = doc_text.lines().collect();
    let mut last_use_line: Option<usize> = None;
    let mut header_end_line = 0usize;

    for (idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim();

        // Track header comments / inner attributes
        if trimmed.starts_with("//!") || trimmed.starts_with("#![") || trimmed.is_empty() {
            if last_use_line.is_none() {
                header_end_line = idx + 1;
            }
            continue;
        }

        if trimmed.starts_with("use ") || (last_use_line.is_some() && trimmed.ends_with(';')) {
            last_use_line = Some(idx);
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
