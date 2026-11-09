/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Targets whose `required-features` the indexed feature set does not enable. `cargo check --all-targets` skips
//! them, while rust-analyzer analyzes every target; each is checked on its own with exactly its required features,
//! and only its own shard joins the variant, so the shared crates keep the variant's feature set.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::Command;

/// A target `cargo check` skips under the variant's features.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedTarget {
    pub package: String,
    /// `example`, `test`, `bench` or `bin`.
    pub kind: String,
    pub name: String,
    pub required: Vec<String>,
}

/// The targets of `metadata` (the `cargo metadata --no-deps` JSON) whose required features are not all enabled
/// by the package's default features plus `features`.
pub fn skipped_targets(metadata: &serde_json::Value, features: &[String], no_default_features: bool) -> Vec<SkippedTarget> {
    let mut out = Vec::new();
    for pkg in metadata["packages"].as_array().into_iter().flatten() {
        let package = pkg["name"].as_str().unwrap_or_default().to_string();
        let enabled = enabled_features(pkg, features, no_default_features);
        for t in pkg["targets"].as_array().into_iter().flatten() {
            let required: Vec<String> = t["required-features"].as_array().into_iter().flatten().filter_map(|f| f.as_str().map(str::to_string)).collect();
            let kind = t["kind"].as_array().and_then(|k| k.first()).and_then(|k| k.as_str()).unwrap_or_default();
            if required.is_empty() || !matches!(kind, "example" | "test" | "bench" | "bin") || required.iter().all(|f| enabled.contains(f)) {
                continue;
            }
            out.push(SkippedTarget { package: package.clone(), kind: kind.to_string(), name: t["name"].as_str().unwrap_or_default().to_string(), required });
        }
    }
    out
}

/// The features a package has enabled: `default` (unless disabled) and `requested`, closed over the features
/// each enables within the package.
fn enabled_features(pkg: &serde_json::Value, requested: &[String], no_default_features: bool) -> Vec<String> {
    let mut stack: Vec<String> = requested.iter().map(|f| f.rsplit('/').next().unwrap_or(f).to_string()).collect();
    if !no_default_features {
        stack.push("default".to_string());
    }
    let mut enabled = Vec::new();
    while let Some(f) = stack.pop() {
        if enabled.contains(&f) {
            continue;
        }
        for sub in pkg["features"][&f].as_array().into_iter().flatten().filter_map(|v| v.as_str()) {
            if !sub.contains('/') && !sub.starts_with("dep:") {
                stack.push(sub.to_string());
            }
        }
        enabled.push(f);
    }
    enabled
}

/// The `check` invocations covering `targets`: one per package and required-feature set, naming each target.
pub fn check_args(targets: &[SkippedTarget], manifest_path: &Path, features: &[String]) -> Vec<Vec<String>> {
    let mut groups: BTreeMap<(String, Vec<String>), Vec<&SkippedTarget>> = BTreeMap::new();
    for t in targets {
        let mut feats = t.required.clone();
        feats.sort();
        groups.entry((t.package.clone(), feats)).or_default().push(t);
    }
    groups
        .into_iter()
        .map(|((package, feats), ts)| {
            let mut args = vec!["check".to_string(), "--manifest-path".to_string(), manifest_path.display().to_string(), "-p".to_string(), package];
            for t in ts {
                args.push(format!("--{}", t.kind));
                args.push(t.name.clone());
            }
            let all: Vec<String> = feats.into_iter().chain(features.iter().cloned()).collect();
            args.push("--features".to_string());
            args.push(all.join(","));
            args
        })
        .collect()
}

/// Indexes the skipped targets into `scratch` and moves their own shards into `variant_out_dir`. Failures are
/// reported and leave the variant as the main check produced it.
pub fn index_skipped_targets(
    program: &str,
    envs: &[(String, String)],
    work_dir: &Path,
    manifest_path: &Path,
    variant_out_dir: &Path,
    features: &[String],
    no_default_features: bool,
) {
    let Ok(output) = Command::new("cargo").args(["metadata", "--format-version", "1", "--no-deps", "--manifest-path"]).arg(manifest_path).output() else { return };
    let Ok(metadata) = serde_json::from_slice::<serde_json::Value>(&output.stdout) else { return };
    let targets = skipped_targets(&metadata, features, no_default_features);
    if targets.is_empty() {
        return;
    }
    eprintln!("  Extra targets  : {} with required features", targets.len());
    let Some(scratch) = variant_out_dir.parent().map(|p| p.join(format!(".required_features-{}", variant_out_dir.file_name().unwrap_or_default().to_string_lossy()))) else {
        return;
    };
    let _ = fs::create_dir_all(&scratch);
    for args in check_args(&targets, manifest_path, features) {
        let mut cmd = Command::new(program);
        cmd.current_dir(work_dir).args(&args);
        for (k, v) in envs {
            let v = if k == "RIDX_OUT_DIR" { scratch.display().to_string() } else { v.clone() };
            cmd.env(k, v);
        }
        match cmd.output() {
            Ok(o) if o.status.success() => {}
            Ok(o) => eprintln!("  warning: check of required-feature targets failed ({}): {}", o.status, args.join(" ")),
            Err(e) => eprintln!("  warning: could not run check of required-feature targets: {e}"),
        }
    }
    // Only the targets' own shards join the variant: the workspace libraries rebuilt with the extra features would
    // duplicate every library per feature set.
    let own: Vec<String> = targets.iter().map(|t| t.name.replace('-', "_")).collect();
    let mut moved = Vec::new();
    for entry in fs::read_dir(&scratch).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.ends_with(".ridx") && own.iter().any(|p| name.starts_with(&format!("{p}-"))) && fs::rename(entry.path(), variant_out_dir.join(&name)).is_ok() {
            moved.push(name);
        }
    }
    adopt_sidecars(&scratch.join(".deps"), &variant_out_dir.join(".deps"), variant_out_dir, &moved);
    let _ = fs::remove_dir_all(&scratch);
}

