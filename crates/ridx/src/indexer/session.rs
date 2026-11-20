/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! The index of an editing session: the published index of a workspace with the crates of the unsaved buffers
//! compiled again from the buffers' text. Nothing of the published index is written: the session's directory links
//! its shards, and each crate holding a buffer is compiled by `ridx-driver` (the direct path's arguments, recorded in
//! its sidecar) with `RIDX_BUFFERS` naming the buffers, into the session's directory. A crate with errors still gets
//! its shard (see the driver's `broken_crate`).

use super::direct::args::sanitize_compiler_args_for_direct;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Builds in `session_dir` the index of `root` with `buffers` (source path, text) in place of their files, and returns
/// the directory to open it from. Fails when the workspace has no published index or no crate holds a buffer.
pub fn build_session_index(root: &Path, session_dir: &Path, buffers: &[(PathBuf, String)]) -> Result<PathBuf, String> {
    let driver = super::driver::find_driver_binary().ok_or("ridx-driver not found")?;
    build_session_index_with(&driver, root, session_dir, buffers)
}

fn build_session_index_with(driver: &Path, root: &Path, session_dir: &Path, buffers: &[(PathBuf, String)]) -> Result<PathBuf, String> {
    let base = root.join("target/ridx");
    let (variant_rel, _) = super::direct::published_variant(&base).ok_or("no published index")?;
    let published = base.join(&variant_rel);
    let _ = fs::remove_dir_all(session_dir);
    let variant = session_dir.join(&variant_rel);
    fs::create_dir_all(variant.join(".deps")).map_err(|e| format!("session directory: {e}"))?;
    link_files(&published, &variant)?;
    link_files(&published.join(".deps"), &variant.join(".deps"))?;
    fs::copy(base.join("ridx_manifest.json"), session_dir.join("ridx_manifest.json")).map_err(|e| format!("manifest: {e}"))?;

    // The buffers' text, and the list the driver reads.
    let texts = session_dir.join("buffers");
    fs::create_dir_all(&texts).map_err(|e| format!("buffers: {e}"))?;
    let mut list = serde_json::Map::new();
    let mut sources = HashSet::new();
    for (i, (src, text)) in buffers.iter().enumerate() {
        let file = texts.join(format!("{i}.rs"));
        fs::write(&file, text).map_err(|e| format!("buffer: {e}"))?;
        list.insert(src.display().to_string(), file.display().to_string().into());
        sources.insert(canonical(src));
    }
    let list_file = session_dir.join("buffers.json");
    fs::write(&list_file, serde_json::Value::Object(list).to_string()).map_err(|e| format!("buffer list: {e}"))?;

    let units = units_holding(&variant.join(".deps"), &sources, root);
    if units.is_empty() {
        return Err("no indexed crate holds the buffers".into());
    }
    let ld_path = super::cargo::compute_sysroot_ld_path();
    let direct = session_dir.join("direct");
    let (deps, incr) = (direct.join("deps"), direct.join("incremental"));
    fs::create_dir_all(&deps).and_then(|_| fs::create_dir_all(&incr)).map_err(|e| format!("direct directory: {e}"))?;
    let cache_dir = ridx_format::resolve_cache_dir();
    // The units are compiled side by side: on this repository a file of `ridx-format` is in its library (1.7 s) and in
    // its unit tests (2.5 s).
    let mut running = Vec::new();
    for unit in &units {
        // The link to the published shard goes first, so that the driver writes the session's own in its place and
        // never into the published one.
        let shard = unit.object.as_ref().and_then(|o| o.file_name()).map(|name| variant.join(name));
        if let Some(shard) = &shard {
            let _ = fs::remove_file(shard);
        }
        let args = sanitize_compiler_args_for_direct(&unit.args, &deps, &incr, &HashSet::new());
        let mut cmd = Command::new(driver);
        cmd.args(&args).current_dir(root).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        cmd.env("RIDX_OUT_DIR", &variant).env("RIDX_CACHE_DIR", &cache_dir).env("RIDX_WORKSPACE_ROOT", root).env("RIDX_BUFFERS", &list_file).env("RUSTC_BOOTSTRAP", "1");
        let target_kinds = variant.join("target_kinds.json");
        if target_kinds.is_file() {
            cmd.env("RIDX_TARGET_KINDS_FILE", target_kinds);
        }
        cmd.envs(&unit.env);
        if let Some(ld) = &ld_path {
            cmd.env("LD_LIBRARY_PATH", ld).env("DYLD_FALLBACK_LIBRARY_PATH", ld);
        }
        running.push((unit, shard, cmd.spawn().map_err(|e| format!("ridx-driver: {e}"))?));
    }
    for (unit, shard, mut child) in running {
        // A failed compile is the normal case while typing: what decides is whether the driver wrote the shard.
        let _ = child.wait();
        match (&shard, &unit.object) {
            // None (a parse error stops the compiler before analysis): the unit keeps its published shard.
            (Some(shard), Some(published)) if !shard.is_file() => {
                if fs::hard_link(published, shard).is_err() {
                    let _ = fs::copy(published, shard);
                }
            }
            _ => super::direct::units::refresh_aliases(&unit.sidecar, &unit.aliases),
        }
    }
    Ok(session_dir.to_path_buf())
}

