/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use super::*;

mod dependency;
mod workspace;

use dependency::build_immutable_dependency_index;
use workspace::build_workspace_crate_index;

struct CrateIndexBuildContext<'a> {
    start: Instant,
    local_stable_crate_id: u64,
    caught_panics: usize,
    crate_name: String,
    crate_version: String,
    manifest_dir: Option<PathBuf>,
    suffix: String,
    unit_key: String,
    deps_dir: PathBuf,
    dep_keys_or_names: Vec<String>,
    extern_units: std::collections::HashMap<String, String>,
    krate_to_unit: std::collections::HashMap<rustc_span::def_id::CrateNum, String>,
    out_dir: &'a Path,
    cache_dir: &'a Path,
    driver_hash: u64,
    compiler_args: &'a [String],
    crate_cfg: &'a [String],
}

fn resolve_extern_and_crate_units(
    tcx: TyCtxt<'_>,
    compiler_args: &[String],
    deps_dir: &Path,
    unit_key: &str,
) -> (Vec<String>, std::collections::HashMap<String, String>, std::collections::HashMap<rustc_span::def_id::CrateNum, String>) {
    let unit_key = unit_key.to_string();
    let mut dep_keys_or_names = Vec::new();
    let mut extern_units = std::collections::HashMap::new();

    for i in 0..compiler_args.len() {
        let arg = &compiler_args[i];
        let ext_val = if arg == "--extern" && i + 1 < compiler_args.len() { Some(compiler_args[i + 1].as_str()) } else { arg.strip_prefix("--extern=") };
        if let Some(val) = ext_val {
            let (name_part, path_opt) = if let Some((n, p)) = val.split_once('=') {
                (n.trim(), Some(p.trim()))
            } else if let Some((n, p)) = val.split_once(':') {
                (n.trim(), Some(p.trim()))
            } else {
                (val.trim(), None)
            };

            let clean_alias = name_part.rsplit(':').next().unwrap_or(name_part);
            if !clean_alias.is_empty() && !dep_keys_or_names.contains(&clean_alias.to_string()) {
                dep_keys_or_names.push(clean_alias.to_string());
            }

            // Look up disambiguated .deps/<dep_stem>.json for this extern
            let mut resolved_unit = None;
            if let Some(p_str) = path_opt {
                let p = Path::new(p_str);
                let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                let stem_without_lib = stem.strip_prefix("lib").unwrap_or(stem);
                let cand_files = [deps_dir.join(format!("{stem_without_lib}.json")), deps_dir.join(format!("{stem}.json")), deps_dir.join(format!("{clean_alias}.json"))];
                for c in &cand_files {
                    if c.is_file() {
                        if let Ok(content) = fs::read(c) {
                            if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&content) {
                                if let Some(uk) = val.get("unit_key").and_then(|v| v.as_str()) {
                                    resolved_unit = Some(uk.to_string());
                                    let dep_name = val.get("name").and_then(|v| v.as_str()).unwrap_or(clean_alias);
                                    if !dep_keys_or_names.contains(&dep_name.to_string()) {
                                        dep_keys_or_names.push(dep_name.to_string());
                                    }
                                    break;
                                }
                            }
                        }
                    }
                }
            } else {
                let c = deps_dir.join(format!("{clean_alias}.json"));
                if let Ok(content) = fs::read(&c) {
                    if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&content) {
                        resolved_unit = val.get("unit_key").and_then(|v| v.as_str()).map(ToString::to_string);
                    }
                }
            }

            if let Some(uk) = resolved_unit {
                extern_units.insert(clean_alias.to_string(), uk.clone());
                if let Some(p_str) = path_opt {
                    let p = Path::new(p_str);
                    if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                        let stem_without_lib = stem.strip_prefix("lib").unwrap_or(stem);
                        extern_units.insert(stem_without_lib.to_string(), uk.clone());
                    }
                }
                if !dep_keys_or_names.contains(&uk) {
                    dep_keys_or_names.push(uk);
                }
            }
        }
    }

    let mut krate_to_unit: std::collections::HashMap<rustc_span::def_id::CrateNum, String> = std::collections::HashMap::new();
    krate_to_unit.insert(rustc_span::def_id::LOCAL_CRATE, unit_key.clone());

    for &cnum in tcx.crates(()) {
        let stable_id = tcx.stable_crate_id(cnum).as_u64();
        let cname = tcx.crate_name(cnum).to_string();
        let csource = tcx.used_crate_source(cnum);
        let cand_path = csource.rmeta.as_deref().or_else(|| csource.rlib.as_deref()).or_else(|| csource.dylib.as_deref());
        let mut found_unit = None;

        // 1. Check stable crate ID first (exact disambiguation for duplicate crate versions e.g. syn 2 vs 3)
        let stable_id_file = deps_dir.join(format!("{cname}_{stable_id:016x}.json"));
        if stable_id_file.is_file() {
            if let Ok(content) = fs::read(&stable_id_file) {
                if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&content) {
                    if let Some(uk) = val.get("unit_key").and_then(|v| v.as_str()) {
                        found_unit = Some(uk.to_string());
                    }
                }
            }
        }

        // 2. Check stem from csource (rmeta/rlib/dylib)
        if found_unit.is_none() {
            if let Some(p) = cand_path {
                let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                let stem_without_lib = stem.strip_prefix("lib").unwrap_or(stem);
                let cand_files = [deps_dir.join(format!("{stem_without_lib}.json")), deps_dir.join(format!("{stem}.json"))];
                for c in &cand_files {
                    if c.is_file() {
                        if let Ok(content) = fs::read(c) {
                            if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&content) {
                                if let Some(uk) = val.get("unit_key").and_then(|v| v.as_str()) {
                                    found_unit = Some(uk.to_string());
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }

        // 3. Fallback to extern_units
        if found_unit.is_none() {
            if let Some(uk) = extern_units.get(&cname) {
                found_unit = Some(uk.clone());
            }
        }
        if let Some(uk) = found_unit {
            krate_to_unit.insert(cnum, uk);
        }
    }

    (dep_keys_or_names, extern_units, krate_to_unit)
}

pub(crate) fn build_crate_index(tcx: TyCtxt<'_>, env: &DriverEnv, disambiguator: Option<&str>, compiler_args: &[String], crate_cfg: &[String]) {
    let out_dir = &env.out_dir;
    let start = Instant::now();
    let local_stable_crate_id = tcx.stable_crate_id(rustc_span::def_id::LOCAL_CRATE).as_u64();
    let caught_panics = CAUGHT_PANICS.load(std::sync::atomic::Ordering::Relaxed);
    let crate_name = tcx.crate_name(rustc_span::def_id::LOCAL_CRATE).to_string();
    let crate_version = env.pkg_version.clone();

    let manifest_dir = env.manifest_dir.clone();
    let is_immutable = manifest_dir.as_ref().map(|m| is_immutable_package(m)).unwrap_or(false);

    let suffix = match disambiguator {
        Some(d) if d.starts_with('-') || d.starts_with('_') => d.to_string(),
        Some(d) if !d.is_empty() => format!("_{d}"),
        _ => String::new(),
    };

    let unit_key =
        compute_dependency_unit_key_with_env(&crate_name, &crate_version, disambiguator, manifest_dir.as_deref(), compiler_args, env.build_out_dir.as_deref(), env.driver_hash);

    let deps_dir = out_dir.join(".deps");
    let _ = fs::create_dir_all(&deps_dir);

    let (dep_keys_or_names, extern_units, krate_to_unit) = resolve_extern_and_crate_units(tcx, compiler_args, &deps_dir, &unit_key);
    let ctx = CrateIndexBuildContext {
        start,
        local_stable_crate_id,
        caught_panics,
        crate_name,
        crate_version,
        manifest_dir,
        suffix,
        unit_key,
        deps_dir,
        dep_keys_or_names,
        extern_units,
        krate_to_unit,
        out_dir,
        cache_dir: &env.cache_dir,
        driver_hash: env.driver_hash,
        compiler_args,
        crate_cfg,
    };
    if is_immutable {
        build_immutable_dependency_index(tcx, ctx);
        return;
    }
    build_workspace_crate_index(tcx, ctx);
}
