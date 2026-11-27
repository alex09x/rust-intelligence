/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Compilation units behind the sidecars of `.deps`. The driver writes a unit's sidecar under up to three names
//! (`name-<extra-filename>.json`, `name_<stable crate id>.json`, and `name.json` for the first unit of that name), all
//! naming the same `object_path`; the fast path compiles each unit once and keeps all its names current.

use super::WorkspaceCrateEntry;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

/// The dependency shards and the workspace units the sidecars of `deps_dir` describe (see [`group_by_unit`]); `None`
/// when one cannot be read, or a workspace sidecar records no compiler arguments.
#[allow(clippy::type_complexity)]
pub(super) fn read_units(deps_dir: &Path, work_dir: &Path) -> Option<(Vec<ridx_format::DependencyShardInfo>, Vec<WorkspaceCrateEntry>, HashMap<String, Vec<PathBuf>>)> {
    let mut dep_shards = Vec::new();
    let mut sidecars: Vec<(WorkspaceCrateEntry, Option<String>)> = Vec::new();
    for entry in fs::read_dir(deps_dir).ok()?.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let val: serde_json::Value = serde_json::from_slice(&fs::read(&path).ok()?).ok()?;
        if val.get("is_dependency").and_then(|v| v.as_bool()).unwrap_or(false) {
            if let Ok(info) = serde_json::from_value::<ridx_format::DependencyShardInfo>(val) {
                dep_shards.push(info);
            }
            continue;
        }
        let crate_name = val.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let args: Vec<String> =
            val.get("compiler_args").and_then(|v| v.as_array()).map(|arr| arr.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default();
        if args.is_empty() {
            eprintln!("[ridx-direct] fallback to Cargo: compiler args missing for crate '{crate_name}'");
            return None;
        }
        let file_hashes: HashMap<String, u64> =
            val.get("file_hashes").and_then(|v| v.as_object()).map(|obj| obj.iter().filter_map(|(k, v)| v.as_u64().map(|h| (k.clone(), h))).collect()).unwrap_or_default();
        let file_hashes = own_file_hashes(file_hashes, val.get("own_sources"), work_dir);
        let env_vars: HashMap<String, String> =
            val.get("env").and_then(|v| v.as_object()).map(|obj| obj.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect()).unwrap_or_default();
        let object = val.get("object_path").and_then(|v| v.as_str()).map(str::to_string);
        let target_id = path.file_stem().and_then(|s| s.to_str()).unwrap_or(&crate_name).to_string();
        sidecars.push(((target_id, crate_name, args, env_vars, file_hashes, path), object));
    }
    // On BTCR an edit of one file compiled 53 sidecars, each of the ~20 units two or three times over.
    let (units, aliases) = group_by_unit(sidecars);
    Some((dep_shards, units, aliases))
}

/// One entry per unit, and the other sidecars naming it, by the unit's target id. The unit is described by the sidecar
/// named after its object (`name-<extra>.json` for `name-<extra>.ridx`), the one the driver always rewrites; the bare
/// `name.json` is written only once and goes stale after a direct rebuild. Entries without an object stay apart.
pub(super) fn group_by_unit(entries: Vec<(WorkspaceCrateEntry, Option<String>)>) -> (Vec<WorkspaceCrateEntry>, HashMap<String, Vec<PathBuf>>) {
    let mut by_object: BTreeMap<String, Vec<WorkspaceCrateEntry>> = BTreeMap::new();
    let mut units = Vec::new();
    for (entry, object) in entries {
        match object {
            Some(o) => by_object.entry(o).or_default().push(entry),
            None => units.push(entry),
        }
    }
    let mut aliases = HashMap::new();
    for (object, mut group) in by_object {
        let object_stem = Path::new(&object).file_stem().map(|s| s.to_string_lossy().to_string());
        // Without the sidecar named after the object, the bare `name.json` (the one the driver never rewrites) is the
        // last choice (review of 3690a98d: by path it sorted first, and its stale text was copied over the rest).
        group.sort_by(|a, b| (a.0 == a.1, &a.5).cmp(&(b.0 == b.1, &b.5)));
        let primary = group.iter().position(|e| Some(&e.0) == object_stem.as_ref()).unwrap_or(0);
        let entry = group.remove(primary);
        aliases.insert(entry.0.clone(), group.into_iter().map(|e| e.5).collect());
        units.push(entry);
    }
    (units, aliases)
}

/// Copies the unit's freshly written sidecar over its other names, so none keeps the hashes of the text before.
pub(crate) fn refresh_aliases(primary: &Path, aliases: &[PathBuf]) {
    let Ok(bytes) = fs::read(primary) else { return };
    for alias in aliases {
        let tmp = alias.with_extension("json.tmp");
        if fs::write(&tmp, &bytes).is_ok() && fs::rename(&tmp, alias).is_err() {
            let _ = fs::remove_file(&tmp);
        }
    }
}

/// The raw and the normalized hash of a file's text.
pub(super) fn source_hashes(bytes: &[u8]) -> (u64, u64) {
    (ridx_format::fnv1a_64(bytes), ridx_format::normalized_source_hash(bytes))
}

/// Whether a text with the hashes `now` still has the hash recorded for it: the driver records the normalized hash of
/// the text it read (`normalized_source_hash`), older sidecars the raw one.
pub(super) fn hash_matches(now: (u64, u64), recorded: u64) -> bool {
    now.0 == recorded || now.1 == recorded
}

/// The hashes of the unit's own sources as the sidecar at `sidecar` records them now (see [`own_file_hashes`]).
pub(super) fn recorded_own_hashes(sidecar: &Path, work_dir: &Path) -> Option<HashMap<String, u64>> {
    let val: serde_json::Value = serde_json::from_slice(&fs::read(sidecar).ok()?).ok()?;
    let hashes = val.get("file_hashes")?.as_object()?.iter().filter_map(|(k, v)| v.as_u64().map(|h| (k.clone(), h))).collect();
    Some(own_file_hashes(hashes, val.get("own_sources"), work_dir))
}

/// The recorded hashes of the unit's own sources (`own_sources`, canonical paths): `file_hashes` also holds the files
/// of the workspace crates it imports, as they were when it was compiled, and a unit left out behind a dependency with
/// errors keeps those of the text before. A file that no longer exists stays, as a change. A sidecar without
/// `own_sources` (older drivers) keeps all.
pub(super) fn own_file_hashes(file_hashes: HashMap<String, u64>, own_sources: Option<&serde_json::Value>, work_dir: &Path) -> HashMap<String, u64> {
    let own: HashSet<&str> = own_sources.and_then(|v| v.as_array()).into_iter().flatten().filter_map(|v| v.as_str()).collect();
    if own.is_empty() {
        return file_hashes;
    }
    file_hashes
        .into_iter()
        .filter(|(file, _)| {
            let path = work_dir.join(file);
            path.canonicalize().map_or(true, |canon| own.contains(canon.to_string_lossy().as_ref()))
        })
        .collect()
}
