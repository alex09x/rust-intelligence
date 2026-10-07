/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Build configuration verification and untracked file detection for direct fast-path indexing.

use crate::indexer::types::IndexOptions;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::time::SystemTime;

/// Computes FNV-1a 64-bit content hashes of workspace configuration files (Cargo.toml, Cargo.lock, build.rs, .cargo/config*).
pub fn compute_workspace_config_hashes(work_dir: &Path) -> HashMap<String, u64> {
    let mut hashes = HashMap::new();
    let root_files = [
        "Cargo.toml",
        "Cargo.lock",
        "rust-toolchain",
        "rust-toolchain.toml",
        ".cargo/config",
        ".cargo/config.toml",
    ];
    for rel in &root_files {
        let p = work_dir.join(rel);
        if p.is_file() {
            if let Ok(bytes) = fs::read(&p) {
                hashes.insert(rel.to_string(), ridx_format::fnv1a_64(&bytes));
            }
        }
    }

    // Discover member Cargo.toml and build.rs files across the workspace
    let mut stack = vec![work_dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if let Ok(entries) = fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                let fname = entry.file_name();
                let fname_str = fname.to_string_lossy();
                if p.is_dir() {
                    if fname_str == "target" || fname_str == ".git" || fname_str == ".cargo" {
                        continue;
                    }
                    stack.push(p);
                } else if fname_str == "Cargo.toml" || fname_str == "build.rs" {
                    if let Ok(rel) = p.strip_prefix(work_dir) {
                        let rel_str = rel.to_string_lossy().to_string();
                        if let Ok(bytes) = fs::read(&p) {
                            hashes.insert(rel_str, ridx_format::fnv1a_64(&bytes));
                        }
                    }
                }
            }
        }
    }

    hashes
}

/// Checks if any build configuration files or build options were modified since the last index.
/// Returns Some(manifest_mtime) if configuration is unchanged, or None to fall back to Cargo.
pub fn verify_build_config_unchanged(
    work_dir: &Path,
    base_out_dir: &Path,
    opts: &IndexOptions,
) -> Option<SystemTime> {
    let manifest_file = base_out_dir.join("ridx_manifest.json");
    let manifest_mtime = fs::metadata(&manifest_file)
        .ok()
        .and_then(|m| m.modified().ok())?;

    let manifest_bytes = match fs::read(&manifest_file) {
        Ok(b) => b,
        Err(_) => {
            eprintln!("[ridx-direct] fallback to Cargo: cannot read ridx_manifest.json");
            return None;
        }
    };
    let val: serde_json::Value = match serde_json::from_slice(&manifest_bytes) {
        Ok(v) => v,
        Err(_) => {
            eprintln!("[ridx-direct] fallback to Cargo: corrupted ridx_manifest.json");
            return None;
        }
    };

    // Check build options match previously published manifest
    let p_feat = val.get("features").and_then(|v| v.as_str());
    let p_nodef = val
        .get("no_default_features")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let p_allfeat = val
        .get("all_features")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let p_target = val.get("target").and_then(|v| v.as_str());
    let p_pkg = val.get("package_filter").and_then(|v| v.as_str());

    if opts.features.as_deref() != p_feat
        || opts.no_default_features != p_nodef
        || opts.all_features != p_allfeat
        || opts.target.as_deref() != p_target
        || opts.package.as_deref() != p_pkg
    {
        eprintln!(
            "[ridx-direct] fallback to Cargo: build options (features/target/package) changed"
        );
        return None;
    }

    let current_hashes = compute_workspace_config_hashes(work_dir);

    // If config_hashes is recorded in manifest, verify hash equality
    if let Some(recorded_obj) = val.get("config_hashes").and_then(|v| v.as_object()) {
        let mut recorded_hashes: HashMap<String, u64> = HashMap::new();
        for (k, v) in recorded_obj {
            if let Some(h) = v.as_u64() {
                recorded_hashes.insert(k.clone(), h);
            }
        }

        if current_hashes != recorded_hashes {
            eprintln!("[ridx-direct] fallback to Cargo: build configuration files (Cargo.toml/Cargo.lock/build.rs) changed");
            return None;
        }
    } else {
        // Fallback for legacy manifests without config_hashes: check mtime
        let config_files = [
            work_dir.join("Cargo.toml"),
            work_dir.join("Cargo.lock"),
            work_dir.join("rust-toolchain"),
            work_dir.join("rust-toolchain.toml"),
            work_dir.join(".cargo/config"),
            work_dir.join(".cargo/config.toml"),
        ];
        for cf in &config_files {
            if cf.is_file() {
                if let Ok(meta) = fs::metadata(cf) {
                    if let Ok(mtime) = meta.modified() {
                        if mtime > manifest_mtime {
                            eprintln!(
                                "[ridx-direct] fallback to Cargo: config file mtime newer than manifest: {}",
                                cf.display()
                            );
                            return None;
                        }
                    }
                }
            }
        }
    }

    Some(manifest_mtime)
}

