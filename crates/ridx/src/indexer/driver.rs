/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Resolution and discovery of the ridx-driver compiler wrapper binary.

use std::path::{Path, PathBuf};
use std::process::Command;

pub fn find_driver_binary() -> Option<PathBuf> {
    find_driver_binary_internal(std::env::current_exe().ok().as_deref())
}

pub fn find_driver_binary_internal(exe: Option<&Path>) -> Option<PathBuf> {
    find_driver_binary_with(std::env::var("RIDX_DRIVER_BIN").ok().as_deref(), exe, std::env::current_dir().ok().as_deref(), std::env::var("PATH").ok().as_deref())
}

pub fn find_driver_binary_with(driver_env: Option<&str>, exe: Option<&Path>, cwd: Option<&Path>, path_env: Option<&str>) -> Option<PathBuf> {
    if let Some(driver_env) = driver_env {
        let p = PathBuf::from(driver_env);
        if p.is_file() {
            return p.canonicalize().ok();
        }
        return None;
    }

    if let Some(exe) = exe {
        let mut cur = exe.parent();
        while let Some(dir) = cur {
            for sub in
                ["ridx-driver", "debug/ridx-driver", "release/ridx-driver", "target/debug/ridx-driver", "target/release/ridx-driver", "target/llvm-cov-target/debug/ridx-driver"]
            {
                let candidate = dir.join(sub);
                if candidate.is_file() {
                    return candidate.canonicalize().ok();
                }
            }
            cur = dir.parent();
        }
    }
    let base_cwd = cwd.map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    for rel in [
        "target/llvm-cov-target/debug/ridx-driver",
        "target/release/ridx-driver",
        "target/debug/ridx-driver",
        "../target/llvm-cov-target/debug/ridx-driver",
        "../target/release/ridx-driver",
        "../target/debug/ridx-driver",
    ] {
        let candidate = base_cwd.join(rel);
        if candidate.is_file() {
            return candidate.canonicalize().ok();
        }
    }
    if let Some(path_str) = path_env {
        for dir in std::env::split_paths(path_str) {
            if dir.as_os_str().is_empty() {
                continue;
            }
            let cand = dir.join("ridx-driver");
            if cand.is_file() {
                return Some(cand);
            }
        }
        return None;
    }
    if let Ok(out) = Command::new("which").arg("ridx-driver").output() {
        if out.status.success() {
            let path_str = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !path_str.is_empty() {
                return Some(PathBuf::from(path_str));
            }
        }
    }
    None
}
