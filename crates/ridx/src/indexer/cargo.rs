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
    resolve_cargo_home_with(std::env::var("CARGO_HOME").ok().as_deref(), std::env::var("HOME").ok().as_deref())
}

pub fn resolve_cargo_home_with(cargo_home: Option<&str>, home: Option<&str>) -> Option<PathBuf> {
    if let Some(cargo_home) = cargo_home {
        let p = PathBuf::from(cargo_home.trim());
        if !p.as_os_str().is_empty() {
            return Some(p);
        }
    }
    if let Some(home) = home {
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
    find_cargo_configs_with_home(work_dir, resolve_cargo_home().as_deref())
}

pub fn find_cargo_configs_with_home(work_dir: &Path, cargo_home: Option<&Path>) -> Vec<PathBuf> {
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
    if let Some(home_dir) = cargo_home {
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

/// The rustup toolchain `ridx-driver` was built against (`1.98.1-aarch64-apple-darwin`), from the sysroot recorded
/// at build time. The driver compiles with that toolchain's `librustc_driver` only, so `cargo check` must run on it
/// even in a workspace whose `rust-toolchain.toml` pins another.
pub fn driver_toolchain() -> Option<String> {
    toolchain_of_sysroot_lib(build_sysroot_lib()?.to_str()?)
}

/// `<rustup home>/toolchains/<name>/lib` → `<name>`; `None` for a sysroot outside rustup.
pub(crate) fn toolchain_of_sysroot_lib(lib: &str) -> Option<String> {
    let lib = std::path::Path::new(lib);
    lib.parent().is_some_and(|t| t.is_dir()).then(|| toolchain_name_of_lib(lib)).flatten()
}

/// The toolchain name a rustup sysroot `lib` path carries, whether or not it exists on this machine.
fn toolchain_name_of_lib(lib: &std::path::Path) -> Option<String> {
    let toolchain = lib.parent()?;
    (lib.file_name()? == "lib" && toolchain.parent()?.file_name()? == "toolchains").then(|| toolchain.file_name()?.to_str().map(str::to_string)).flatten()
}

/// The sysroot `lib` of the toolchain ridx was built with, on this machine: the path recorded at build time, or,
/// for a release binary built on another machine, the same rustup toolchain under this machine's rustup home.
pub(crate) fn build_sysroot_lib() -> Option<std::path::PathBuf> {
    let recorded = option_env!("RIDX_BUILD_SYSROOT_LIB").filter(|s| !s.is_empty())?;
    let home = std::env::var_os("RUSTUP_HOME").map(std::path::PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".rustup")));
    relocate_sysroot_lib(recorded, home.as_deref())
}

pub(crate) fn relocate_sysroot_lib(recorded: &str, rustup_home: Option<&std::path::Path>) -> Option<std::path::PathBuf> {
    let recorded = std::path::Path::new(recorded);
    if recorded.is_dir() {
        return Some(recorded.to_path_buf());
    }
    let lib = rustup_home?.join("toolchains").join(toolchain_name_of_lib(recorded)?).join("lib");
    lib.is_dir().then_some(lib)
}

pub fn compute_sysroot_ld_path() -> Option<String> {
    // Prefer the toolchain sysroot recorded when `ridx`/`ridx-driver` was built so CWD
    // outside the workspace cannot resolve a different rustup toolchain whose libLLVM
    // collides with `librustc_driver`.
    let mut ld = build_sysroot_lib().map(|p| p.to_string_lossy().into_owned()).or_else(|| {
        let out = std::process::Command::new("rustc").args(["--print", "sysroot"]).output().ok()?;
        if !out.status.success() {
            return None;
        }
        let sysroot = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let lib_dir = std::path::PathBuf::from(sysroot).join("lib");
        Some(lib_dir.to_string_lossy().into_owned())
    })?;
    if let Ok(existing) = std::env::var("LD_LIBRARY_PATH") {
        if !existing.is_empty() {
            ld = format!("{ld}:{existing}");
        }
    }
    Some(ld)
}

#[cfg(test)]
mod tests {
    use super::{relocate_sysroot_lib, toolchain_of_sysroot_lib};

    #[test]
    fn test_the_driver_toolchain_is_read_off_a_rustup_sysroot() {
        let home = tempfile::tempdir().unwrap();
        let lib = home.path().join("toolchains/1.98.1-aarch64-apple-darwin/lib");
        std::fs::create_dir_all(&lib).unwrap();
        assert_eq!(toolchain_of_sysroot_lib(lib.to_str().unwrap()).as_deref(), Some("1.98.1-aarch64-apple-darwin"));
        let custom = home.path().join("my-rust/lib");
        std::fs::create_dir_all(&custom).unwrap();
        assert_eq!(toolchain_of_sysroot_lib(custom.to_str().unwrap()), None, "a sysroot outside rustup names no toolchain");
    }

    #[test]
    fn test_a_binary_built_elsewhere_finds_its_toolchain_under_this_rustup_home() {
        let home = tempfile::tempdir().unwrap();
        let built_on = "/home/runner/.rustup/toolchains/1.98.1-x86_64-unknown-linux-gnu/lib";
        assert_eq!(relocate_sysroot_lib(built_on, Some(home.path())), None, "the toolchain is not installed here");
        let here = home.path().join("toolchains/1.98.1-x86_64-unknown-linux-gnu/lib");
        std::fs::create_dir_all(&here).unwrap();
        assert_eq!(relocate_sysroot_lib(built_on, Some(home.path())), Some(here.clone()));
        assert_eq!(relocate_sysroot_lib(here.to_str().unwrap(), None), Some(here), "the recorded path is used where it exists");
        assert_eq!(relocate_sysroot_lib("/opt/my-rust/lib", Some(home.path())), None, "a sysroot outside rustup is not relocated");
    }
}
