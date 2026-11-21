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

/// What Cargo reads of a package: its manifest's text, and the entries of the directories it discovers targets in
/// (`src/main.rs` added makes another target with no manifest changed: review of 8b38fdc2).
fn fingerprint(manifest: &Path) -> String {
    let mut out = std::fs::read_to_string(manifest).unwrap_or_default();
    if let Some(dir) = manifest.parent() {
        for sub in ["src", "src/bin", "examples", "tests", "benches"] {
            let mut names: Vec<String> = std::fs::read_dir(dir.join(sub)).into_iter().flatten().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
            names.sort();
            out.push_str(&format!("\u{0}{sub}:{}", names.join("/")));
        }
    }
    out
}

/// The manifests an answer of Cargo rests on, its workspace's and every package's, with their fingerprints.
fn read_manifests(v: &Value) -> Vec<(PathBuf, String)> {
    let root = v["workspace_root"].as_str().map(|r| Path::new(r).join("Cargo.toml"));
    let packages = v["packages"].as_array().into_iter().flatten().filter_map(|p| p["manifest_path"].as_str().map(PathBuf::from));
    root.into_iter().chain(packages).map(|m| (m.clone(), fingerprint(&m))).collect()
}

/// One run of `cargo metadata` for `manifest`; `None` where Cargo gives no answer.
fn run(manifest: &Path) -> Option<Value> {
    let out = std::process::Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--no-deps", "--offline", "--manifest-path"])
        .arg(manifest)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    serde_json::from_slice::<Value>(&out.stdout).ok().filter(|_| out.status.success())
}

/// `cargo metadata` of the nearest manifest above `file`, cached until the fingerprint of a manifest above the file, or
/// of one the answer rests on, changes; `None` with no manifest, or where Cargo gives no answer (not kept: a member
/// missing now may be there on the next request, review of c5dee55b).
fn metadata(file: &Path) -> Option<Arc<Value>> {
    static CACHE: OnceLock<Mutex<Answers>> = OnceLock::new();
    let manifests: Vec<PathBuf> = file.ancestors().skip(1).map(|d| d.join("Cargo.toml")).filter(|m| m.is_file()).collect();
    let nearest = manifests.first()?.clone();
    let key: String = manifests.iter().map(|m| fingerprint(m)).collect::<Vec<_>>().join("\u{1}");
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some((k, read, v)) = cache.lock().ok()?.get(&nearest) {
        if *k == key && read.iter().all(|(m, fp)| fingerprint(m) == *fp) {
            return v.clone();
        }
    }
    // The manifests an answer rests on are known from a first run; their fingerprints are taken before the run whose
    // answer is kept, so a change after it is seen on the next request, never paired with an older answer (review of
    // 8b38fdc2).
    let first = run(&nearest)?;
    let read = read_manifests(&first);
    let v = Arc::new(run(&nearest)?);
    // A manifest the kept answer rests on that the fingerprints miss (a member added between the runs): not kept.
    let tracked = read_manifests(&v).iter().all(|(m, _)| read.iter().any(|(r, _)| r == m));
    if tracked {
        cache.lock().ok()?.insert(nearest, (key, read, Some(v.clone())));
    }
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
