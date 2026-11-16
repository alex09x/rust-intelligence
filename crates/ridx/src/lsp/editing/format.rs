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
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};

fn rustfmt_bin() -> &'static Path {
    static BIN: OnceLock<PathBuf> = OnceLock::new();
    BIN.get_or_init(|| {
        if let Ok(out) = Command::new("rustup").args(["which", "rustfmt"]).output() {
            if out.status.success() {
                if let Ok(s) = String::from_utf8(out.stdout) {
                    let p = PathBuf::from(s.trim());
                    if p.is_file() {
                        return p;
                    }
                }
            }
        }
        PathBuf::from("rustfmt")
    })
}

pub fn find_crate_edition(file_path: &str) -> String {
    let mut cur = Path::new(file_path).parent();
    let mut is_workspace_inherited = false;

    while let Some(dir) = cur {
        let cargo_toml = dir.join("Cargo.toml");
        if cargo_toml.is_file() {
            if let Ok(content) = std::fs::read_to_string(&cargo_toml) {
                let mut in_package = false;
                let mut in_workspace_package = false;
                let mut found_package = false;

                for line in content.lines() {
                    let trimmed = line.trim();
                    if trimmed.starts_with('[') && trimmed.ends_with(']') {
                        let sec = trimmed[1..trimmed.len() - 1].trim();
                        in_package = sec == "package";
                        if in_package {
                            found_package = true;
                        }
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
                        if (in_workspace_package || is_workspace_inherited) && key.starts_with("edition") && matches!(val, "2015" | "2018" | "2021" | "2024") {
                            return val.to_string();
                        }
                    }
                }

                if found_package && !is_workspace_inherited {
                    return "2021".to_string();
                }
            }
        }

        // Stop ascending if repository boundary is reached
        if dir.join(".git").exists() {
            break;
        }

        cur = dir.parent();
    }

    "2021".to_string()
}

fn default_rustfmt_config_dir() -> &'static Path {
    static DEFAULT_DIR: OnceLock<PathBuf> = OnceLock::new();
    DEFAULT_DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join("ridx_default_rustfmt_cfg");
        let _ = std::fs::create_dir_all(&dir);
        let cfg = dir.join("rustfmt.toml");
        if !cfg.is_file() {
            let _ = std::fs::write(&cfg, "");
        }
        dir
    })
}

fn find_rustfmt_config_dir(file_path: &str) -> PathBuf {
    let mut cur = Path::new(file_path).parent();
    while let Some(dir) = cur {
        if dir.join("rustfmt.toml").is_file() || dir.join(".rustfmt.toml").is_file() {
            return dir.to_path_buf();
        }
        if dir.join(".git").exists() {
            break;
        }
        cur = dir.parent();
    }
    default_rustfmt_config_dir().to_path_buf()
}

pub fn handle_formatting(msg: &Value, overlay_mgr: &OverlayManager) -> Value {
    static FMT_CACHE: OnceLock<Mutex<HashMap<(PathBuf, u64), Value>>> = OnceLock::new();
    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
    let file = uri_to_path(uri);

    // Only a buffer the editor opened: the file on disk may be behind it. Zed asks to format a file it saves before it
    // opens it, right after a rename edited that buffer; formatting the text on disk wrote the old names back.
    let Some(doc_text) = overlay_mgr.get(&file).map(|o| o.text.clone()) else {
        return json!([]);
    };

    if doc_text.trim().is_empty() {
        return json!([]);
    }

    let edition = find_crate_edition(&file);
    let cfg_dir = find_rustfmt_config_dir(&file);
    let mut hasher = DefaultHasher::new();
    edition.hash(&mut hasher);
    doc_text.hash(&mut hasher);
    let cache_key = (cfg_dir.clone(), hasher.finish());

    if let Ok(guard) = FMT_CACHE.get_or_init(|| Mutex::new(HashMap::new())).lock() {
        if let Some(cached) = guard.get(&cache_key) {
            return cached.clone();
        }
    }

    let mut cmd = Command::new(rustfmt_bin());
    cmd.arg("--config-path").arg(&cfg_dir);
    cmd.arg("--edition").arg(&edition);
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());

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
    };

    let Ok(formatted) = String::from_utf8(output.stdout) else {
        return json!([]);
    };

    let result = if formatted == doc_text {
        json!([])
    } else {
        let line_count = doc_text.lines().count() as u32;
        let last_line = doc_text.lines().last().unwrap_or("");
        // LSP columns count UTF-16 code units, so a non-BMP char on an unterminated last line spans two.
        let last_char = last_line.encode_utf16().count() as u32;
        let (end_line, end_char) = if doc_text.ends_with('\n') { (line_count, 0) } else { (line_count.saturating_sub(1), last_char) };
        json!([
            {
                "range": {
                    "start": { "line": 0, "character": 0 },
                    "end": { "line": end_line, "character": end_char }
                },
                "newText": formatted
            }
        ])
    };

    if let Ok(mut guard) = FMT_CACHE.get_or_init(|| Mutex::new(HashMap::new())).lock() {
        if guard.len() >= 256 {
            guard.clear();
        }
        guard.insert(cache_key, result.clone());
    }

    result
}

