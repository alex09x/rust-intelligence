/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Cargo workspace target discovery via `cargo metadata` for entrypoint, `proc-macro`,
//! and unindexed workspace target resolution.

use super::{TargetKind, WorkspaceIndex};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Metadata for a single Cargo target belonging to a workspace member package.
#[derive(Debug, Clone)]
pub struct CargoTargetInfo {
    pub name: String,
    pub pkg_crate: String,
    pub kinds: Vec<String>,
    pub src_path: PathBuf,
    pub harness: bool,
    pub dep_crates: HashSet<String>,
}

/// Discovered Cargo targets across the workspace.
#[derive(Debug, Clone, Default)]
pub struct CargoWorkspaceTargets {
    /// Canonical `src_path` -> `(target_kind, target_name)` for targets with a `fn main` entrypoint.
    pub main_entrypoints: HashMap<PathBuf, (String, String)>,
    /// `(crate_ident, src_dir)` for workspace `proc-macro` targets.
    pub proc_macro_crates: Vec<(String, PathBuf)>,
    /// Non-lib Cargo targets (`bin`, `example`, `test`, `bench`, `custom-build`) of workspace members.
    pub member_targets: Vec<CargoTargetInfo>,
    /// Transitive workspace dependency map (`crate_ident` -> reachable crate idents).
    pub ws_pkg_deps: HashMap<String, HashSet<String>>,
}

fn canonical_or_normalized(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| crate::paths::normalize_path(path))
}

