/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! What a variant keeps after a check: the units it rebuilt replace their earlier shards and sidecars, and, when the
//! check failed, a unit it did not rebuild keeps its last good ones (set aside in `.stale` by the invalidation, or
//! those of the variant active before). A unit is a sidecar's crate name, target kind and source root, so a
//! package's lib and bin, or two versions of one dependency, never stand in for each other.

use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

type Unit = (String, String, String);

/// The unit a sidecar describes.
fn unit_of(j: &Value) -> Option<Unit> {
    Some((j["name"].as_str()?.to_string(), j["target_kind"].as_str().unwrap_or_default().to_string(), j["source_root"].as_str().unwrap_or_default().to_string()))
}

fn object_of(j: &Value) -> Option<PathBuf> {
    j["object_path"].as_str().map(PathBuf::from)
}

/// The sidecars of `deps`: path, contents, and whether written at or after `started`.
fn sidecars(deps: &Path, started: SystemTime) -> Vec<(PathBuf, Value, bool)> {
    let mut out: Vec<(PathBuf, Value, bool)> = fs::read_dir(deps)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .filter_map(|p| {
            let j = serde_json::from_slice::<Value>(&fs::read(&p).ok()?).ok()?;
            let fresh = fs::metadata(&p).and_then(|m| m.modified()).is_ok_and(|t| t >= started);
            Some((p, j, fresh))
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Settles `dir` after a check that started at `started`: superseded units go, and when it failed (`!ok`) the units
/// it did not rebuild come back from `stale` (the sidecars the invalidation set aside), else from `previous`.
pub(super) fn settle_after_check(dir: &Path, started: SystemTime, ok: bool, stale: &Path, previous: Option<&Path>) {
    let deps = dir.join(".deps");
    let current = sidecars(&deps, started);
    // The object each unit rebuilt by this check now has.
    let rebuilt: HashMap<Unit, PathBuf> = current.iter().filter(|s| s.2).filter_map(|(_, j, _)| Some((unit_of(j)?, object_of(j)?))).collect();
    let mut dropped_objects = Vec::new();
    // A sidecar of a rebuilt unit pointing elsewhere is superseded, with its shard.
    for (path, j, fresh) in &current {
        let (Some(unit), Some(obj)) = (unit_of(j), object_of(j)) else { continue };
        if !fresh && rebuilt.get(&unit).is_some_and(|now| *now != obj) {
            let _ = fs::remove_file(path);
            dropped_objects.push(obj);
        }
    }
    let mut kept: HashSet<Unit> = current.iter().filter(|s| s.0.exists()).filter_map(|s| unit_of(&s.1)).collect();
    for (path, j, _) in sidecars(&stale.join(".deps"), started) {
        let (Some(unit), Some(obj)) = (unit_of(&j), object_of(&j)) else { continue };
        let back = deps.join(path.file_name().unwrap_or_default());
        if ok || rebuilt.contains_key(&unit) || !obj.is_file() || back.exists() {
            if rebuilt.get(&unit) != Some(&obj) {
                dropped_objects.push(obj);
            }
            continue;
        }
        if fs::rename(&path, &back).is_ok() {
            kept.insert(unit);
        }
    }
    if !ok {
        if let Some(prev) = previous {
            adopt_previous(dir, prev, &kept);
        }
    }
    // A shard of this variant that no sidecar names any more goes (a dependency's object stays in the shared cache).
    let named: HashSet<PathBuf> = sidecars(&deps, started).iter().filter_map(|s| object_of(&s.1)).collect();
    for obj in dropped_objects {
        if obj.parent() == Some(dir) && !named.contains(&obj) {
            let _ = fs::remove_file(obj);
        }
    }
}

/// The units of `prev` that `dir` has none of, with their sidecars: a shard of `prev` is copied into `dir`, a
/// dependency's sidecar names its object in the shared cache as it is.
fn adopt_previous(dir: &Path, prev: &Path, kept: &HashSet<Unit>) {
    let deps = dir.join(".deps");
    let _ = fs::create_dir_all(&deps);
    for (path, mut j, _) in sidecars(&prev.join(".deps"), SystemTime::UNIX_EPOCH) {
        let (Some(unit), Some(obj)) = (unit_of(&j), object_of(&j)) else { continue };
        let to = deps.join(path.file_name().unwrap_or_default());
        if kept.contains(&unit) || !obj.is_file() || to.exists() {
            continue;
        }
        if obj.parent() == Some(prev) {
            let Some(name) = obj.file_name() else { continue };
            let copy = dir.join(name);
            if !copy.is_file() && fs::copy(&obj, &copy).is_err() {
                continue;
            }
            j["object_path"] = serde_json::json!(copy.display().to_string());
        }
        if let Ok(b) = serde_json::to_vec(&j) {
            let _ = fs::write(&to, b);
        }
    }
}
