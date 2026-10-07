/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Cargo configuration discovery, home resolution, and workspace manifest collection.

use std::fs;
use std::path::{Path, PathBuf};

pub fn resolve_cargo_home() -> Option<PathBuf> {
    if let Ok(cargo_home) = std::env::var("CARGO_HOME") {
        let p = PathBuf::from(cargo_home.trim());
        if !p.as_os_str().is_empty() {
            return Some(p);
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        let trimmed = home.trim();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed).join(".cargo"));
        }
    }
    #[cfg(windows)]
    if let Ok(userprofile) = std::env::var("USERPROFILE") {
        let p = PathBuf::from(userprofile.trim()).join(".cargo");
        if !userprofile.trim().is_empty() {
            return Some(p);
        }
    }
    None
}

pub fn find_cargo_configs(work_dir: &Path) -> Vec<PathBuf> {
    let mut configs = Vec::new();
    let mut cur = Some(work_dir);
    while let Some(dir) = cur {
        for name in [".cargo/config.toml", ".cargo/config"] {
            let p = dir.join(name);
            if p.is_file() && !configs.contains(&p) {
                configs.push(p);
            }
        }
        cur = dir.parent();
    }
    if let Some(home_dir) = resolve_cargo_home() {
        for name in ["config.toml", "config"] {
            let p = home_dir.join(name);
            if p.is_file() && !configs.contains(&p) {
                configs.push(p);
            }
        }
    }
    configs
}

pub fn extract_config_target(content: &str) -> Option<String> {
    let mut in_build = false;
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            in_build = line == "[build]";
            continue;
        }
        if in_build {
            if let Some((k, v)) = line.split_once('=') {
                if k.trim() == "target" {
                    let v = v.trim().trim_matches('"').trim_matches('\'').trim();
                    if !v.is_empty() {
                        return Some(v.to_string());
                    }
                }
            }
        }
    }
    None
}

pub fn collect_all_workspace_manifests(dir: &Path, manifests: &mut Vec<PathBuf>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_file() {
                if p.file_name().and_then(|s| s.to_str()) == Some("Cargo.toml") {
                    manifests.push(p);
                }
            } else if p.is_dir() {
                let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
                if name != "target" && name != ".git" && !name.starts_with('.') {
                    collect_all_workspace_manifests(&p, manifests);
                }
            }
        }
    }
}

pub fn compute_sysroot_ld_path() -> Option<String> {
    let out = std::process::Command::new("rustc").args(["--print", "sysroot"]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let sysroot = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let lib_dir = std::path::PathBuf::from(&sysroot).join("lib");
    let mut ld = lib_dir.to_string_lossy().to_string();
    if let Some(parent) = std::path::PathBuf::from(&sysroot).parent() {
        if let Ok(entries) = std::fs::read_dir(parent) {
            for entry in entries.flatten() {
                let cand_lib = entry.path().join("lib");
                if cand_lib.is_dir() {
                    ld = format!("{}:{}", ld, cand_lib.to_string_lossy());
                }
            }
        }
    }
    if let Ok(existing) = std::env::var("LD_LIBRARY_PATH") {
        ld = format!("{}:{}", ld, existing);
    }
    Some(ld)
}
