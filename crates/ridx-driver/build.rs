/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=RUSTC");

    let rustc = env::var("RUSTC").unwrap_or_else(|_| "rustc".to_string());
    if let Ok(output) = Command::new(rustc).arg("--print").arg("sysroot").output() {
        if output.status.success() {
            let sysroot_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let lib_dir = PathBuf::from(sysroot_str).join("lib");
            if lib_dir.is_dir() {
                println!("cargo:rustc-link-search=native={}", lib_dir.display());
                if env::var("CARGO_CFG_TARGET_FAMILY").as_deref() == Ok("unix") {
                    println!("cargo:rustc-link-arg=-Wl,-rpath,{}", lib_dir.display());
                }
            }
        }
    }

    let commit = env::var("RIDX_BUILD_COMMIT").ok().unwrap_or_else(|| {
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

    let dirty = env::var("RIDX_BUILD_DIRTY").ok().map(|v| v == "true" || v == "1").unwrap_or_else(|| {
        Command::new("git")
            .args(["status", "--porcelain", "--untracked-files=no"])
            .output()
            .ok()
            .map(|out| out.status.success() && !String::from_utf8_lossy(&out.stdout).trim().is_empty())
            .unwrap_or(false)
    });

    println!("cargo:rustc-env=RIDX_BUILD_COMMIT={commit}");
    println!("cargo:rustc-env=RIDX_BUILD_DIRTY={dirty}");
    println!("cargo:rerun-if-env-changed=RIDX_BUILD_COMMIT");
    println!("cargo:rerun-if-env-changed=RIDX_BUILD_DIRTY");

    if let Ok(out) = Command::new("git").args(["rev-parse", "--git-dir"]).output() {
        if out.status.success() {
            let git_dir = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let p = Path::new(&git_dir);
            println!("cargo:rerun-if-changed={}", p.join("HEAD").display());
            println!("cargo:rerun-if-changed={}", p.join("index").display());
        }
    }

    let manifest_dir = env::var("CARGO_MANIFEST_DIR").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("."));
    let ws_root = manifest_dir.parent().and_then(|p| p.parent()).unwrap_or(&manifest_dir);
    println!("cargo:rerun-if-changed={}", ws_root.join("crates").display());
}
