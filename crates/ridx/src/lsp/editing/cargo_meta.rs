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

/// By the nearest manifest: the text of every manifest above it, the manifests Cargo's answer read with their text
/// (a workspace's elsewhere, `package.workspace = "../control"`: review of b854363f), and the answer.
type Answers = HashMap<PathBuf, (String, Vec<(PathBuf, String)>, Option<Arc<Value>>)>;

/// The manifests an answer of Cargo rests on: its workspace's and every package's.
fn read_manifests(v: &Value) -> Vec<(PathBuf, String)> {
    let root = v["workspace_root"].as_str().map(|r| Path::new(r).join("Cargo.toml"));
    let packages = v["packages"].as_array().into_iter().flatten().filter_map(|p| p["manifest_path"].as_str().map(PathBuf::from));
    root.into_iter().chain(packages).map(|m| (m.clone(), std::fs::read_to_string(&m).unwrap_or_default())).collect()
}

/// `cargo metadata` of the nearest manifest above `file`, cached until the text of a manifest above the file, or of one
/// the answer read, changes; `None` with no manifest, or where Cargo gives no answer.
fn metadata(file: &Path) -> Option<Arc<Value>> {
    static CACHE: OnceLock<Mutex<Answers>> = OnceLock::new();
    let manifests: Vec<PathBuf> = file.ancestors().skip(1).map(|d| d.join("Cargo.toml")).filter(|m| m.is_file()).collect();
    let nearest = manifests.first()?.clone();
    let key: String = manifests.iter().map(|m| std::fs::read_to_string(m).unwrap_or_default()).collect::<Vec<_>>().join("\u{0}");
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some((k, read, v)) = cache.lock().ok()?.get(&nearest) {
        if *k == key && read.iter().all(|(m, text)| std::fs::read_to_string(m).unwrap_or_default() == *text) {
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
    // A failure is not kept: a member missing now may be there on the next request (review of c5dee55b).
    let v = Arc::new(serde_json::from_slice::<Value>(&out.stdout).ok().filter(|_| out.status.success())?);
    let read = read_manifests(&v);
    cache.lock().ok()?.insert(nearest, (key, read, Some(v.clone())));
    Some(v)
}

/// The root files of the targets of the package holding `file` (`src/lib.rs`, `src/main.rs`, ..): where crate-level
/// declarations such as `extern crate` are.
pub(crate) fn target_roots(file: &Path) -> Option<Vec<PathBuf>> {
    let j = metadata(file)?;
    let file = ridx_format::canonicalize_best_effort(file);
    let package = j["packages"]
        .as_array()?
        .iter()
        .filter_map(|p| {
            let dir = ridx_format::canonicalize_best_effort(Path::new(p["manifest_path"].as_str()?).parent()?);
            file.starts_with(&dir).then(|| (dir.components().count(), p))
        })
        .max_by_key(|(depth, _)| *depth)?
        .1;
    Some(package["targets"].as_array()?.iter().filter_map(|t| t["src_path"].as_str().map(PathBuf::from)).collect())
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

/// Where the Cargo workspace holding a file keeps its sources, as Cargo resolves them (canonical).
pub(crate) struct Layout {
    pub root: PathBuf,
    pub target_dir: PathBuf,
    /// Every member's directory, outside the root too (`package.workspace = "../control"`).
    pub packages: Vec<PathBuf>,
    /// The directory of every target's root file, a declared hidden one too (`path = ".cases/caller.rs"`).
    pub targets: Vec<PathBuf>,
}

/// The layout of the Cargo workspace holding `file`, `Err` when a manifest is above it and Cargo gives no answer,
/// `Ok(None)` with no manifest at all.
pub(crate) fn layout(file: &Path) -> Result<Option<Layout>, ()> {
    if !file.ancestors().skip(1).any(|d| d.join("Cargo.toml").is_file()) {
        return Ok(None);
    }
    let j = metadata(file).ok_or(())?;
    let canon = |s: &str| ridx_format::canonicalize_best_effort(Path::new(s));
    let packages = j["packages"].as_array().ok_or(())?;
    Ok(Some(Layout {
        root: j["workspace_root"].as_str().map(canon).ok_or(())?,
        target_dir: j["target_directory"].as_str().map(canon).ok_or(())?,
        packages: packages.iter().filter_map(|p| p["manifest_path"].as_str().and_then(|m| canon(m).parent().map(Path::to_path_buf))).collect(),
        targets: packages
            .iter()
            .flat_map(|p| p["targets"].as_array().into_iter().flatten())
            .filter_map(|t| t["src_path"].as_str().and_then(|s| canon(s).parent().map(Path::to_path_buf)))
            .collect(),
    }))
}
