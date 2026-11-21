/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Compiler message parser, span mapping, and rustc driver resolution.

use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub fn is_external_or_virtual(file_name: &str, root: &Path) -> bool {
    if file_name.starts_with('<') {
        return true;
    }
    let p = Path::new(file_name);
    if p.is_absolute() {
        !p.starts_with(root)
    } else {
        file_name.contains(".cargo/registry") || file_name.contains(".cargo/git")
    }
}

pub fn resolve_effective_span<'a>(span: &'a Value, root: &Path) -> &'a Value {
    let mut cur = span;
    let mut best = span;
    let file_name = span.get("file_name").and_then(|f| f.as_str()).unwrap_or("");

    while let Some(exp_span) = cur.get("expansion").and_then(|e| e.get("span")) {
        let exp_file = exp_span.get("file_name").and_then(|f| f.as_str()).unwrap_or("");
        if !is_external_or_virtual(exp_file, root) {
            best = exp_span;
        }
        cur = exp_span;
    }

    if is_external_or_virtual(file_name, root) || (cur != span && best != span) {
        best
    } else {
        span
    }
}

pub fn parse_compiler_message(msg: &Value, root: &Path) -> Option<(String, Value)> {
    parse_compiler_message_at(msg, root).map(|(path, diag)| (format!("file://{}", path.display()), diag))
}