pub fn handle_range_formatting(msg: &Value, overlay_mgr: &OverlayManager) -> Value {
    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
    let file = uri_to_path(uri);

    // Only an open buffer is formatted, as in `handle_formatting`: a buffer edited before its didOpen (a multi-file
    // rename) differs from the disk, and edits computed from the disk would revert it.
    let Some(doc_text) = overlay_mgr.get(&file).map(|o| o.text.clone()) else {
        return json!([]);
    };

    let start_line = msg["params"]["range"]["start"]["line"].as_u64().unwrap_or(0) as u32;
    let end_line = msg["params"]["range"]["end"]["line"].as_u64().unwrap_or(u32::MAX as u64) as u32;
    // A selection of whole lines ends at the start of the next one (`character: 0`), which it does not include.
    let end_at_line_start = msg["params"]["range"]["end"]["character"].as_u64() == Some(0);
    let end_line = if end_at_line_start && end_line > start_line { end_line - 1 } else { end_line };

    let doc_lines: Vec<&str> = doc_text.lines().collect();
    if doc_lines.is_empty() || start_line as usize >= doc_lines.len() {
        return json!([]);
    }
    let end_line = end_line.min(doc_lines.len().saturating_sub(1) as u32);

    let orig_slice = &doc_lines[start_line as usize..=end_line as usize];
    let orig_text = orig_slice.join("\n");
    let edition = find_crate_edition(&file);

    let Some(formatted_at_column_0) = format_range_snippet(&orig_text, &edition, &file) else {
        return json!([]);
    };
    // The snippet is formatted on its own, at column 0: it keeps the indentation it has in the file.
    // A line inside a multi-line string literal is the literal's text: it gets no indentation.
    let indent = orig_slice.iter().find(|l| !l.trim().is_empty()).map_or("", |l| &l[..l.len() - l.trim_start().len()]);
    let in_string = lines_starting_in_string(&formatted_at_column_0);
    let formatted_snippet = formatted_at_column_0
        .lines()
        .enumerate()
        .map(|(i, l)| if l.is_empty() || in_string.contains(&i) { l.to_string() } else { format!("{indent}{l}") })
        .collect::<Vec<_>>()
        .join("\n");

    if formatted_snippet == orig_text {
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
            "newText": formatted_snippet
        }
    ])
}