/// A compilation unit of the published index whose sources include one of the buffers.
struct Unit {
    args: Vec<String>,
    env: HashMap<String, String>,
    object: Option<PathBuf>,
    sidecar: PathBuf,
    aliases: Vec<PathBuf>,
}

/// The workspace units (not dependencies) of the sidecars in `deps` (of the workspace at `root`) that own one of `sources`,
/// once per object: a unit's sidecar has up to three names, the one named after the object describes it.
fn units_holding(deps: &Path, sources: &HashSet<PathBuf>, root: &Path) -> Vec<Unit> {
    let mut by_object: HashMap<PathBuf, Unit> = HashMap::new();
    let mut entries: Vec<PathBuf> = fs::read_dir(deps).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
    entries.sort();
    for path in entries {
        let Some(j) = fs::read(&path).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()) else { continue };
        if j["is_dependency"].as_bool() == Some(true) {
            continue;
        }
        let holds = j["file_hashes"].as_object().is_some_and(|files| files.keys().any(|f| sources.contains(&canonical(Path::new(f)))));
        let args: Vec<String> = j["compiler_args"].as_array().into_iter().flatten().filter_map(|a| a.as_str().map(str::to_string)).collect();
        if !holds || args.is_empty() {
            continue;
        }
        let object = j["object_path"].as_str().map(PathBuf::from);
        let key = object.clone().unwrap_or_else(|| path.clone());
        let named_after_object = object.as_ref().and_then(|o| o.file_stem()) == path.file_stem();
        match by_object.get_mut(&key) {
            Some(unit) if named_after_object => {
                let alias = std::mem::replace(&mut unit.sidecar, path.clone());
                unit.aliases.push(alias);
                unit.args = args;
            }
            Some(unit) => unit.aliases.push(path.clone()),
            None => {
                let env = j["env"].as_object().into_iter().flatten().filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string()))).collect();
                by_object.insert(key, Unit { args, env, object, sidecar: path.clone(), aliases: Vec::new() });
            }
        }
    }
    // A unit records the sources of the workspace crates it depends on too: on this repository a file of
    // `ridx-format` was in 68 units (each integration test of the package among them) and its session took 6 s. A
    // source belongs to the units whose crate root lies in the innermost directory holding it (`src/lib.rs` and
    // `src/main.rs` own `src/`, `tests/a.rs` does not).
    let root_dir = |u: &Unit| crate_root(u, root).and_then(|r| r.parent().map(Path::to_path_buf));
    let owner_of = |src: &Path| by_object.values().filter_map(root_dir).filter(|d| src.starts_with(d)).max_by_key(|d| d.as_os_str().len());
    let owners: HashSet<PathBuf> = sources.iter().filter_map(|s| owner_of(s)).collect();
    // A source outside every crate root's directory (`#[path]`) keeps all the units recording it.
    let unowned = sources.iter().any(|s| owner_of(s).is_none());
    let mut units: Vec<Unit> = by_object.into_values().filter(|u| unowned || root_dir(u).is_none_or(|d| owners.contains(&d))).collect();
    units.sort_by(|a, b| a.sidecar.cmp(&b.sidecar));
    units
}

/// The unit's crate root file: the first of its compiler arguments naming an existing `.rs` file, relative to the
/// workspace root (where Cargo runs rustc) unless absolute.
fn crate_root(unit: &Unit, root: &Path) -> Option<PathBuf> {
    unit.args
        .iter()
        .skip(1)
        .filter(|a| a.ends_with(".rs") && !a.starts_with('-'))
        .map(|a| if Path::new(a).is_absolute() { PathBuf::from(a) } else { root.join(a) })
        .find(|p| p.is_file())
        .map(|p| canonical(&p))
}

/// Puts every file of `from` into `to`, never its subdirectories: a shard as a link (they are large and only ever
/// replaced, never written in place), anything else as a copy, since the driver rewrites sidecars in place and a
/// link would carry that into the published index.
fn link_files(from: &Path, to: &Path) -> Result<(), String> {
    for entry in fs::read_dir(from).map_err(|e| format!("{}: {e}", from.display()))?.flatten() {
        let path = entry.path();
        if path.is_file() {
            let dest = to.join(entry.file_name());
            let is_shard = path.extension().is_some_and(|x| x == "ridx");
            if !is_shard || fs::hard_link(&path, &dest).is_err() {
                fs::copy(&path, &dest).map_err(|e| format!("{}: {e}", path.display()))?;
            }
        }
    }
    Ok(())
}

fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