/// The units of each workspace library in the variant's sidecars, the one the variant's other units link most
/// first (one library can be built several times, for targets with different features).
fn variant_lib_units(deps_dir: &Path) -> BTreeMap<String, Vec<String>> {
    let sidecars: Vec<serde_json::Value> =
        fs::read_dir(deps_dir).into_iter().flatten().flatten().filter_map(|e| fs::read(e.path()).ok().and_then(|b| serde_json::from_slice(&b).ok())).collect();
    let mut links: BTreeMap<&str, usize> = BTreeMap::new();
    for j in &sidecars {
        for u in j["extern_units"].as_object().into_iter().flat_map(|m| m.values()).filter_map(|v| v.as_str()) {
            *links.entry(u).or_default() += 1;
        }
    }
    let mut units: BTreeMap<String, Vec<(usize, String)>> = BTreeMap::new();
    for j in &sidecars {
        if j["is_dependency"] == false && j["target_kind"] == "lib" {
            if let (Some(n), Some(u)) = (j["name"].as_str(), j["unit_key"].as_str()) {
                let list = units.entry(n.to_string()).or_default();
                if !list.iter().any(|(_, x)| x == u) {
                    list.push((links.get(u).copied().unwrap_or(0), u.to_string()));
                }
            }
        }
    }
    units
        .into_iter()
        .map(|(n, mut list)| {
            list.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
            (n, list.into_iter().map(|(_, u)| u).collect())
        })
        .collect()
}

/// Points a sidecar's workspace libraries at the variant's units of the same crates: `extern_units` at the most
/// linked one, `dependencies` at all of them, so each is reachable from the shard. The shard's references keep
/// naming the libraries built with the extra features; the workspace matches such a reference, whose unit it does
/// not hold, through the shard's `extern_units`.
fn repoint_units(sidecar: &mut serde_json::Value, lib_units: &BTreeMap<String, Vec<String>>) {
    if let Some(ext) = sidecar["extern_units"].as_object_mut() {
        for (k, v) in ext.iter_mut() {
            if let Some(u) = lib_units.get(k.split('-').next().unwrap_or(k)).and_then(|l| l.first()) {
                *v = serde_json::json!(u);
            }
        }
    }
    // A flat list of crate names, each followed by its unit key when it has one.
    let is_key = |v: &serde_json::Value| v.as_str().is_some_and(|s| s.len() == 16 && s.bytes().all(|b| b.is_ascii_hexdigit()));
    if let Some(deps) = sidecar["dependencies"].as_array_mut() {
        let mut extra = Vec::new();
        for i in 0..deps.len().saturating_sub(1) {
            if is_key(&deps[i + 1]) {
                if let Some((name, list)) = deps[i].as_str().and_then(|n| lib_units.get(n).map(|l| (n.to_string(), l))) {
                    if let Some(first) = list.first() {
                        deps[i + 1] = serde_json::json!(first);
                    }
                    for u in list.iter().skip(1) {
                        extra.push(serde_json::json!(name));
                        extra.push(serde_json::json!(u));
                    }
                }
            }
        }
        deps.extend(extra);
    }
}

/// Adds to a sidecar's `extern_units` the workspace libraries its linked units reach in turn (`clap_builder` behind
/// `clap`), from the variant's sidecars (`unit_externs`: unit key → that unit's `extern_units`). The shard's
/// references name such libraries too.
fn link_transitive(sidecar: &mut serde_json::Value, unit_externs: &BTreeMap<String, BTreeMap<String, String>>, lib_units: &BTreeMap<String, Vec<String>>) {
    let Some(ext) = sidecar["extern_units"].as_object_mut() else { return };
    let mut queue: Vec<String> = ext.values().filter_map(|v| v.as_str().map(str::to_string)).collect();
    let mut seen: std::collections::BTreeSet<String> = queue.iter().cloned().collect();
    while let Some(unit) = queue.pop() {
        for (name, u) in unit_externs.get(&unit).into_iter().flatten() {
            let name = name.split('-').next().unwrap_or(name);
            if lib_units.contains_key(name) && !ext.contains_key(name) {
                ext.insert(name.to_string(), serde_json::json!(u));
            }
            if seen.insert(u.clone()) {
                queue.push(u.clone());
            }
        }
    }
}

