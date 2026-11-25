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

/// The driver's file name on this platform: `ridx-driver.exe` on Windows, where a release run found no driver next to
/// `ridx.exe` looking for `ridx-driver`.
pub fn driver_file_name() -> String {
    format!("ridx-driver{}", std::env::consts::EXE_SUFFIX)
}

/// `path` canonicalized, without the `\\?\` prefix Windows gives a canonical path with a drive letter: Cargo runs the
/// wrapper by that path, and tools reading it back expect the ordinary form.
pub fn canonical(path: &Path) -> Option<PathBuf> {
    let canon = path.canonicalize().ok()?;
    let text = canon.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => Some(PathBuf::from(rest)),
        _ => Some(canon),
    }
}

pub fn find_driver_binary_with(driver_env: Option<&str>, exe: Option<&Path>, cwd: Option<&Path>, path_env: Option<&str>) -> Option<PathBuf> {
    if let Some(driver_env) = driver_env {
        let p = PathBuf::from(driver_env);
        return if p.is_file() { canonical(&p) } else { None };
    }
    let name = driver_file_name();

    if let Some(exe) = exe {
        let mut cur = exe.parent();
        while let Some(dir) = cur {
            for sub in ["", "debug", "release", "target/debug", "target/release", "target/llvm-cov-target/debug"] {
                let candidate = dir.join(sub).join(&name);
                if candidate.is_file() {
                    return canonical(&candidate);
                }
            }
            cur = dir.parent();
        }
    }
    let base_cwd = cwd.map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    for rel in ["target/llvm-cov-target/debug", "target/release", "target/debug", "../target/llvm-cov-target/debug", "../target/release", "../target/debug"] {
        let candidate = base_cwd.join(rel).join(&name);
        if candidate.is_file() {
            return canonical(&candidate);
        }
    }
    if let Some(path_str) = path_env {
        for dir in std::env::split_paths(path_str) {
            if dir.as_os_str().is_empty() {
                continue;
            }
            let cand = dir.join(&name);
            if cand.is_file() {
                return Some(cand);
            }
        }
        return None;
    }
    if let Ok(out) = Command::new(if cfg!(windows) { "where" } else { "which" }).arg(&name).output() {
        if out.status.success() {
            let path_str = String::from_utf8_lossy(&out.stdout).lines().next().unwrap_or("").trim().to_string();
            if !path_str.is_empty() {
                return Some(PathBuf::from(path_str));
            }
        }
    }
    None
}