pub(crate) fn find_manifest_path(ws: &WorkspaceIndex) -> Option<PathBuf> {
    if let Some(ref mp) = ws.manifest_path {
        if mp.file_name().and_then(|n| n.to_str()) == Some("Cargo.toml") && mp.is_file() {
            return Some(mp.clone());
        }
    }
    for base in [ws.workspace_root.as_deref(), ws.root_dir.as_deref()].into_iter().flatten() {
        for dir in base.ancestors().take(4) {
            let cand = dir.join("Cargo.toml");
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    None
}

type MetadataCacheMap = HashMap<PathBuf, (Option<std::time::SystemTime>, Option<Value>)>;
static CARGO_METADATA_CACHE: std::sync::OnceLock<std::sync::Mutex<MetadataCacheMap>> = std::sync::OnceLock::new();

pub(crate) fn cached_cargo_metadata(manifest: &Path) -> Option<Value> {
    let key = canonical_or_normalized(manifest);
    let mtime = std::fs::metadata(&key).and_then(|m| m.modified()).ok();
    let mutex = CARGO_METADATA_CACHE.get_or_init(|| std::sync::Mutex::new(HashMap::new()));
    if let Some((cached_mtime, ref cached_val)) = mutex.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        if *cached_mtime == mtime {
            return cached_val.clone();
        }
    }
    let val = run_cargo_metadata(&key);
    mutex.lock().unwrap_or_else(|e| e.into_inner()).insert(key, (mtime, val.clone()));
    val
}

fn run_cargo_metadata(manifest: &Path) -> Option<Value> {
    for offline in [true, false] {
        let mut cmd = Command::new("cargo");
        cmd.arg("metadata").arg("--no-deps").arg("--format-version").arg("1");
        if offline {
            cmd.arg("--offline");
        }
        cmd.arg("--manifest-path").arg(manifest);
        if let Ok(out) = cmd.output() {
            if out.status.success() {
                if let Ok(val) = serde_json::from_slice::<Value>(&out.stdout) {
                    return Some(val);
                }
            }
        }
    }
    None
}

impl CargoWorkspaceTargets {
    /// Discovers workspace targets from `cargo metadata`, falling back to shard `TargetKind` metadata
    /// when no `Cargo.toml` exists on disk (e.g. standalone synthetic index fixtures).
    pub fn discover(ws: &WorkspaceIndex) -> Self {
        if let Some(manifest) = find_manifest_path(ws) {
            if let Some(meta) = cached_cargo_metadata(&manifest) {
                return Self::from_metadata_json(ws, &meta);
            }
        }
        Self::from_shard_fallback(ws)
    }

    fn from_metadata_json(ws: &WorkspaceIndex, meta: &Value) -> Self {
        let mut out = Self::default();
        let member_ids: HashSet<&str> = meta["workspace_members"].as_array().into_iter().flatten().filter_map(|v| v.as_str()).collect();

        let Some(packages) = meta["packages"].as_array() else {
            return Self::from_shard_fallback(ws);
        };

        let mut enabled_feats: HashMap<String, HashSet<String>> = HashMap::new();
        for pkg in packages {
            let pkg_id = pkg["id"].as_str().unwrap_or("");
            if !member_ids.is_empty() && !member_ids.contains(pkg_id) {
                continue;
            }
            let pname = pkg["name"].as_str().unwrap_or("");
            if let Some(fmap) = pkg["features"].as_object() {
                if fmap.contains_key("default") {
                    enabled_feats.entry(pname.to_string()).or_default().insert("default".to_string());
                }
            }
        }
        let mut f_changed = true;
        while f_changed {
            f_changed = false;
            for pkg in packages {
                let pname = pkg["name"].as_str().unwrap_or("");
                let Some(fmap) = pkg["features"].as_object() else { continue };
                let cur: Vec<String> = enabled_feats.get(pname).into_iter().flatten().cloned().collect();
                for f in cur {
                    for sub in fmap.get(&f).and_then(|v| v.as_array()).into_iter().flatten().filter_map(|v| v.as_str()) {
                        let sub = sub.strip_prefix("dep:").unwrap_or(sub);
                        if let Some((dep_pkg, dep_f)) = sub.split_once('/') {
                            let clean_pkg = dep_pkg.trim_end_matches('?');
                            if enabled_feats.entry(clean_pkg.to_string()).or_default().insert(dep_f.to_string()) {
                                f_changed = true;
                            }
                            if enabled_feats.entry(pname.to_string()).or_default().insert(clean_pkg.to_string()) {
                                f_changed = true;
                            }
                        } else if enabled_feats.entry(pname.to_string()).or_default().insert(sub.to_string()) {
                            f_changed = true;
                        }
                    }
                }
            }
        }

        let mut ws_pkg_deps: HashMap<String, HashSet<String>> = HashMap::new();
        for pkg in packages {
            let pkg_id = pkg["id"].as_str().unwrap_or("");
            if !member_ids.is_empty() && !member_ids.contains(pkg_id) {
                continue;
            }
            let pkg_raw_name = pkg["name"].as_str().unwrap_or("");
            let pkg_crate = pkg_raw_name.replace('-', "_");
            if pkg_crate.is_empty() {
                continue;
            }
            let pkg_feats = enabled_feats.get(pkg_raw_name);
            let mut pkg_set = HashSet::from([pkg_crate.clone()]);
            let mut lib_idents = Vec::new();
            if let Some(deps) = pkg["dependencies"].as_array() {
                for dep in deps {
                    let dep_name = dep["name"].as_str().unwrap_or("");
                    let raw = dep["rename"].as_str().filter(|s| !s.is_empty()).unwrap_or(dep_name);
                    let is_opt = dep["optional"].as_bool().unwrap_or(false);
                    let opt_active = !is_opt || pkg_feats.is_some_and(|fs| fs.contains(raw) || fs.contains(dep_name));
                    if !raw.is_empty() && opt_active {
                        pkg_set.insert(raw.replace('-', "_"));
                    }
                }
            }
            if let Some(targets) = pkg["targets"].as_array() {
                for t2 in targets {
                    if t2["kind"].as_array().is_some_and(|ks| ks.iter().any(|k| matches!(k.as_str(), Some("lib" | "rlib" | "proc-macro")))) {
                        if let Some(lib_nm) = t2["name"].as_str() {
                            let lib_ident = lib_nm.replace('-', "_");
                            pkg_set.insert(lib_ident.clone());
                            lib_idents.push(lib_ident);
                        }
                    }
                }
            }
            ws_pkg_deps.entry(pkg_crate).or_default().extend(pkg_set.iter().cloned());
            for lib_ident in lib_idents {
                ws_pkg_deps.entry(lib_ident).or_default().extend(pkg_set.iter().cloned());
            }
        }
        let ws_keys: Vec<String> = ws_pkg_deps.keys().cloned().collect();
        let mut changed = true;
        while changed {
            changed = false;
            for k in &ws_keys {
                let current: Vec<String> = ws_pkg_deps.get(k).into_iter().flatten().cloned().collect();
                for dep in current {
                    if let Some(trans) = ws_pkg_deps.get(&dep).cloned() {
                        let entry = ws_pkg_deps.get_mut(k).unwrap();
                        for td in trans {
                            if entry.insert(td) {
                                changed = true;
                            }
                        }
                    }
                }
            }
        }

        for pkg in packages {
            let pkg_id = pkg["id"].as_str().unwrap_or("");
            if !member_ids.is_empty() && !member_ids.contains(pkg_id) {
                continue;
            }
            let pkg_name = pkg["name"].as_str().unwrap_or("");
            let pkg_crate = pkg_name.replace('-', "_");
            let dep_crates = ws_pkg_deps.get(&pkg_crate).cloned().unwrap_or_else(|| {
                let mut s = HashSet::new();
                if !pkg_crate.is_empty() {
                    s.insert(pkg_crate.clone());
                }
                s
            });

            let Some(targets) = pkg["targets"].as_array() else {
                continue;
            };
            for tgt in targets {
                let Some(src_str) = tgt["src_path"].as_str() else {
                    continue;
                };
                let src_path = canonical_or_normalized(Path::new(src_str));
                let tgt_name = tgt["name"].as_str().unwrap_or(pkg_name).to_string();
                let harness = tgt["harness"].as_bool().unwrap_or(true);
                let kinds: Vec<String> = tgt["kind"].as_array().into_iter().flatten().filter_map(|k| k.as_str().map(String::from)).collect();
                let crate_types: Vec<&str> = tgt["crate_types"].as_array().into_iter().flatten().filter_map(|k| k.as_str()).collect();

                let is_proc_macro = kinds.iter().any(|k| k == "proc-macro") || crate_types.contains(&"proc-macro");
                if is_proc_macro {
                    if let Some(src_dir) = src_path.parent() {
                        let pm_ident = tgt_name.replace('-', "_");
                        if !out.proc_macro_crates.iter().any(|(n, d)| n == &pm_ident && d == src_dir) {
                            out.proc_macro_crates.push((pm_ident, src_dir.to_path_buf()));
                        }
                    }
                }

                let req_satisfied = tgt["required-features"]
                    .as_array()
                    .is_none_or(|rfs| rfs.iter().filter_map(|v| v.as_str()).all(|rf| enabled_feats.get(pkg_name).is_some_and(|fs| fs.contains(rf))));
                if req_satisfied || !ws.resolved_reader_file_ids(src_str).is_empty() {
                    if let Some(main_kind) = kinds.iter().find(|k| matches!(k.as_str(), "bin" | "example") || (!harness && matches!(k.as_str(), "test" | "bench"))) {
                        out.main_entrypoints.insert(src_path.clone(), (main_kind.clone(), tgt_name.clone()));
                    }
                }

                if kinds.iter().any(|k| matches!(k.as_str(), "bin" | "example" | "test" | "bench" | "custom-build")) {
                    let mut target_deps = dep_crates.clone();
                    for t2 in targets {
                        if t2["kind"].as_array().is_some_and(|ks| ks.iter().any(|k| matches!(k.as_str(), Some("lib" | "rlib" | "proc-macro")))) {
                            if let Some(lib_nm) = t2["name"].as_str() {
                                target_deps.insert(lib_nm.replace('-', "_"));
                            }
                        }
                    }
                    out.member_targets.push(CargoTargetInfo { name: tgt_name, pkg_crate: pkg_crate.clone(), kinds, src_path, harness, dep_crates: target_deps });
                }
            }
        }
        out.ws_pkg_deps = ws_pkg_deps;
        out
    }

    /// Returns `true` if `root_crate` can re-export symbols defined in `expected_crate`.
    pub fn crate_can_reexport(&self, root_crate: &str, expected_crate: &str) -> bool {
        if root_crate == expected_crate || self.ws_pkg_deps.is_empty() {
            return true;
        }
        self.ws_pkg_deps.get(root_crate).is_some_and(|deps| deps.contains(expected_crate))
    }

    fn from_shard_fallback(ws: &WorkspaceIndex) -> Self {
        let mut out = Self::default();
        for (i, reader) in ws.readers.iter().enumerate() {
            if ws.is_reader_external(i) {
                continue;
            }
            let Some(tk) = ws.target_kinds.get(i).and_then(|k| *k) else {
                continue;
            };
            let cname = reader.crate_name().to_string();
            match tk {
                TargetKind::Bin | TargetKind::Example => {
                    let kind_label = if tk == TargetKind::Bin { "bin" } else { "example" };
                    for d in reader.definitions() {
                        if crate::format::DefKind::from(d.kind) == crate::format::DefKind::Fn
                            && reader.get_str(d.name_offset()) == "main"
                            && !reader.get_str(d.path_offset()).strip_suffix("::main").unwrap_or("").contains("::")
                        {
                            let f = reader.get_file(d.span().file_id());
                            let full = ws.resolve_path_for_crate(&cname, f);
                            out.main_entrypoints.insert(canonical_or_normalized(&full), (kind_label.to_string(), cname.clone()));
                        }
                    }
                }
                TargetKind::Lib => {
                    if let Some(first_def) = reader.definitions().first() {
                        let f = reader.get_file(first_def.span().file_id());
                        let full = ws.resolve_path_for_crate(&cname, f);
                        let uses_pm = reader.references().iter().any(|r| reader.get_str(r.target_path_offset()).starts_with("proc_macro"));
                        if uses_pm {
                            if let Some(dir) = full.parent() {
                                let norm_dir = canonical_or_normalized(dir);
                                if !out.proc_macro_crates.iter().any(|(n, d)| n == &cname && d == &norm_dir) {
                                    out.proc_macro_crates.push((cname, norm_dir));
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        out
    }

    /// Checks if `file_path` is the root `src_path` of a Cargo target whose entrypoint is `fn main`.
    pub fn main_entrypoint_for(&self, file_path: &Path) -> Option<&(String, String)> {
        let canon = canonical_or_normalized(file_path);
        self.main_entrypoints.get(&canon)
    }
}