/// The 0-based lines of `text` that start inside a string literal (`"…"`, `r#"…"#`, `b"…"`) opened on a line above.
fn lines_starting_in_string(text: &str) -> std::collections::HashSet<usize> {
    let b = text.as_bytes();
    let (mut out, mut line, mut i) = (std::collections::HashSet::new(), 0usize, 0usize);
    while i < b.len() {
        let rest = &b[i..];
        if rest.starts_with(b"//") {
            i += rest.iter().position(|&c| c == b'\n').unwrap_or(rest.len());
            continue;
        }
        // A block comment (nested ones too) holds no string: `/* " */`.
        if rest.starts_with(b"/*") {
            let (mut depth, mut j) = (1usize, i + 2);
            while j < b.len() && depth > 0 {
                if b[j..].starts_with(b"/*") {
                    depth += 1;
                    j += 2;
                } else if b[j..].starts_with(b"*/") {
                    depth -= 1;
                    j += 2;
                } else {
                    line += usize::from(b[j] == b'\n');
                    j += 1;
                }
            }
            i = j;
            continue;
        }
        // A char literal (`'"'`, `'\''`) holds no string.
        if rest[0] == b'\'' {
            let len = if rest.get(1) == Some(&b'\\') { rest.iter().skip(2).position(|&c| c == b'\'').map(|p| p + 3) } else { (rest.get(2) == Some(&b'\'')).then_some(3) };
            i += len.unwrap_or(1);
            continue;
        }
        // A raw string: `r` (or `br`, `cr`), hashes, `"`, up to `"` and as many hashes.
        let prev_is_ident = i > 0 && (b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_');
        let after_r = rest.strip_prefix(b"br").or_else(|| rest.strip_prefix(b"cr")).or_else(|| rest.strip_prefix(b"r")).filter(|_| !prev_is_ident);
        let raw_hashes = after_r.and_then(|r| {
            let hashes = r.iter().take_while(|&&c| c == b'#').count();
            (r.get(hashes) == Some(&b'"')).then_some(hashes)
        });
        let (open_len, close) = match raw_hashes {
            Some(h) => (rest.len() - after_r.map_or(0, <[u8]>::len) + h + 1, [b"\"".as_slice(), &b"#".repeat(h)].concat()),
            None if rest[0] == b'"' => (1, b"\"".to_vec()),
            None => {
                if rest[0] == b'\n' {
                    line += 1;
                }
                i += 1;
                continue;
            }
        };
        let mut j = i + open_len;
        while j < b.len() {
            if raw_hashes.is_none() && b[j] == b'\\' {
                if b.get(j + 1) == Some(&b'\n') {
                    line += 1;
                    out.insert(line);
                }
                j += 2;
                continue;
            }
            if b[j..].starts_with(&close) {
                break;
            }
            if b[j] == b'\n' {
                line += 1;
                out.insert(line);
            }
            j += 1;
        }
        i = (j + close.len()).min(b.len());
    }
    out
}

fn format_range_snippet(snippet: &str, edition: &str, file: &str) -> Option<String> {
    let cfg_dir = find_rustfmt_config_dir(file);
    // 1. Try formatting the range snippet directly as Rust items
    let mut cmd = Command::new(rustfmt_bin());
    cmd.arg("--config-path").arg(&cfg_dir);
    cmd.arg("--edition").arg(edition);
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());

    if let Ok(mut child) = cmd.spawn() {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(snippet.as_bytes());
        }
        if let Ok(output) = child.wait_with_output() {
            if output.status.success() {
                if let Ok(formatted) = String::from_utf8(output.stdout) {
                    let trimmed = formatted.trim_end().to_string();
                    if !trimmed.is_empty() {
                        return Some(trimmed);
                    }
                }
            }
        }
    }

    // 2. Wrap snippet inside a function body if it contains statements
    let wrapped = format!("fn __ridx_wrap() {{\n{snippet}\n}}\n");
    let mut cmd = Command::new(rustfmt_bin());
    cmd.arg("--config-path").arg(&cfg_dir);
    cmd.arg("--edition").arg(edition);
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());

    if let Ok(mut child) = cmd.spawn() {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(wrapped.as_bytes());
        }
        if let Ok(output) = child.wait_with_output() {
            if output.status.success() {
                if let Ok(formatted) = String::from_utf8(output.stdout) {
                    let lines: Vec<&str> = formatted.lines().collect();
                    if lines.len() >= 2 && lines.first().map(|l| l.contains("__ridx_wrap")).unwrap_or(false) && lines.last() == Some(&"}") {
                        let inner_lines = &lines[1..lines.len() - 1];
                        // The wrapper's indentation comes off the code; a line inside a multi-line string literal is
                        // the literal's text and keeps every space.
                        let in_string = lines_starting_in_string(&inner_lines.join("\n"));
                        let unwrapped: Vec<String> = inner_lines
                            .iter()
                            .enumerate()
                            .map(|(i, l)| match l.strip_prefix("    ") {
                                Some(stripped) if !in_string.contains(&i) => stripped.to_string(),
                                _ => l.to_string(),
                            })
                            .collect();
                        let res = unwrapped.join("\n").trim_end().to_string();
                        if !res.is_empty() {
                            return Some(res);
                        }
                    }
                }
            }
        }
    }

    None
}

