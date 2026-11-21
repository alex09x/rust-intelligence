/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! What Cargo resolves for the package holding a file (`cargo metadata --no-deps --offline`), read as Cargo reads its
//! manifests, never by matching their lines: the edition, the workspace's root.

use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

/// By the nearest manifest: the text of every manifest above it, and what Cargo answered for it.
type Answers = HashMap<PathBuf, (String, Option<Arc<Value>>)>;

/// `cargo metadata` of the nearest manifest above `file`, cached until the text of a manifest above the file changes;
/// `None` with no manifest, or where Cargo gives no answer.
fn metadata(file: &Path) -> Option<Arc<Value>> {
    static CACHE: OnceLock<Mutex<Answers>> = OnceLock::new();
    let manifests: Vec<PathBuf> = file.ancestors().skip(1).map(|d| d.join("Cargo.toml")).filter(|m| m.is_file()).collect();
    let nearest = manifests.first()?.clone();
    let key: String = manifests.iter().map(|m| std::fs::read_to_string(m).unwrap_or_default()).collect::<Vec<_>>().join("\u{0}");
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some((k, v)) = cache.lock().ok()?.get(&nearest) {
        if *k == key {
            return v.clone();
        }
    }
    let out = std::process::Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--no-deps", "--offline", "--manifest-path"])
        .arg(&nearest)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    let v = serde_json::from_slice::<Value>(&out.stdout).ok().filter(|_| out.status.success()).map(Arc::new);
    cache.lock().ok()?.insert(nearest, (key, v.clone()));
    v
}

/// The edition of the package holding `file`: of the package whose directory holds it most closely.
pub(crate) fn package_edition(file: &Path) -> Option<String> {
    let j = metadata(file)?;
    let file = ridx_format::canonicalize_best_effort(file);
    j["packages"]
        .as_array()?
        .iter()
        .filter_map(|p| {
            let dir = ridx_format::canonicalize_best_effort(Path::new(p["manifest_path"].as_str()?).parent()?);
            let edition = p["edition"].as_str()?;
            file.starts_with(&dir).then(|| (dir.components().count(), edition.to_string()))
        })
        .max_by_key(|(depth, _)| *depth)
        .map(|(_, e)| e)
}

/// The root of the Cargo workspace holding `file` and its build directory (where ridx's indexes are written too),
/// `Err` when a manifest is above it and Cargo gives no answer, `Ok(None)` with no manifest at all.
pub(crate) fn workspace_root(file: &Path) -> Result<Option<(PathBuf, PathBuf)>, ()> {
    if !file.ancestors().skip(1).any(|d| d.join("Cargo.toml").is_file()) {
        return Ok(None);
    }
    let j = metadata(file).ok_or(())?;
    let dir = |k: &str| j[k].as_str().map(|r| ridx_format::canonicalize_best_effort(Path::new(r)));
    dir("workspace_root").zip(dir("target_directory")).map(Some).ok_or(())
}
