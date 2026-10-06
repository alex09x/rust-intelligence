/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Document formatting and edition resolution via rustfmt.

use crate::lsp::protocol::uri_to_path;
use crate::overlay::OverlayManager;
use serde_json::{json, Value};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

pub fn find_crate_edition(file_path: &str) -> String {
    let mut cur = Path::new(file_path).parent();
    let mut is_workspace_inherited = false;

    while let Some(dir) = cur {
        let cargo_toml = dir.join("Cargo.toml");
        if cargo_toml.is_file() {
            if let Ok(content) = std::fs::read_to_string(&cargo_toml) {
                let mut in_package = false;
                let mut in_workspace_package = false;

                for line in content.lines() {
                    let trimmed = line.trim();
                    if trimmed.starts_with('[') && trimmed.ends_with(']') {
                        let sec = trimmed[1..trimmed.len() - 1].trim();
                        in_package = sec == "package";
                        in_workspace_package = sec == "workspace.package";
                        continue;
                    }

                    if let Some((k, v)) = trimmed.split_once('=') {
                        let key = k.trim();
                        let val = v.trim().trim_matches('"').trim_matches('\'').trim();
                        if in_package && key.starts_with("edition") {
                            if val == "true" || key.contains("workspace") {
                                is_workspace_inherited = true;
                            } else if matches!(val, "2015" | "2018" | "2021" | "2024") {
                                return val.to_string();
                            }
                        }
                        if (in_workspace_package || is_workspace_inherited)
                            && key.starts_with("edition")
                            && matches!(val, "2015" | "2018" | "2021" | "2024")
                        {
                            return val.to_string();
                        }
                    }
                }
            }
        }
        cur = dir.parent();
    }

    "2021".to_string()
}

pub fn handle_formatting(msg: &Value, overlay_mgr: &OverlayManager) -> Value {
    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
    let file = uri_to_path(uri);

    let doc_text = if let Some(overlay) = overlay_mgr.get(&file) {
        overlay.text.clone()
    } else if let Ok(t) = std::fs::read_to_string(&file) {
        t
    } else {
        return json!([]);
    };

    let edition = find_crate_edition(&file);

    let mut cmd = Command::new("rustfmt");
    cmd.arg("--edition").arg(&edition);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    let Ok(mut child) = cmd.spawn() else {
        return json!([]);
    };

    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(doc_text.as_bytes());
    }

    let Ok(output) = child.wait_with_output() else {
        return json!([]);
    };

    if !output.status.success() {
        return json!([]);
    }

    let Ok(formatted) = String::from_utf8(output.stdout) else {
        return json!([]);
    };

    if formatted == doc_text {
        return json!([]);
    }

    let line_count = doc_text.lines().count() as u32;
    let last_line = doc_text.lines().last().unwrap_or("");
    let last_char = last_line.chars().count() as u32;

    json!([
        {
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": { "line": line_count, "character": last_char }
            },
            "newText": formatted
        }
    ])
}

pub fn handle_range_formatting(msg: &Value, overlay_mgr: &OverlayManager) -> Value {
    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
    let file = uri_to_path(uri);

    let doc_text = if let Some(overlay) = overlay_mgr.get(&file) {
        overlay.text.clone()
    } else if let Ok(t) = std::fs::read_to_string(&file) {
        t
    } else {
        return json!([]);
    };

    let start_line = msg["params"]["range"]["start"]["line"]
        .as_u64()
        .unwrap_or(0) as u32;
    let end_line = msg["params"]["range"]["end"]["line"]
        .as_u64()
        .unwrap_or(u32::MAX as u64) as u32;

    let doc_lines: Vec<&str> = doc_text.lines().collect();
    if doc_lines.is_empty() || start_line as usize >= doc_lines.len() {
        return json!([]);
    }
    let end_line = end_line.min(doc_lines.len().saturating_sub(1) as u32);

    let edition = find_crate_edition(&file);

    let mut cmd = Command::new("rustfmt");
    cmd.arg("--edition").arg(&edition);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    let Ok(mut child) = cmd.spawn() else {
        return json!([]);
    };

    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(doc_text.as_bytes());
    }

    let Ok(output) = child.wait_with_output() else {
        return json!([]);
    };

    if !output.status.success() {
        return json!([]);
    }

    let Ok(formatted) = String::from_utf8(output.stdout) else {
        return json!([]);
    };

    let fmt_lines: Vec<&str> = formatted.lines().collect();

    let prefix_len = start_line as usize;
    let suffix_len = doc_lines.len().saturating_sub(end_line as usize + 1);

    if prefix_len + suffix_len > fmt_lines.len() {
        return json!([]);
    }

    let fmt_slice = &fmt_lines[prefix_len..fmt_lines.len() - suffix_len];
    let new_text = fmt_slice.join("\n");
    let orig_slice = &doc_lines[prefix_len..=end_line as usize];
    let orig_text = orig_slice.join("\n");

    if new_text == orig_text {
        return json!([]);
    }

    let last_orig_line = doc_lines[end_line as usize];
    let last_char = last_orig_line.chars().count() as u32;

    json!([
        {
            "range": {
                "start": { "line": start_line, "character": 0 },
                "end": { "line": end_line, "character": last_char }
            },
            "newText": new_text
        }
    ])
}

pub fn handle_on_type_formatting(msg: &Value, overlay_mgr: &OverlayManager) -> Value {
    handle_formatting(msg, overlay_mgr)
}