/// [`parse_compiler_message`] with the file as a path (canonical), not a URI: a name with `%` in it is the name.
pub fn parse_compiler_message_at(msg: &Value, root: &Path) -> Option<(PathBuf, Value)> {
    let actual_msg = if let Some(reason) = msg.get("reason").and_then(|r| r.as_str()) {
        if reason != "compiler-message" {
            return None;
        }
        msg.get("message")?
    } else {
        msg
    };

    let spans = actual_msg.get("spans")?.as_array()?;
    let raw_primary = spans.iter().find(|s| s.get("is_primary") == Some(&Value::Bool(true)))?;
    let primary_span = resolve_effective_span(raw_primary, root);

    let file_name = primary_span.get("file_name")?.as_str()?;
    let abs_file = if Path::new(file_name).is_absolute() { PathBuf::from(file_name) } else { root.join(file_name) };
    let canon = ridx_format::canonicalize_best_effort(&abs_file);

    let line_start = primary_span.get("line_start")?.as_u64()?.saturating_sub(1);
    let col_start_char = primary_span.get("column_start")?.as_u64()?.saturating_sub(1);
    let line_end = primary_span.get("line_end")?.as_u64()?.saturating_sub(1);
    let col_end_char = primary_span.get("column_end")?.as_u64()?.saturating_sub(1);

    // Compute UTF-16 code unit offset: first line for start, last line for end
    let (col_start, col_end) = if let Some(text_arr) = primary_span.get("text").and_then(|t| t.as_array()) {
        let col_start = text_arr.first().and_then(|t| t.get("text")).and_then(|t| t.as_str()).map(|t| char_col_to_utf16(t, col_start_char)).unwrap_or(col_start_char);

        let col_end = text_arr.last().and_then(|t| t.get("text")).and_then(|t| t.as_str()).map(|t| char_col_to_utf16(t, col_end_char)).unwrap_or(col_end_char);

        (col_start, col_end)
    } else {
        (col_start_char, col_end_char)
    };

    let level_str = actual_msg.get("level")?.as_str().unwrap_or("error");
    let severity = match level_str {
        "error" => 1,
        "warning" => 2,
        "note" => 3,
        "help" => 4,
        _ => 1,
    };

    let mut message_text = actual_msg.get("message")?.as_str()?.to_string();
    // The primary span's label says what is wrong there ("expected `u32`, found `&str`" under "mismatched types"):
    // rust-analyzer puts it on the line after the message.
    if let Some(label) = raw_primary.get("label").and_then(|l| l.as_str()).filter(|l| !l.is_empty()) {
        message_text.push('\n');
        message_text.push_str(label);
    }

    // Capture child notes and help messages
    if let Some(children) = actual_msg.get("children").and_then(|c| c.as_array()) {
        for child in children {
            if let Some(ch_msg) = child.get("message").and_then(|m| m.as_str()) {
                if !ch_msg.is_empty() {
                    let ch_level = child.get("level").and_then(|l| l.as_str()).unwrap_or("note");
                    message_text.push_str(&format!("\n\n{ch_level}: {ch_msg}"));
                }
            }
        }
    }

    let code_val = actual_msg.get("code").and_then(|c| c.get("code")).cloned().unwrap_or(Value::Null);

    let mut related = Vec::new();
    for s in spans {
        if s.get("is_primary") != Some(&Value::Bool(true)) {
            if let (Some(rel_file), Some(label)) = (s.get("file_name").and_then(|f| f.as_str()), s.get("label").and_then(|l| l.as_str())) {
                let rel_abs = if Path::new(rel_file).is_absolute() { PathBuf::from(rel_file) } else { root.join(rel_file) };
                let rel_canon = ridx_format::canonicalize_best_effort(&rel_abs);
                let r_lstart = s.get("line_start").and_then(|l| l.as_u64()).unwrap_or(1).saturating_sub(1);
                let r_cstart = s.get("column_start").and_then(|c| c.as_u64()).unwrap_or(1).saturating_sub(1);
                let r_lend = s.get("line_end").and_then(|l| l.as_u64()).unwrap_or(1).saturating_sub(1);
                let r_cend = s.get("column_end").and_then(|c| c.as_u64()).unwrap_or(1).saturating_sub(1);

                related.push(json!({
                    "location": {
                        "uri": format!("file://{}", rel_canon.display()),
                        "range": {
                            "start": { "line": r_lstart, "character": r_cstart },
                            "end": { "line": r_lend, "character": r_cend }
                        }
                    },
                    "message": label
                }));
            }
        }
    }

    if primary_span != raw_primary {
        if let (Some(rel_file), Some(label)) = (raw_primary.get("file_name").and_then(|f| f.as_str()), raw_primary.get("label").and_then(|l| l.as_str())) {
            let rel_abs = if Path::new(rel_file).is_absolute() { PathBuf::from(rel_file) } else { root.join(rel_file) };
            let rel_canon = ridx_format::canonicalize_best_effort(&rel_abs);
            let r_lstart = raw_primary.get("line_start").and_then(|l| l.as_u64()).unwrap_or(1).saturating_sub(1);
            let r_cstart = raw_primary.get("column_start").and_then(|c| c.as_u64()).unwrap_or(1).saturating_sub(1);
            let r_lend = raw_primary.get("line_end").and_then(|l| l.as_u64()).unwrap_or(1).saturating_sub(1);
            let r_cend = raw_primary.get("column_end").and_then(|c| c.as_u64()).unwrap_or(1).saturating_sub(1);

            related.push(json!({
                "location": {
                    "uri": format!("file://{}", rel_canon.display()),
                    "range": {
                        "start": { "line": r_lstart, "character": r_cstart },
                        "end": { "line": r_lend, "character": r_cend }
                    }
                },
                "message": format!("in macro definition: {label}")
            }));
        }
    }

    let diag = json!({
        "range": {
            "start": { "line": line_start, "character": col_start },
            "end": { "line": line_end, "character": col_end }
        },
        "severity": severity,
        "code": code_val,
        "source": "rustc",
        "message": message_text,
        "relatedInformation": related
    });

    Some((canon, diag))
}

pub fn char_col_to_utf16(line_str: &str, char_col: u64) -> u64 {
    let mut utf16_count = 0u64;
    for (i, c) in line_str.chars().enumerate() {
        if i >= char_col as usize {
            break;
        }
        utf16_count += c.len_utf16() as u64;
    }
    utf16_count
}

pub fn find_ridx_driver(root: &Path) -> Option<PathBuf> {
    let env_driver = std::env::var("RIDX_DRIVER_BIN").ok();
    // The workspace's own build, else the driver the index build uses (beside the running ridx, then PATH).
    find_ridx_driver_with(root, env_driver.as_deref().map(Path::new)).or_else(|| crate::indexer::driver::find_driver_binary_internal(std::env::current_exe().ok().as_deref()))
}

pub fn find_ridx_driver_with(root: &Path, env_override: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = env_override {
        if p.exists() {
            return Some(p.to_path_buf());
        }
    }

    let candidates = [root.join("target/debug/ridx-driver"), root.join("target/release/ridx-driver")];
    for c in &candidates {
        if c.exists() {
            return Some(c.clone());
        }
    }
    None
}