/// Each workspace unit's `extern_units` in the variant's sidecars.
fn variant_unit_externs(deps_dir: &Path) -> BTreeMap<String, BTreeMap<String, String>> {
    let mut out = BTreeMap::new();
    for entry in fs::read_dir(deps_dir).into_iter().flatten().flatten() {
        let Some(j) = fs::read(entry.path()).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()) else { continue };
        if j["is_dependency"] != false {
            continue;
        }
        if let (Some(u), Some(ext)) = (j["unit_key"].as_str(), j["extern_units"].as_object()) {
            out.insert(u.to_string(), ext.iter().filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string()))).collect());
        }
    }
    out
}

/// Moves the workspace sidecars of the `moved` shards from `from` to `to`, pointed at the variant's libraries and
/// at the shards' new location.
fn adopt_sidecars(from: &Path, to: &Path, variant_out_dir: &Path, moved: &[String]) {
    let read = |p: &Path| fs::read(p).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok());
    let _ = fs::create_dir_all(to);
    let lib_units = variant_lib_units(to);
    let unit_externs = variant_unit_externs(to);
    for entry in fs::read_dir(from).into_iter().flatten().flatten() {
        let Some(mut j) = read(&entry.path()) else { continue };
        let shard = j["object_path"].as_str().and_then(|p| Path::new(p).file_name()).map(|f| f.to_string_lossy().to_string());
        if j["is_dependency"] != false || !shard.is_some_and(|s| moved.contains(&s)) || to.join(entry.file_name()).exists() {
            continue;
        }
        repoint_units(&mut j, &lib_units);
        link_transitive(&mut j, &unit_externs, &lib_units);
        if let Some(obj) = j["object_path"].as_str().and_then(|p| Path::new(p).file_name().map(|f| variant_out_dir.join(f))) {
            j["object_path"] = serde_json::json!(obj.display().to_string());
        }
        if let Ok(bytes) = serde_json::to_vec(&j) {
            let _ = fs::write(to.join(entry.file_name()), bytes);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_skipped_targets_and_their_checks() {
        let meta = json!({ "packages": [{
            "name": "clap",
            "features": { "default": ["std", "color"], "std": [], "color": [], "cargo": [], "derive": ["dep:clap_derive"] },
            "targets": [
                { "name": "clap", "kind": ["lib"], "required-features": [] },
                { "name": "find", "kind": ["example"], "required-features": ["cargo"] },
                { "name": "colors", "kind": ["example"], "required-features": ["color"] },
                { "name": "01_quick", "kind": ["example"], "required-features": ["cargo"] },
                { "name": "derive_ui", "kind": ["test"], "required-features": ["derive", "cargo"] }
            ]
        }]});
        let skipped = skipped_targets(&meta, &[], false);
        let names: Vec<&str> = skipped.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["find", "01_quick", "derive_ui"]);
        let args = check_args(&skipped, Path::new("/w/Cargo.toml"), &[]);
        assert_eq!(args.len(), 2);
        assert!(args.iter().any(|a| a.ends_with(&["--example".into(), "find".into(), "--example".into(), "01_quick".into(), "--features".into(), "cargo".into()])));
        assert!(skipped_targets(&meta, &["cargo".to_string(), "derive".to_string()], false).is_empty());
    }

    #[test]
    fn test_repoint_units_to_variant_libraries() {
        let (aa, ss, main) = ("aaaaaaaaaaaaaaaa", "5555555555555555", "0123456789abcdef");
        let mut side = json!({ "extern_units": { "clap": aa, "clap-9edd": aa, "shlex": ss }, "dependencies": ["clap_cargo", "clap", aa, "shlex", ss] });
        let other = "1111111111111111";
        let libs = BTreeMap::from([("clap".to_string(), vec![main.to_string(), other.to_string()])]);
        repoint_units(&mut side, &libs);
        assert_eq!(side, json!({ "extern_units": { "clap": main, "clap-9edd": main, "shlex": ss }, "dependencies": ["clap_cargo", "clap", main, "shlex", ss, "clap", other] }));
    }

    #[test]
    fn test_link_transitive_libraries() {
        let mut side = json!({ "extern_units": { "clap": "c1", "shlex": "s1" } });
        let unit_externs = BTreeMap::from([
            ("c1".to_string(), BTreeMap::from([("clap_builder".to_string(), "b1".to_string())])),
            ("b1".to_string(), BTreeMap::from([("clap_lex".to_string(), "l1".to_string())])),
        ]);
        let libs = BTreeMap::from([("clap_builder".to_string(), vec!["b1".to_string()]), ("clap_lex".to_string(), vec!["l1".to_string()])]);
        link_transitive(&mut side, &unit_externs, &libs);
        assert_eq!(side, json!({ "extern_units": { "clap": "c1", "shlex": "s1", "clap_builder": "b1", "clap_lex": "l1" } }));
    }
}
