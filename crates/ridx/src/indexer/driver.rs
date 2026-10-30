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
    if let Ok(driver_env) = std::env::var("RIDX_DRIVER_BIN") {
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
    for candidate in [
        PathBuf::from("target/llvm-cov-target/debug/ridx-driver"),
        PathBuf::from("target/release/ridx-driver"),
        PathBuf::from("target/debug/ridx-driver"),
        PathBuf::from("../target/llvm-cov-target/debug/ridx-driver"),
        PathBuf::from("../target/release/ridx-driver"),
        PathBuf::from("../target/debug/ridx-driver"),
    ] {
        if candidate.is_file() {
            return candidate.canonicalize().ok();
        }
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