pub fn handle_on_type_formatting(msg: &Value, overlay_mgr: &OverlayManager) -> Value {
    handle_formatting(msg, overlay_mgr)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::overlay::OverlayManager;
    use serde_json::json;

    #[test]
    fn test_handle_formatting_range_end_with_trailing_newline() {
        let mut overlay_mgr = OverlayManager::new();
        let uri = "file:///tmp/fmt_nl.rs";
        let code = "fn main() {\nlet x = 1;\n}\n";
        overlay_mgr.open_document(uri, "/tmp/fmt_nl.rs", 1, code.to_string());

        let msg = json!({
            "params": {
                "textDocument": { "uri": uri }
            }
        });
        let res = handle_formatting(&msg, &overlay_mgr);
        let edits = res.as_array().expect("edits array");
        assert_eq!(edits.len(), 1);
        let end = &edits[0]["range"]["end"];
        assert_eq!(end["line"], 3);
        assert_eq!(end["character"], 0);
    }

    #[test]
    fn test_handle_formatting_range_end_without_trailing_newline() {
        let mut overlay_mgr = OverlayManager::new();
        let uri = "file:///tmp/fmt_no_nl.rs";
        let code = "fn main() {\nlet x = 1;\n}";
        overlay_mgr.open_document(uri, "/tmp/fmt_no_nl.rs", 1, code.to_string());

        let msg = json!({
            "params": {
                "textDocument": { "uri": uri }
            }
        });
        let res = handle_formatting(&msg, &overlay_mgr);
        let edits = res.as_array().expect("edits array");
        assert_eq!(edits.len(), 1);
        let end = &edits[0]["range"]["end"];
        assert_eq!(end["line"], 2);
        assert_eq!(end["character"], 1);
    }

    #[test]
    fn test_handle_formatting_range_end_counts_utf16_units() {
        let mut overlay_mgr = OverlayManager::new();
        let uri = "file:///tmp/fmt_utf16.rs";
        let code = "fn main() {\nlet s = \"🦀\";}";
        overlay_mgr.open_document(uri, "/tmp/fmt_utf16.rs", 1, code.to_string());

        let res = handle_formatting(&json!({ "params": { "textDocument": { "uri": uri } } }), &overlay_mgr);
        let edits = res.as_array().expect("edits array");
        assert_eq!(edits.len(), 1);
        // `let s = "🦀";}` is 13 chars but 14 UTF-16 units: the crab is a surrogate pair.
        assert_eq!(edits[0]["range"]["end"], json!({ "line": 1, "character": 14 }));
    }

    #[test]
    fn test_handle_formatting_blank_document_returns_no_edits() {
        let mut overlay_mgr = OverlayManager::new();
        let uri = "file:///tmp/fmt_blank.rs";
        let code = "   \n\n  \t \n";
        overlay_mgr.open_document(uri, "/tmp/fmt_blank.rs", 1, code.to_string());

        let msg = json!({
            "params": {
                "textDocument": { "uri": uri }
            }
        });
        let res = handle_formatting(&msg, &overlay_mgr);
        let edits = res.as_array().expect("edits array");
        assert_eq!(edits, json!([]).as_array().unwrap());
    }
}
