/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Verification test for non-blocking asynchronous flycheck during LSP message dispatch.

use ridx::lsp::run_lsp_server_stream;
use ridx_test_support::make_lsp_msg;
use serde_json::json;
use std::fs;
use std::io::Cursor;
use std::time::Instant;

use crate::PATH_MUTEX;

#[test]
fn test_didsave_does_not_block_dispatch_loop_and_hover_responds_immediately() {
    let _path_lock = PATH_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let pid = std::process::id();
    let tmp = std::env::temp_dir().join(format!("test_flycheck_async_{pid}"));
    let bin_dir = tmp.join("bin");
    let ws_dir = tmp.join("workspace");
    let src_dir = ws_dir.join("src");
    fs::create_dir_all(&bin_dir).unwrap();
    fs::create_dir_all(&src_dir).unwrap();

    // Create a slow mock cargo script that sleeps for 3 seconds on "check"
    let mock_cargo = bin_dir.join("cargo");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let script = "#!/bin/sh\nif [ \"$1\" = \"check\" ]; then\n  sleep 3\nfi\nexit 0\n";
        fs::write(&mock_cargo, script).unwrap();
        let mut perms = fs::metadata(&mock_cargo).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&mock_cargo, perms).unwrap();
    }

    let orig_path = std::env::var("PATH").unwrap_or_default();
    let new_path = format!("{}:{}", bin_dir.display(), orig_path);
    std::env::set_var("PATH", &new_path);

    fs::write(
        ws_dir.join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    let main_file = src_dir.join("main.rs");
    fs::write(&main_file, "fn main() {\n    let val = 42;\n}\n").unwrap();
    let main_uri = format!("file://{}", main_file.canonicalize().unwrap().display());

    let mut input = Vec::new();
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "rootUri": format!("file://{}", ws_dir.canonicalize().unwrap().display()),
            "capabilities": {}
        }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "initialized",
        "params": {}
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &main_uri,
                "languageId": "rust",
                "version": 1,
                "text": "fn main() {\n    let val = 42;\n}\n"
            }
        }
    })));
    // didSave: would block for 3s if synchronous!
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didSave",
        "params": {
            "textDocument": { "uri": &main_uri }
        }
    })));
    // hover immediately after didSave: must respond in milliseconds
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "textDocument/hover",
        "params": {
            "textDocument": { "uri": &main_uri },
            "position": { "line": 1, "character": 9 }
        }
    })));
    // shutdown & exit
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "shutdown",
        "params": null
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "exit",
        "params": null
    })));

    let mut output = Vec::new();
    let t_start = Instant::now();
    let res = run_lsp_server_stream(Cursor::new(input), &mut output, Some(ws_dir.clone()));
    let elapsed = t_start.elapsed();

    std::env::set_var("PATH", orig_path);
    let _ = fs::remove_dir_all(&tmp);

    assert!(res.is_ok(), "Server should complete cleanly");
    // Verify hover and shutdown were processed without blocking for the 3s cargo sleep!
    assert!(
        elapsed.as_millis() < 1500,
        "LSP dispatch loop should NOT block on cargo check: elapsed was {} ms",
        elapsed.as_millis()
    );

    // Verify responses contain initialize, hover, and shutdown
    let out_str = String::from_utf8_lossy(&output);
    assert!(out_str.contains("\"id\":1"), "Must respond to initialize");
    assert!(out_str.contains("\"id\":2"), "Must respond to hover");
    assert!(out_str.contains("\"id\":3"), "Must respond to shutdown");
}

#[test]
fn test_flycheck_overlapping_workers_race_and_cancel() {
    let _path_lock = PATH_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    use ridx::lsp::presentation::flycheck::FlycheckState;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    let pid = std::process::id();
    let tmp = std::env::temp_dir().join(format!("test_flycheck_race_{pid}"));
    let bin_dir = tmp.join("bin");
    let ws_dir = tmp.join("workspace");
    let src_dir = ws_dir.join("src");
    let count_file = tmp.join("counter");
    fs::create_dir_all(&bin_dir).unwrap();
    fs::create_dir_all(&src_dir).unwrap();
    fs::write(&count_file, "0").unwrap();

    let mock_cargo = bin_dir.join("cargo");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let script = format!(
            r#"#!/bin/sh
COUNT_FILE="{}"
CNT=$(cat "$COUNT_FILE" 2>/dev/null || echo 0)
NEXT=$((CNT + 1))
echo "$NEXT" > "$COUNT_FILE"

if [ "$CNT" -eq 0 ]; then
  sleep 1
  echo '{{"reason":"compiler-message","message":{{"message":"error from worker A","level":"error","spans":[{{"file_name":"src/main.rs","is_primary":true,"line_start":1,"column_start":1,"line_end":1,"column_end":5}}]}}}}'
else
  sleep 0.05
  echo '{{"reason":"compiler-message","message":{{"message":"error from worker B","level":"error","spans":[{{"file_name":"src/main.rs","is_primary":true,"line_start":2,"column_start":1,"line_end":2,"column_end":5}}]}}}}'
fi
exit 0
"#,
            count_file.display()
        );
        fs::write(&mock_cargo, script).unwrap();
        let mut perms = fs::metadata(&mock_cargo).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&mock_cargo, perms).unwrap();
    }

    let orig_path = std::env::var("PATH").unwrap_or_default();
    let new_path = format!("{}:{}", bin_dir.display(), orig_path);
    std::env::set_var("PATH", &new_path);

    fs::write(
        ws_dir.join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    let main_file = src_dir.join("main.rs");
    fs::write(&main_file, "fn main() {}\n").unwrap();

    let state = FlycheckState::new();
    let captured = Arc::new(Mutex::new(Vec::new()));
    let cap_clone = Arc::clone(&captured);
    state.set_notifier(move |_gen, notif| {
        cap_clone.lock().unwrap().push(notif);
    });

    // Trigger A, then immediately trigger B
    state.trigger_check(&ws_dir);
    std::thread::sleep(Duration::from_millis(15));
    state.trigger_check(&ws_dir);

    // Wait for B to finish (200ms is plenty for B's 50ms sleep)
    std::thread::sleep(Duration::from_millis(250));

    let notifs = captured.lock().unwrap().clone();

    let all_messages: Vec<String> = notifs
        .iter()
        .filter_map(|n| {
            n["params"]["diagnostics"].as_array().map(|arr| {
                arr.iter()
                    .map(|d| d["message"].as_str().unwrap_or("").to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            })
        })
        .collect();

    let all_str = all_messages.join("; ");
    assert!(
        all_str.contains("error from worker B"),
        "Worker B must publish diagnostic: {all_str}"
    );
    assert!(
        !all_str.contains("error from worker A"),
        "Worker A must NOT publish after being superseded: {all_str}"
    );

    state.cancel();
    state.shutdown();

    std::env::set_var("PATH", orig_path);
    let _ = fs::remove_dir_all(&tmp);
}