/// Recursively checks if any `.rs` file in crate root (src, tests, benches, examples, build.rs) is not in `tracked`.
/// Returns true if an untracked `.rs` file was found (triggering fallback to Cargo).
pub fn has_untracked_rs_files(crate_dir: &Path, tracked: &HashSet<String>) -> bool {
    let subdirs = ["src", "tests", "benches", "examples"];
    for sub in &subdirs {
        let dir = crate_dir.join(sub);
        if !dir.is_dir() {
            continue;
        }
        let mut stack = vec![dir];
        while let Some(d) = stack.pop() {
            if let Ok(entries) = fs::read_dir(&d) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    if p.is_dir() {
                        stack.push(p);
                    } else if p.extension().and_then(|s| s.to_str()) == Some("rs") {
                        let p_str = p.display().to_string();
                        let canon_str = p
                            .canonicalize()
                            .map(|cp| cp.display().to_string())
                            .unwrap_or_else(|_| p_str.clone());
                        let rel_str = p
                            .strip_prefix(crate_dir)
                            .ok()
                            .map(|r| r.display().to_string());
                        if !tracked.contains(&p_str)
                            && !tracked.contains(&canon_str)
                            && !rel_str
                                .as_ref()
                                .map(|r| tracked.contains(r))
                                .unwrap_or(false)
                        {
                            eprintln!(
                                "[ridx-direct] fallback to Cargo: untracked .rs file found: {}",
                                p.display()
                            );
                            return true;
                        }
                    }
                }
            }
        }
    }

    let build_rs = crate_dir.join("build.rs");
    if build_rs.is_file() {
        let p_str = build_rs.display().to_string();
        let canon_str = build_rs
            .canonicalize()
            .map(|cp| cp.display().to_string())
            .unwrap_or_else(|_| p_str.clone());
        let rel_str = build_rs
            .strip_prefix(crate_dir)
            .ok()
            .map(|r| r.display().to_string());
        if !tracked.contains(&p_str)
            && !tracked.contains(&canon_str)
            && !rel_str
                .as_ref()
                .map(|r| tracked.contains(r))
                .unwrap_or(false)
        {
            eprintln!(
                "[ridx-direct] fallback to Cargo: untracked build.rs found: {}",
                build_rs.display()
            );
            return true;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_workspace_config_hashes_detects_changes() {
        let tmp = std::env::temp_dir().join(format!("ridx_cfg_hash_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        fs::write(tmp.join("Cargo.toml"), "[workspace]\n").unwrap();
        fs::write(tmp.join("Cargo.lock"), "version = 3\n").unwrap();
        let sub = tmp.join("crates").join("sub");
        fs::create_dir_all(&sub).unwrap();
        fs::write(sub.join("Cargo.toml"), "[package]\nname = \"sub\"\n").unwrap();
        fs::write(sub.join("build.rs"), "fn main() {}\n").unwrap();

        let hashes1 = compute_workspace_config_hashes(&tmp);
        assert!(hashes1.contains_key("Cargo.toml"));
        assert!(hashes1.contains_key("Cargo.lock"));
        assert!(hashes1.contains_key("crates/sub/Cargo.toml"));
        assert!(hashes1.contains_key("crates/sub/build.rs"));

        // Mutate sub/build.rs
        fs::write(
            sub.join("build.rs"),
            "fn main() { println!(\"changed\"); }\n",
        )
        .unwrap();
        let hashes2 = compute_workspace_config_hashes(&tmp);
        assert_ne!(
            hashes1.get("crates/sub/build.rs"),
            hashes2.get("crates/sub/build.rs")
        );
        assert_ne!(hashes1, hashes2);

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_has_untracked_rs_files_in_multiple_target_dirs() {
        let tmp = std::env::temp_dir().join(format!("ridx_untracked_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(tmp.join("src")).unwrap();
        fs::create_dir_all(tmp.join("tests")).unwrap();
        fs::create_dir_all(tmp.join("examples")).unwrap();
        fs::create_dir_all(tmp.join("benches")).unwrap();

        let lib_rs = tmp.join("src/lib.rs");
        let test_rs = tmp.join("tests/test1.rs");
        let ex_rs = tmp.join("examples/ex1.rs");
        let bench_rs = tmp.join("benches/b1.rs");
        let build_rs = tmp.join("build.rs");

        fs::write(&lib_rs, "pub fn f() {}\n").unwrap();
        fs::write(&test_rs, "fn t() {}\n").unwrap();
        fs::write(&ex_rs, "fn main() {}\n").unwrap();
        fs::write(&bench_rs, "fn b() {}\n").unwrap();
        fs::write(&build_rs, "fn main() {}\n").unwrap();

        let mut tracked = HashSet::new();
        tracked.insert(lib_rs.display().to_string());
        tracked.insert(test_rs.display().to_string());
        tracked.insert(ex_rs.display().to_string());
        tracked.insert(bench_rs.display().to_string());
        tracked.insert(build_rs.display().to_string());

        // When all files are tracked, returns false
        assert!(!has_untracked_rs_files(&tmp, &tracked));

        // Untracked file in tests/ triggers fallback
        let untracked_test = tmp.join("tests/new_test.rs");
        fs::write(&untracked_test, "fn new() {}\n").unwrap();
        assert!(has_untracked_rs_files(&tmp, &tracked));
        fs::remove_file(&untracked_test).unwrap();

        // Untracked file in examples/ triggers fallback
        let untracked_ex = tmp.join("examples/new_ex.rs");
        fs::write(&untracked_ex, "fn new_ex() {}\n").unwrap();
        assert!(has_untracked_rs_files(&tmp, &tracked));
        fs::remove_file(&untracked_ex).unwrap();

        let _ = fs::remove_dir_all(&tmp);
    }
}
