/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Verification test for flycheck publish and clear delivery under client queue saturation.

use ridx::lsp::run_lsp_server_stream;
use ridx_test_support::make_lsp_msg;
use serde_json::json;
use std::fs;
use std::io::Cursor;

use crate::PATH_MUTEX;

struct PipeReader {
    rx: std::sync::mpsc::Receiver<Vec<u8>>,
    cur: Cursor<Vec<u8>>,
}

impl std::io::Read for PipeReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.cur.read(buf)?;
        if n > 0 {
            return Ok(n);
        }
        match self.rx.recv() {
            Ok(b) => {
                self.cur = Cursor::new(b);
                self.cur.read(buf)
            }
            Err(_) => Ok(0),
        }
    }
}

impl std::io::BufRead for PipeReader {
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        if self.cur.position() < self.cur.get_ref().len() as u64 {
            return self.cur.fill_buf();
        }
        match self.rx.recv() {
            Ok(b) => {
                self.cur = Cursor::new(b);
                self.cur.fill_buf()
            }
            Err(_) => Ok(&[]),
        }
    }

    fn consume(&mut self, amt: usize) {
        self.cur.consume(amt);
    }
}

#[test]
fn test_flycheck_diagnostics_delivered_under_queue_saturation() {
    let _path_lock = PATH_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let pid = std::process::id();
    let tmp = std::env::temp_dir().join(format!("test_flycheck_sat_{pid}"));
    let bin_dir = tmp.join("bin");
    let ws_dir = tmp.join("workspace");
    let src_dir = ws_dir.join("src");
    fs::create_dir_all(&bin_dir).unwrap();
    fs::create_dir_all(&src_dir).unwrap();

    let count_file = tmp.join("counter");
    fs::write(&count_file, "0").unwrap();

    let mock_cargo = bin_dir.join("cargo");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let script = format!(
            r#"#!/bin/sh
CF="{}"
C=$(cat "$CF" 2>/dev/null || echo 0)
echo $((C + 1)) > "$CF"
if [ "$1" = "check" ]; then
  sleep 0.05
  if [ "$C" -eq 0 ]; then
    echo '{{"reason":"compiler-message","message":{{"message":"error file 1","level":"error","spans":[{{"file_name":"src/main.rs","is_primary":true,"line_start":1,"column_start":1,"line_end":1,"column_end":5}}]}}}}'
    echo '{{"reason":"compiler-message","message":{{"message":"error file 2","level":"error","spans":[{{"file_name":"src/extra.rs","is_primary":true,"line_start":1,"column_start":1,"line_end":1,"column_end":5}}]}}}}'
  fi
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

    fs::write(ws_dir.join("Cargo.toml"), "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n").unwrap();
    let main_file = src_dir.join("main.rs");
    let extra_file = src_dir.join("extra.rs");
    fs::write(&main_file, "fn main() {}\n").unwrap();
    fs::write(&extra_file, "pub fn extra() {}\n").unwrap();
    let main_uri = format!("file://{}", main_file.canonicalize().unwrap().display());

    let (pipe_tx, pipe_rx) = std::sync::mpsc::channel::<Vec<u8>>();
    let reader = PipeReader { rx: pipe_rx, cur: Cursor::new(Vec::new()) };

    let init_uri = format!("file://{}", ws_dir.canonicalize().unwrap().display());
    pipe_tx
        .send(make_lsp_msg(&json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "rootUri": init_uri, "capabilities": {} }
        })))
        .unwrap();
    pipe_tx.send(make_lsp_msg(&json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }))).unwrap();
    pipe_tx
        .send(make_lsp_msg(&json!({
            "jsonrpc": "2.0", "method": "textDocument/didOpen",
            "params": { "textDocument": { "uri": &main_uri, "languageId": "rust", "version": 1, "text": "fn main() {}\n" } }
        })))
        .unwrap();

    // Run 1: trigger errors, saturate queue with 70 requests
    pipe_tx
        .send(make_lsp_msg(&json!({
            "jsonrpc": "2.0", "method": "textDocument/didSave", "params": { "textDocument": { "uri": &main_uri } }
        })))
        .unwrap();
    for req_id in 100..170 {
        pipe_tx
            .send(make_lsp_msg(&json!({
                "jsonrpc": "2.0", "id": req_id, "method": "textDocument/hover",
                "params": { "textDocument": { "uri": &main_uri }, "position": { "line": 0, "character": 3 } }
            })))
            .unwrap();
    }

    let ws_clone = ws_dir.clone();
    let server_handle = std::thread::spawn(move || {
        let mut output = Vec::new();
        let res = run_lsp_server_stream(reader, &mut output, Some(ws_clone));
        (res, output)
    });

    std::thread::sleep(std::time::Duration::from_millis(250));

    // Run 2: clear errors, saturate queue with 70 requests again
    pipe_tx
        .send(make_lsp_msg(&json!({
            "jsonrpc": "2.0", "method": "textDocument/didSave", "params": { "textDocument": { "uri": &main_uri } }
        })))
        .unwrap();
    for req_id in 200..270 {
        pipe_tx
            .send(make_lsp_msg(&json!({
                "jsonrpc": "2.0", "id": req_id, "method": "textDocument/hover",
                "params": { "textDocument": { "uri": &main_uri }, "position": { "line": 0, "character": 3 } }
            })))
            .unwrap();
    }

    std::thread::sleep(std::time::Duration::from_millis(250));

    pipe_tx.send(make_lsp_msg(&json!({ "jsonrpc": "2.0", "id": 300, "method": "shutdown", "params": null }))).unwrap();
    pipe_tx.send(make_lsp_msg(&json!({ "jsonrpc": "2.0", "method": "exit", "params": null }))).unwrap();
    drop(pipe_tx);

    let (res, output) = server_handle.join().unwrap();
    std::env::set_var("PATH", orig_path);
    let _ = fs::remove_dir_all(&tmp);

    assert!(res.is_ok(), "Server should complete cleanly");
    let out_str = String::from_utf8_lossy(&output);

    for req_id in (100..170).chain(200..270) {
        assert!(out_str.contains(&format!("\"id\":{req_id}")), "Must respond to req {req_id}");
    }
    assert!(out_str.contains("\"id\":300"), "Must respond to shutdown");
    assert!(out_str.contains("error file 1"), "Published diagnostic 1 must arrive: {out_str}");
    assert!(out_str.contains("error file 2"), "Published diagnostic 2 must arrive: {out_str}");
    assert!(out_str.contains("\"diagnostics\":[]"), "Clear diagnostics must arrive: {out_str}");
}
