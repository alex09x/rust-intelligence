/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Asynchronous background flycheck and compiler diagnostics publisher.

use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

#[derive(Default)]
pub struct FlycheckState {
    running_child: Mutex<Option<Child>>,
    published_uris: Mutex<HashSet<String>>,
}

impl FlycheckState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        let mut guard = self.running_child.lock().unwrap();
        if let Some(mut child) = guard.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    pub fn run_check<W: std::io::Write>(
        &self,
        workspace_root: &Path,
        writer: &mut W,
    ) -> std::io::Result<usize> {
        self.cancel();

        let mut cmd = Command::new("cargo");
        cmd.args([
            "check",
            "--message-format=json",
            "--workspace",
            "--all-targets",
        ]);
        cmd.current_dir(workspace_root);
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());

        // Check if ridx-driver is available in target/debug or target/release or PATH
        if let Some(driver) = find_ridx_driver(workspace_root) {
            cmd.env("RUSTC_WRAPPER", driver);
        }

        let mut child = cmd.spawn()?;
        let stdout = match child.stdout.take() {
            Some(out) => out,
            None => return Ok(0),
        };

        {
            let mut guard = self.running_child.lock().unwrap();
            *guard = Some(child);
        }

        let reader = BufReader::new(stdout);
        let mut file_diagnostics: HashMap<String, Vec<Value>> = HashMap::new();

        for line_res in reader.lines() {
            let line = match line_res {
                Ok(l) => l,
                Err(_) => break,
            };
            if let Ok(val) = serde_json::from_str::<Value>(&line) {
                if val.get("reason").and_then(|r| r.as_str()) == Some("compiler-message") {
                    if let Some(msg) = val.get("message") {
                        if let Some((uri, diag)) = parse_compiler_message(msg, workspace_root) {
                            file_diagnostics.entry(uri).or_default().push(diag);
                        }
                    }
                }
            }
        }

        // Wait on child to finish
        {
            let mut guard = self.running_child.lock().unwrap();
            if let Some(mut c) = guard.take() {
                let _ = c.wait();
            }
        }

        let mut published_guard = self.published_uris.lock().unwrap();
        let mut current_uris = HashSet::new();

        // 1. Publish new / updated diagnostics
        for (uri, diags) in &file_diagnostics {
            current_uris.insert(uri.clone());
            let notif = json!({
                "jsonrpc": "2.0",
                "method": "textDocument/publishDiagnostics",
                "params": {
                    "uri": uri,
                    "diagnostics": diags
                }
            });
            super::super::protocol::send_response(writer, &notif)?;
        }

        // 2. Clear previously published URIs that now have 0 errors
        for old_uri in published_guard.iter() {
            if !current_uris.contains(old_uri) {
                let clear_notif = json!({
                    "jsonrpc": "2.0",
                    "method": "textDocument/publishDiagnostics",
                    "params": {
                        "uri": old_uri,
                        "diagnostics": []
                    }
                });
                super::super::protocol::send_response(writer, &clear_notif)?;
            }
        }

        let total_diags = file_diagnostics.values().map(|d| d.len()).sum();
        *published_guard = current_uris;
        Ok(total_diags)
    }
}

pub fn parse_compiler_message(msg: &Value, root: &Path) -> Option<(String, Value)> {
    let spans = msg.get("spans")?.as_array()?;
    let primary_span = spans
        .iter()
        .find(|s| s.get("is_primary") == Some(&Value::Bool(true)))?;

    let file_name = primary_span.get("file_name")?.as_str()?;
    let abs_file = if Path::new(file_name).is_absolute() {
        PathBuf::from(file_name)
    } else {
        root.join(file_name)
    };
    let canon = abs_file.canonicalize().unwrap_or(abs_file);
    let uri = format!("file://{}", canon.display());

    let line_start = primary_span.get("line_start")?.as_u64()?.saturating_sub(1);
    let col_start = primary_span
        .get("column_start")?
        .as_u64()?
        .saturating_sub(1);
    let line_end = primary_span.get("line_end")?.as_u64()?.saturating_sub(1);
    let col_end = primary_span.get("column_end")?.as_u64()?.saturating_sub(1);

    let level_str = msg.get("level")?.as_str().unwrap_or("error");
    let severity = match level_str {
        "error" => 1,
        "warning" => 2,
        "note" => 3,
        "help" => 4,
        _ => 1,
    };

    let message_text = msg.get("message")?.as_str().unwrap_or("");
    let code_val = msg
        .get("code")
        .and_then(|c| c.get("code"))
        .cloned()
        .unwrap_or(Value::Null);

    let mut related = Vec::new();
    for s in spans {
        if s.get("is_primary") != Some(&Value::Bool(true)) {
            if let (Some(rel_file), Some(label)) = (
                s.get("file_name").and_then(|f| f.as_str()),
                s.get("label").and_then(|l| l.as_str()),
            ) {
                let rel_abs = if Path::new(rel_file).is_absolute() {
                    PathBuf::from(rel_file)
                } else {
                    root.join(rel_file)
                };
                let rel_canon = rel_abs.canonicalize().unwrap_or(rel_abs);
                let r_lstart = s
                    .get("line_start")
                    .and_then(|l| l.as_u64())
                    .unwrap_or(1)
                    .saturating_sub(1);
                let r_cstart = s
                    .get("column_start")
                    .and_then(|c| c.as_u64())
                    .unwrap_or(1)
                    .saturating_sub(1);
                let r_lend = s
                    .get("line_end")
                    .and_then(|l| l.as_u64())
                    .unwrap_or(1)
                    .saturating_sub(1);
                let r_cend = s
                    .get("column_end")
                    .and_then(|c| c.as_u64())
                    .unwrap_or(1)
                    .saturating_sub(1);

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

    Some((uri, diag))
}

fn find_ridx_driver(root: &Path) -> Option<PathBuf> {
    if let Ok(driver_env) = std::env::var("RIDX_DRIVER_BIN") {
        let p = PathBuf::from(driver_env);
        if p.exists() {
            return Some(p);
        }
    }

    let candidates = [
        root.join("target/debug/ridx-driver"),
        root.join("target/release/ridx-driver"),
    ];
    for c in &candidates {
        if c.exists() {
            return Some(c.clone());
        }
    }
    None
}
