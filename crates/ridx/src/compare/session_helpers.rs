/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Helper functions for comparison sessions: server version detection and result matching.

use super::query_runner::spans_match;
use super::types::ExternalLspResult;
use std::process::Command;

pub fn get_server_version(cmd: &[String]) -> String {
    let bin = if cmd.first().map(|s| s.as_str()) == Some("taskset") {
        cmd.iter().skip(1).find(|s| !s.starts_with('-') && Some(s.as_str()) != cmd.get(2).map(|x| x.as_str()))
    } else {
        cmd.first()
    };
    if let Some(b) = bin {
        if let Ok(out) = Command::new(b).arg("--version").output() {
            if out.status.success() {
                return String::from_utf8_lossy(&out.stdout).trim().to_string();
            }
        }
    }
    "unknown".to_string()
}

pub fn check_results_match(res_a: Option<&ExternalLspResult>, res_b: Option<&ExternalLspResult>, capability: &str) -> bool {
    match (res_a, res_b) {
        (Some(a), Some(b)) => {
            if !a.found && !b.found {
                true
            } else if a.found && b.found {
                match capability {
                    "hover" | "textDocument/hover" => true,
                    "refs" | "textDocument/references" => a.count > 0 && b.count > 0,
                    _ => spans_match(&a.span, &b.span),
                }
            } else {
                false
            }
        }
        _ => false,
    }
}

pub fn check_results_match_line_level(res_a: Option<&ExternalLspResult>, res_b: Option<&ExternalLspResult>, capability: &str) -> bool {
    match (res_a, res_b) {
        (Some(a), Some(b)) => {
            if !a.found && !b.found {
                true
            } else if a.found && b.found {
                match capability {
                    "hover" | "textDocument/hover" => true,
                    "refs" | "textDocument/references" => a.count > 0 && b.count > 0,
                    _ => super::query_runner::spans_match_line_level(&a.span, &b.span),
                }
            } else {
                false
            }
        }
        _ => false,
    }
}

pub fn shutdown_lsp_server<R: std::io::BufRead, W: std::io::Write>(reader: &mut R, mut stdin: W, mut child: std::process::Child) -> u128 {
    let t_start = std::time::Instant::now();
    let shutdown_req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 999,
        "method": "shutdown",
        "params": null
    });
    let _ = super::transport::send_lsp_msg(&mut stdin, &shutdown_req);
    let _ = super::transport::read_lsp_response_with_writer(reader, &mut stdin, 999);
    let exit_notif = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "exit",
        "params": null
    });
    let _ = super::transport::send_lsp_msg(&mut stdin, &exit_notif);
    drop(stdin);

    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
    let mut graceful_exit = false;
    while std::time::Instant::now() < deadline {
        if let Ok(Some(_)) = child.try_wait() {
            graceful_exit = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    if !graceful_exit {
        let _ = child.kill();
        let _ = child.wait();
    }
    t_start.elapsed().as_micros()
}
