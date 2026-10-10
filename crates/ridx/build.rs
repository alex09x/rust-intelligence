/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use std::process::Command;

fn main() {
    let commit = std::env::var("RIDX_BUILD_COMMIT").ok().unwrap_or_else(|| {
        Command::new("git")
            .args(["rev-parse", "HEAD"])
            .output()
            .ok()
            .and_then(|out| {
                if out.status.success() {
                    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    if s.len() == 40 {
                        Some(s)
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .unwrap_or_else(|| "unknown".to_string())
    });

    let dirty = std::env::var("RIDX_BUILD_DIRTY")
        .ok()
        .map(|v| v == "true" || v == "1")
        .unwrap_or_else(|| {
            Command::new("git")
                .args(["status", "--porcelain", "--untracked-files=no"])
                .output()
                .ok()
                .map(|out| {
                    out.status.success()
                        && !String::from_utf8_lossy(&out.stdout).trim().is_empty()
                })
                .unwrap_or(false)
        });

    println!("cargo:rustc-env=RIDX_BUILD_COMMIT={commit}");
    println!("cargo:rustc-env=RIDX_BUILD_DIRTY={dirty}");
    println!("cargo:rerun-if-env-changed=RIDX_BUILD_COMMIT");
    println!("cargo:rerun-if-env-changed=RIDX_BUILD_DIRTY");
}
