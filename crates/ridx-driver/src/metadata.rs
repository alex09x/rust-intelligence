/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use super::*;

pub(crate) fn is_immutable_package(manifest_dir: &Path) -> bool {
    if let Ok(cargo_home) = std::env::var("CARGO_HOME") {
        let ch = Path::new(&cargo_home);
        if manifest_dir.starts_with(ch.join("registry")) || manifest_dir.starts_with(ch.join("git"))
        {
            return true;
        }
    }
    let s = manifest_dir.to_string_lossy();
    s.contains("/.cargo/registry/")
        || s.contains("\\.cargo\\registry\\")
        || s.contains("/.cargo/git/")
        || s.contains("\\.cargo\\git\\")
}

pub(crate) fn collect_file_hashes<'tcx>(
    tcx: TyCtxt<'tcx>,
    package_root: Option<&Path>,
    manifest_dir: Option<&Path>,
) -> std::collections::HashMap<String, u64> {
    let mut hashes = std::collections::HashMap::new();
    let root_scope = package_root.or(manifest_dir);
    let source_map = tcx.sess.source_map();

    for file in source_map.files().iter() {
        let name_str = source_map.filename_for_diagnostics(&file.name).to_string();
        if name_str.starts_with('<') {
            continue;
        }

        if let Some(root) = root_scope {
            let p = Path::new(&name_str);
            let abs_p = if p.is_absolute() {
                p.to_path_buf()
            } else if let Ok(cwd) = std::env::current_dir() {
                cwd.join(p)
            } else {
                p.to_path_buf()
            };
            let abs_canon = abs_p.canonicalize().unwrap_or(abs_p);
            let root_canon = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
            if !abs_canon.starts_with(&root_canon) {
                continue;
            }
        }

        let hash_opt = if let Some(ref src) = file.src {
            Some(ridx_format::normalized_source_hash(src.as_bytes()))
        } else {
            let p = Path::new(&name_str);
            hash_file_contents(p)
                .or_else(|| manifest_dir.and_then(|m| hash_file_contents(&m.join(p))))
        };

        if let Some(h) = hash_opt {
            let p = Path::new(&name_str);
            hashes.insert(name_str.clone(), h);

            if let Ok(canon) = p.canonicalize() {
                hashes.insert(canon.to_string_lossy().to_string(), h);
            }

            if let Some(root) = package_root.or(manifest_dir) {
                if let Ok(rel) = p.strip_prefix(root) {
                    hashes.insert(rel.to_string_lossy().to_string(), h);
                }
                let full = root.join(p);
                if let Ok(canon) = full.canonicalize() {
                    hashes.insert(canon.to_string_lossy().to_string(), h);
                }
            }
        }
    }

    hashes
}

pub(crate) fn write_sidecar_metadata(
    deps_dir: &Path,
    crate_name: &str,
    suffix: &str,
    stable_crate_id: u64,
    dep_info: &serde_json::Value,
) {
    let content = serde_json::to_string_pretty(dep_info).unwrap_or_default();
    // 1. By crate name + suffix (e.g. syn-3d995041387dd824.json or serde.json)
    let _ = fs::write(
        deps_dir.join(format!("{crate_name}{suffix}.json")),
        &content,
    );
    // 2. By crate name + stable crate id (e.g. syn_a1b2c3d4e5f60718.json)
    let _ = fs::write(
        deps_dir.join(format!("{crate_name}_{stable_crate_id:016x}.json")),
        &content,
    );
    // 3. By bare crate name (only if it does not exist yet)
    let bare = deps_dir.join(format!("{crate_name}.json"));
    if !bare.is_file() {
        let _ = fs::write(bare, &content);
    }
}

fn load_target_kind_from_env(entry_path: &Path) -> Option<String> {
    let kinds_file_var = std::env::var("RIDX_TARGET_KINDS_FILE").ok()?;
    let path = PathBuf::from(kinds_file_var);
    if !path.is_file() {
        return None;
    }
    let content = fs::read(&path).ok()?;
    let map: std::collections::HashMap<PathBuf, String> = serde_json::from_slice(&content).ok()?;
    if let Some(kind) = map.get(entry_path) {
        return Some(kind.clone());
    }
    if let Ok(canon) = fs::canonicalize(entry_path) {
        if let Some(kind) = map.get(&canon) {
            return Some(kind.clone());
        }
    }
    None
}

pub(crate) fn determine_target_kind(compiler_args: &[String]) -> ridx_format::TargetKind {
    let entry_rs = compiler_args
        .iter()
        .find(|a| !a.starts_with('-') && a.ends_with(".rs"));

    let base_kind = entry_rs.and_then(|entry| {
        let p = Path::new(entry);
        load_target_kind_from_env(p)
    });

    ridx_format::classify_target(base_kind.as_deref(), compiler_args)
}
