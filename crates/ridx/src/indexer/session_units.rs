/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Which compilation units a session index compiles again: those that own an edited buffer.

use super::session::canonical;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

/// A compilation unit of the published index whose sources include one of the buffers.
pub(super) struct Unit {
    pub(super) name: String,
    pub(super) args: Vec<String>,
    pub(super) env: HashMap<String, String>,
    pub(super) object: Option<PathBuf>,
    pub(super) sidecar: PathBuf,
    pub(super) aliases: Vec<PathBuf>,
    /// The sources the unit was compiled from itself (`own_sources` of the sidecar), when the driver recorded them.
    own: Option<HashSet<PathBuf>>,
}

/// The `own_sources` a sidecar records (canonical), if it does.
fn own_sources_of(sidecar: &serde_json::Value) -> Option<HashSet<PathBuf>> {
    sidecar["own_sources"].as_array().map(|a| a.iter().filter_map(|s| s.as_str()).map(|s| canonical(Path::new(s))).collect())
}

/// The sources the sidecar at `path` records, if it does.
pub(super) fn sources_recorded(path: &Path) -> Option<HashSet<PathBuf>> {
    fs::read(path).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()).as_ref().and_then(own_sources_of)
}

impl Unit {
    /// A library unit, whose metadata other units read (not a test harness, a binary or a build script).
    pub(super) fn is_lib(&self) -> bool {
        !self.args.iter().any(|a| a == "--test") && crate_types(&self.args).iter().all(|t| matches!(t.as_str(), "lib" | "rlib" | "dylib" | "proc-macro"))
    }

    /// The name of the metadata file it writes, without extension: `lib<crate>` and its `-C extra-filename`.
    pub(super) fn artifact_stem(&self) -> String {
        let mut extra = "";
        let mut args = self.args.iter();
        while let Some(a) = args.next() {
            let opt = if a == "-C" { args.next().map(String::as_str) } else { a.strip_prefix("-C") };
            if let Some(e) = opt.and_then(|o| o.strip_prefix("extra-filename=")) {
                extra = e;
            }
        }
        format!("lib{}{extra}", self.name)
    }

    /// Its `--extern` arguments: the name it binds (as `rewrite_extern_arg` reads it), the stem of the file bound, and
    /// that file's name.
    pub(super) fn extern_edges(&self) -> Vec<(String, String, String)> {
        let mut out = Vec::new();
        let mut args = self.args.iter();
        while let Some(a) = args.next() {
            let spec = if a == "--extern" { args.next().map(String::as_str) } else { a.strip_prefix("--extern=") };
            if let Some((prefix, path)) = spec.and_then(|s| s.split_once('=')) {
                let binding = prefix.rsplit(':').next().unwrap_or(prefix).to_string();
                let file = Path::new(path);
                if let (Some(stem), Some(name)) = (file.file_stem(), file.file_name()) {
                    out.push((binding, stem.to_string_lossy().into_owned(), name.to_string_lossy().into_owned()));
                }
            }
        }
        out
    }
}

/// `args` with the file of each `--extern` replaced by `replace(file)` when it gives one, every other argument (and
/// every other extern, even of the same binding) as it is.
pub(super) fn rebind_externs(args: &[String], replace: impl Fn(&str) -> Option<PathBuf>) -> Vec<String> {
    let rebind = |spec: &str| match spec.split_once('=') {
        Some((prefix, file)) => replace(file).map_or_else(|| spec.to_string(), |new| format!("{prefix}={}", new.display())),
        None => spec.to_string(),
    };
    let mut out = Vec::with_capacity(args.len());
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--extern" {
            out.push(a.clone());
            if let Some(spec) = it.next() {
                out.push(rebind(spec));
            }
        } else if let Some(spec) = a.strip_prefix("--extern=") {
            out.push(format!("--extern={}", rebind(spec)));
        } else {
            out.push(a.clone());
        }
    }
    out
}

fn crate_types(args: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--crate-type" {
            out.extend(it.next().cloned());
        } else if let Some(t) = a.strip_prefix("--crate-type=") {
            out.push(t.to_string());
        }
    }
    out
}

fn crate_name(args: &[String], env: &HashMap<String, String>) -> String {
    env.get("CARGO_CRATE_NAME").cloned().or_else(|| args.windows(2).find(|w| w[0] == "--crate-name").map(|w| w[1].clone())).unwrap_or_default()
}

/// The workspace units (not dependencies) of the sidecars in `deps` (of the workspace at `root`) that own one of
/// `sources`, once per object: a unit's sidecar has up to three names, the one named after the object describes it.
pub(super) fn units_holding(deps: &Path, sources: &HashSet<PathBuf>, root: &Path) -> Vec<Unit> {
    let mut by_object: HashMap<PathBuf, Unit> = HashMap::new();
    let mut entries: Vec<PathBuf> = fs::read_dir(deps).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
    entries.sort();
    for path in entries {
        let Some(j) = fs::read(&path).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()) else { continue };
        if j["is_dependency"].as_bool() == Some(true) {
            continue;
        }
        let own = own_sources_of(&j);
        let holds = match &own {
            Some(own) => sources.iter().any(|s| own.contains(s)),
            None => j["file_hashes"].as_object().is_some_and(|files| files.keys().any(|f| sources.contains(&canonical(Path::new(f))))),
        };
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
                let env: HashMap<String, String> = j["env"].as_object().into_iter().flatten().filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string()))).collect();
                let name = crate_name(&args, &env);
                by_object.insert(key, Unit { name, args, env, object, sidecar: path.clone(), aliases: Vec::new(), own });
            }
        }
    }
    // Without `own_sources` (a sidecar from before it), a unit records the sources of the workspace crates it depends
    // on too: on this repository a file of `ridx-format` was in 68 units. Such a source belongs to the units whose
    // crate root lies in the innermost directory holding it (`src/lib.rs` and `src/main.rs` own `src/`).
    let root_dir = |u: &Unit| crate_root(u, root).and_then(|r| r.parent().map(Path::to_path_buf));
    let guessed = |u: &&Unit| u.own.is_none();
    let owner_of = |src: &Path| by_object.values().filter(guessed).filter_map(root_dir).filter(|d| src.starts_with(d)).max_by_key(|d| d.as_os_str().len());
    let owners: HashSet<PathBuf> = sources.iter().filter_map(|s| owner_of(s)).collect();
    let unowned = sources.iter().any(|s| owner_of(s).is_none());
    let mut units: Vec<Unit> = by_object.into_values().filter(|u| u.own.is_some() || unowned || root_dir(u).is_none_or(|d| owners.contains(&d))).collect();
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
