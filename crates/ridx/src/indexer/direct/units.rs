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
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

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
pub(super) fn refresh_aliases(primary: &Path, aliases: &[PathBuf]) {
    let Ok(bytes) = fs::read(primary) else { return };
    for alias in aliases {
        let tmp = alias.with_extension("json.tmp");
        if fs::write(&tmp, &bytes).is_ok() && fs::rename(&tmp, alias).is_err() {
            let _ = fs::remove_file(&tmp);
        }
    }
}

/// Whether `bytes` still have the hash recorded for them: the driver records the normalized hash of the text it read
/// (`normalized_source_hash`), older sidecars the raw one.
pub(super) fn hash_matches(bytes: &[u8], recorded: u64) -> bool {
    ridx_format::fnv1a_64(bytes) == recorded || ridx_format::normalized_source_hash(bytes) == recorded
}
