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
    is_immutable_package_with_home(manifest_dir, std::env::var("CARGO_HOME").ok().as_deref().map(Path::new))
        || built_as_non_local_package(std::env::args(), std::env::var_os("CARGO_PRIMARY_PACKAGE").is_some())
}

/// Cargo caps lints only for packages that are not local (registry, git, or a source cache a build tool
/// unpacked them into), and marks the packages selected for the build as primary: a capped, non-primary
/// compilation is a dependency's, wherever its sources were unpacked.
pub(crate) fn built_as_non_local_package(compiler_args: impl IntoIterator<Item = String>, primary_package: bool) -> bool {
    !primary_package && compiler_args.into_iter().any(|a| a == "--cap-lints" || a.starts_with("--cap-lints="))
}

pub(crate) fn is_immutable_package_with_home(manifest_dir: &Path, cargo_home: Option<&Path>) -> bool {
    if let Some(ch) = cargo_home {
        if manifest_dir.starts_with(ch.join("registry")) || manifest_dir.starts_with(ch.join("git")) {
            return true;
        }
    }
    // One separator: a Windows path can mix them.
    let s = manifest_dir.to_string_lossy().replace('\\', "/");
    s.contains("/.cargo/registry/") || s.contains("/.cargo/git/")
}

/// The source files this crate itself was compiled from (canonical paths): the local crate's files in the source map,
/// not those imported from its dependencies' metadata, which `file_hashes` records too. A session index compiles a
/// buffer in the units listing it here (review of ca0c1ec7: a file included by `#[path]` from another package).
pub(crate) fn own_source_files(tcx: TyCtxt<'_>) -> Vec<String> {
    let cwd = std::env::current_dir().unwrap_or_default();
    let mut own: Vec<String> = tcx
        .sess
        .source_map()
        .files()
        .iter()
        .filter(|f| f.cnum == rustc_span::def_id::LOCAL_CRATE)
        // The file's path on this machine, not its name for diagnostics, which `--remap-path-prefix` rewrites (review
        // of 11310378: `/virtual-source/lib.rs` recorded instead of the checkout's file).
        .filter_map(|f| match &f.name {
            rustc_span::FileName::Real(real) => real.local_path().map(|p| cwd.join(p)),
            _ => None,
        })
        .map(|p| p.canonicalize().unwrap_or(p).display().to_string())
        .collect();
    own.sort();
    own.dedup();
    own
}

pub(crate) fn collect_file_hashes<'tcx>(tcx: TyCtxt<'tcx>, package_root: Option<&Path>, manifest_dir: Option<&Path>) -> std::collections::HashMap<String, u64> {
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
            hash_file_contents(p).or_else(|| manifest_dir.and_then(|m| hash_file_contents(&m.join(p))))
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

fn write_json_atomic(deps_dir: &Path, filename: &str, content: &str, overwrite: bool) {
    let dest = deps_dir.join(filename);
    if !overwrite && dest.is_file() {
        return;
    }
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let tmp = deps_dir.join(format!(".tmp_{}_{nonce}_{filename}", std::process::id()));
    if fs::write(&tmp, content).is_err() {
        return;
    }
    if overwrite {
        let _ = fs::rename(&tmp, &dest);
    } else {
        if fs::hard_link(&tmp, &dest).is_err() && !dest.is_file() {
            let _ = fs::rename(&tmp, &dest);
        }
        let _ = fs::remove_file(&tmp);
    }
}

pub(crate) fn write_sidecar_metadata(deps_dir: &Path, crate_name: &str, suffix: &str, stable_crate_id: u64, dep_info: &serde_json::Value) {
    let content = serde_json::to_string_pretty(dep_info).unwrap_or_default();
    // 1. By crate name + suffix (e.g. syn-3d995041387dd824.json or serde.json)
    write_json_atomic(deps_dir, &format!("{crate_name}{suffix}.json"), &content, true);
    // 2. By crate name + stable crate id (e.g. syn_a1b2c3d4e5f60718.json)
    write_json_atomic(deps_dir, &format!("{crate_name}_{stable_crate_id:016x}.json"), &content, true);
    // 3. By bare crate name (only if it does not exist yet — atomic link avoids torn concurrent writes)
    write_json_atomic(deps_dir, &format!("{crate_name}.json"), &content, false);
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
    let entry_rs = compiler_args.iter().find(|a| !a.starts_with('-') && a.ends_with(".rs"));

    let base_kind = entry_rs.and_then(|entry| {
        let p = Path::new(entry);
        load_target_kind_from_env(p)
    });

    ridx_format::classify_target(base_kind.as_deref(), compiler_args)
}
