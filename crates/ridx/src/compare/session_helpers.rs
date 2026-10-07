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
        cmd.iter()
            .skip(1)
            .find(|s| !s.starts_with('-') && Some(s.as_str()) != cmd.get(2).map(|x| x.as_str()))
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

pub fn check_results_match(
    res_a: Option<&ExternalLspResult>,
    res_b: Option<&ExternalLspResult>,
    capability: &str,
) -> bool {
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
