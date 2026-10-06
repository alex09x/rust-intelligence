/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Cargo indexing orchestration, variant hashing, driver invocation, and manifest publishing.

use colored::*;
use ridx_format::WorkspaceIndex;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

#[derive(Debug, Clone)]
pub struct IndexOptions {
    pub path: PathBuf,
    pub out_dir_opt: Option<PathBuf>,
    pub package: Option<String>,
    pub release: bool,
    pub features: Option<String>,
    pub no_default_features: bool,
    pub all_features: bool,
    pub target: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PublishedVariant {
    pub base_out_dir: PathBuf,
    pub variant_out_dir: PathBuf,
    #[allow(dead_code)]
    pub variant_id: String,
}

pub fn find_driver_binary() -> Option<PathBuf> {
    find_driver_binary_internal(std::env::current_exe().ok().as_deref())
}

pub fn find_driver_binary_internal(exe: Option<&Path>) -> Option<PathBuf> {
    if let Ok(driver_env) = std::env::var("RIDX_DRIVER_BIN") {
        let p = PathBuf::from(driver_env);
        if p.is_file() {
            return p.canonicalize().ok();
        }
        return None;
    }

    if let Some(exe) = exe {
        let mut cur = exe.parent();
        while let Some(dir) = cur {
            for sub in [
                "ridx-driver",
                "debug/ridx-driver",
                "release/ridx-driver",
                "target/debug/ridx-driver",
                "target/release/ridx-driver",
                "target/llvm-cov-target/debug/ridx-driver",
            ] {
                let candidate = dir.join(sub);
                if candidate.is_file() {
                    return candidate.canonicalize().ok();
                }
            }
            cur = dir.parent();
        }
    }
    for candidate in [
        PathBuf::from("target/llvm-cov-target/debug/ridx-driver"),
        PathBuf::from("target/release/ridx-driver"),
        PathBuf::from("target/debug/ridx-driver"),
        PathBuf::from("../target/llvm-cov-target/debug/ridx-driver"),
        PathBuf::from("../target/release/ridx-driver"),
        PathBuf::from("../target/debug/ridx-driver"),
    ] {
        if candidate.is_file() {
            return candidate.canonicalize().ok();
        }
    }
    if let Ok(out) = Command::new("which").arg("ridx-driver").output() {
        if out.status.success() {
            let path_str = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !path_str.is_empty() {
                return Some(PathBuf::from(path_str));
            }
        }
    }
    None
}

pub fn resolve_cargo_home() -> Option<PathBuf> {
    if let Ok(cargo_home) = std::env::var("CARGO_HOME") {
        let p = PathBuf::from(cargo_home.trim());
        if !p.as_os_str().is_empty() {
            return Some(p);
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        let p = PathBuf::from(home.trim()).join(".cargo");
        if !home.trim().is_empty() {
            return Some(p);
        }
    }
    #[cfg(windows)]
    if let Ok(userprofile) = std::env::var("USERPROFILE") {
        let p = PathBuf::from(userprofile.trim()).join(".cargo");
        if !userprofile.trim().is_empty() {
            return Some(p);
        }
    }
    None
}

pub fn find_cargo_configs(work_dir: &Path) -> Vec<PathBuf> {
    let mut configs = Vec::new();
    let mut cur = Some(work_dir);
    while let Some(dir) = cur {
        for name in [".cargo/config.toml", ".cargo/config"] {
            let p = dir.join(name);
            if p.is_file() && !configs.contains(&p) {
                configs.push(p);
            }
        }
        cur = dir.parent();
    }
    if let Some(home_dir) = resolve_cargo_home() {
        for name in ["config.toml", "config"] {
            let p = home_dir.join(name);
            if p.is_file() && !configs.contains(&p) {
                configs.push(p);
            }
        }
    }
    configs
}

pub fn extract_config_target(content: &str) -> Option<String> {
    let mut in_build = false;
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            in_build = line == "[build]";
            continue;
        }
        if in_build {
            if let Some((k, v)) = line.split_once('=') {
                if k.trim() == "target" {
                    let v = v.trim().trim_matches('"').trim_matches('\'').trim();
                    if !v.is_empty() {
                        return Some(v.to_string());
                    }
                }
            }
        }
    }
    None
}

pub fn collect_all_workspace_manifests(dir: &Path, manifests: &mut Vec<PathBuf>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_file() {
                if p.file_name().and_then(|s| s.to_str()) == Some("Cargo.toml") {
                    manifests.push(p);
                }
            } else if p.is_dir() {
                let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
                if name != "target" && name != ".git" && !name.starts_with('.') {
                    collect_all_workspace_manifests(&p, manifests);
                }
            }
        }
    }
}

pub fn compute_variant_id(
    profile: &str,
    target: Option<&str>,
    package: Option<&str>,
    features: Option<&str>,
    no_default_features: bool,
    all_features: bool,
    manifest_path: &Path,
) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    profile.hash(&mut hasher);

    let work_dir = manifest_path.parent().unwrap_or_else(|| Path::new("."));

    // 1. Cargo configuration files (.cargo/config.toml, .cargo/config)
    let config_files = find_cargo_configs(work_dir);
    let mut config_target = None;
    for cfg in &config_files {
        cfg.display().to_string().hash(&mut hasher);
        if let Ok(content) = fs::read_to_string(cfg) {
            content.hash(&mut hasher);
            if config_target.is_none() {
                config_target = extract_config_target(&content);
            }
        }
    }

    // 2. Effective target: CLI target -> CARGO_BUILD_TARGET -> .cargo/config target -> "host"
    let env_target = std::env::var("CARGO_BUILD_TARGET").ok();
    let effective_target = target
        .or(env_target.as_deref())
        .or(config_target.as_deref())
        .unwrap_or("host");
    effective_target.hash(&mut hasher);

    // 3. Package filter and feature flags
    package.unwrap_or("workspace").hash(&mut hasher);
    features.unwrap_or("").hash(&mut hasher);
    no_default_features.hash(&mut hasher);
    all_features.hash(&mut hasher);

    // 4. All workspace member manifests (Cargo.toml in workspace root and subdirectories)
    let mut all_manifests = Vec::new();
    collect_all_workspace_manifests(work_dir, &mut all_manifests);
    if !all_manifests.contains(&manifest_path.to_path_buf()) {
        all_manifests.push(manifest_path.to_path_buf());
    }
    all_manifests.sort();
    for m in all_manifests {
        m.strip_prefix(work_dir)
            .unwrap_or(&m)
            .display()
            .to_string()
            .hash(&mut hasher);
        if let Ok(bytes) = fs::read(&m) {
            bytes.hash(&mut hasher);
        }
    }

    // 5. Cargo.lock full file contents (captures resolved dependency graph, versions, sources, active features)
    let lock_file = work_dir.join("Cargo.lock");
    if let Ok(lock_bytes) = fs::read(&lock_file) {
        lock_bytes.hash(&mut hasher);
    }

    // 6. Toolchain version and host details via rustc -Vv
    if let Ok(out) = Command::new("rustc").arg("-Vv").output() {
        if out.status.success() {
            out.stdout.hash(&mut hasher);
        }
    } else if let Ok(toolchain) = std::env::var("RUSTUP_TOOLCHAIN") {
        toolchain.hash(&mut hasher);
    }

    // 7. Flags affecting rustc compilation
    if let Ok(rustflags) = std::env::var("RUSTFLAGS") {
        rustflags.hash(&mut hasher);
    }
    if let Ok(encoded_flags) = std::env::var("CARGO_ENCODED_RUSTFLAGS") {
        encoded_flags.hash(&mut hasher);
    }

    // 8. RIDX binary format and extractor version
    ridx_format::CURRENT_FORMAT_VERSION.hash(&mut hasher);
    ridx_format::EXTRACTOR_VERSION.hash(&mut hasher);

    let hash = hasher.finish();
    format!("{profile}-{:016x}", hash)
}

pub fn try_run_indexing(opts: IndexOptions) -> Result<PublishedVariant, String> {
    let path = opts.path;
    let out_dir_opt = opts.out_dir_opt;
    let package = opts.package;
    let release = opts.release;
    let features = opts.features;
    let no_default_features = opts.no_default_features;
    let all_features = opts.all_features;
    let target = opts.target;

    let (work_dir, manifest_path) = if path.is_file() {
        if path.file_name().and_then(|s| s.to_str()) != Some("Cargo.toml") {
            return Err(format!(
                "target path {:?} is a file but not a Cargo.toml manifest.",
                path
            ));
        }
        let canon = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let dir = canon
            .parent()
            .unwrap_or_else(|| Path::new("/"))
            .to_path_buf();
        (dir, canon)
    } else if path.is_dir() {
        let canon = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let manifest = canon.join("Cargo.toml");
        if !manifest.is_file() {
            return Err(format!("no Cargo.toml found in directory {:?}.", path));
        }
        (canon, manifest)
    } else {
        return Err(format!("path {:?} does not exist.", path));
    };

    let start = Instant::now();
    let base_out_dir = match out_dir_opt {
        Some(d) => {
            if d.is_absolute() {
                d
            } else {
                std::env::current_dir()
                    .unwrap_or_else(|_| PathBuf::from("."))
                    .join(d)
            }
        }
        None => work_dir.join("target/ridx"),
    };
    if let Err(e) = fs::create_dir_all(&base_out_dir) {
        return Err(format!(
            "failed to create output directory {:?}: {e}",
            base_out_dir
        ));
    }
    let base_out_dir = base_out_dir.canonicalize().unwrap_or(base_out_dir);

    let driver_bin = match find_driver_binary() {
        Some(b) => b,
        None => {
            return Err("could not find ridx-driver binary. Please build ridx-driver first (e.g. cargo build -p ridx-driver) or place it in PATH.".to_string());
        }
    };

    let profile_name = if release { "release" } else { "dev" };
    let variant_id = compute_variant_id(
        profile_name,
        target.as_deref(),
        package.as_deref(),
        features.as_deref(),
        no_default_features,
        all_features,
        &manifest_path,
    );

    let variant_rel_dir = format!("variants/{}", variant_id);
    let variant_out_dir = base_out_dir.join(&variant_rel_dir);
    if let Err(e) = fs::create_dir_all(&variant_out_dir) {
        return Err(format!(
            "failed to create variant directory {:?}: {e}",
            variant_out_dir
        ));
    }
    let variant_out_dir = variant_out_dir.canonicalize().unwrap_or(variant_out_dir);

    // Isolate Cargo target cache per variant so Cargo unconditionally executes wrapper on variant switch
    let cargo_target_dir = work_dir.join("target/ridx_cargo").join(&variant_id);
    let _ = fs::create_dir_all(&cargo_target_dir);

    eprintln!(
        "{}",
        "═══════════════════════════════════════════════════════════════════════".dimmed()
    );
    eprintln!(
        "  {} {}",
        "RIDX Workspace Indexer:".bold(),
        work_dir.display().to_string().cyan()
    );
    eprintln!(
        "{}",
        "═══════════════════════════════════════════════════════════════════════".dimmed()
    );
    eprintln!(
        "  Work Directory : {}",
        work_dir.display().to_string().green()
    );
    eprintln!(
        "  Manifest Path  : {}",
        manifest_path.display().to_string().green()
    );
    eprintln!(
        "  Driver Binary  : {}",
        driver_bin.display().to_string().green()
    );
    eprintln!(
        "  Index Output   : {}",
        variant_out_dir.display().to_string().green()
    );
    eprintln!("  Active Variant : {}", variant_id.cyan().bold());
    eprintln!("  Build Profile  : {}", profile_name.cyan());
    if let Some(ref feats) = features {
        eprintln!("  Features       : {}", feats.cyan());
    }
    if no_default_features {
        eprintln!("  No Default Feat: true");
    }
    if all_features {
        eprintln!("  All Features   : true");
    }
    if let Some(ref tgt) = target {
        eprintln!("  Target Triple  : {}", tgt.cyan());
    }
    eprintln!("  Action         : cargo check with RUSTC_WRAPPER...");

    let mut cmd = Command::new("cargo");
    cmd.current_dir(&work_dir);
    cmd.arg("check");
    cmd.arg("--manifest-path").arg(&manifest_path);
    if let Some(ref p) = package {
        cmd.arg("-p").arg(p);
    } else {
        cmd.arg("--workspace");
    }
    if release {
        cmd.arg("--release");
    }
    if let Some(ref feats) = features {
        cmd.arg("--features").arg(feats);
    }
    if no_default_features {
        cmd.arg("--no-default-features");
    }
    if all_features {
        cmd.arg("--all-features");
    }
    if let Some(ref tgt) = target {
        cmd.arg("--target").arg(tgt);
    }

    let cache_dir = ridx_format::resolve_cache_dir();
    let _ = fs::create_dir_all(cache_dir.join("objects"));

    cmd.env("RIDX_OUT_DIR", &variant_out_dir);
    cmd.env("RIDX_CACHE_DIR", &cache_dir);
    cmd.env("RIDX_WORKSPACE_ROOT", &work_dir);
    cmd.env("CARGO_TARGET_DIR", &cargo_target_dir);
    cmd.env("RUSTC_BOOTSTRAP", "1");
    if std::env::var("RIDX_NO_DEPS").is_err() {
        cmd.env("RUSTC_WRAPPER", &driver_bin);
        cmd.env("RUSTC_WORKSPACE_WRAPPER", "");
    } else {
        cmd.env("RUSTC_WORKSPACE_WRAPPER", &driver_bin);
        cmd.env("RUSTC_WRAPPER", "");
    }

    // Ensure librustc_driver shared library can be located by dynamic linker
    if let Ok(out) = Command::new("rustc").args(["--print", "sysroot"]).output() {
        if out.status.success() {
            let sysroot = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let lib_dir = PathBuf::from(&sysroot).join("lib");
            let mut ld_path = lib_dir.to_string_lossy().to_string();
            if let Ok(existing) = std::env::var("LD_LIBRARY_PATH") {
                ld_path = format!("{}:{}", ld_path, existing);
            }
            cmd.env("LD_LIBRARY_PATH", &ld_path);
            cmd.env("DYLD_FALLBACK_LIBRARY_PATH", &ld_path);
        }
    }

    let status = match cmd.status() {
        Ok(s) => s,
        Err(e) => {
            return Err(format!("failed to execute cargo check: {e}"));
        }
    };

    let elapsed = start.elapsed();

    if !status.success() {
        return Err(format!("cargo check exited with status {status}"));
    }

    let ws = match WorkspaceIndex::open(&variant_out_dir) {
        Ok(w) => w,
        Err(e) => {
            return Err(format!(
                "failed to open generated variant index {:?}: {e}",
                variant_out_dir
            ));
        }
    };

    let stats = ws.stats();

    // Record build variant fingerprint manifest atomically in base_out_dir
    let manifest_file = base_out_dir.join("ridx_manifest.json");

    // Collect external dependency shards recorded by compiler driver
    let mut dep_shards: Vec<ridx_format::DependencyShardInfo> = Vec::new();
    let deps_dir = variant_out_dir.join(".deps");
    if deps_dir.is_dir() {
        if let Ok(entries) = fs::read_dir(&deps_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) == Some("json") {
                    if let Ok(content) = fs::read(&path) {
                        if let Ok(mut info) =
                            serde_json::from_slice::<ridx_format::DependencyShardInfo>(&content)
                        {
                            if info.is_dependency {
                                let obj_p = Path::new(&info.object_path);
                                if obj_p.is_file() {
                                    if let Ok(r) = ridx_format::IndexReader::open(obj_p) {
                                        let bin_gen = r.generation();
                                        if !bin_gen.is_empty() {
                                            let sidecar_gen =
                                                info.generation.as_deref().unwrap_or("");
                                            let gen_matches = if !sidecar_gen.is_empty() {
                                                sidecar_gen == bin_gen
                                            } else if !info.file_hashes.is_empty() {
                                                let computed_digest =
                                                    ridx_format::compute_source_snapshot_digest(
                                                        &info.file_hashes,
                                                    );
                                                format!("{:016x}", computed_digest) == bin_gen
                                            } else {
                                                false
                                            };
                                            if !gen_matches {
                                                info.file_hashes.clear();
                                                info.dependencies.clear();
                                                info.extern_units.clear();
                                            }
                                            info.generation = Some(bin_gen.to_string());
                                        }
                                        let embedded = r.embedded_file_hashes();
                                        let sroot_path = info.source_root.as_deref().map(Path::new);
                                        ridx_format::merge_embedded_hashes_authoritative(
                                            &mut info.file_hashes,
                                            &embedded,
                                            sroot_path,
                                        );
                                    }
                                }
                                dep_shards.push(info);
                            }
                        }
                    }
                }
            }
        }
    }
    dep_shards.sort_by(|a, b| a.name.cmp(&b.name));

    let config_files = find_cargo_configs(&work_dir);
    let mut config_target = None;
    for cfg in &config_files {
        if let Ok(content) = fs::read_to_string(cfg) {
            if let Some(t) = extract_config_target(&content) {
                config_target = Some(t);
                break;
            }
        }
    }
    let env_target = std::env::var("CARGO_BUILD_TARGET").ok();
    let effective_target = target.or(env_target).or(config_target);

    let manifest_data = serde_json::json!({
        "version": "2.0",
        "format_version": ridx_format::CURRENT_FORMAT_VERSION,
        "extractor_version": ridx_format::EXTRACTOR_VERSION,
        "active_variant_id": variant_id,
        "variant_dir": variant_rel_dir,
        "base_out_dir": base_out_dir.display().to_string(),
        "workspace_root": work_dir.display().to_string(),
        "manifest_path": manifest_path.display().to_string(),
        "profile": profile_name,
        "target": effective_target,
        "features": features,
        "no_default_features": no_default_features,
        "all_features": all_features,
        "package_filter": package,
        "timestamp_epoch_secs": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        "total_crates": stats.total_crates,
        "total_defs": stats.total_defs,
        "total_refs": stats.total_refs,
        "total_types": stats.total_types,
        "dependency_shards": dep_shards,
        "crates": stats.crate_summaries.iter().map(|c| serde_json::json!({
            "name": c.name,
            "version": c.version,
            "file": c.path.file_name().and_then(|s| s.to_str()).unwrap_or(""),
            "file_size": c.file_size,
            "is_dependency": c.is_dependency,
        })).collect::<Vec<_>>()
    });

    let serialized = serde_json::to_string_pretty(&manifest_data).unwrap();

    // Also save variant-specific manifest inside variant_out_dir
    let variant_manifest_file = variant_out_dir.join("variant_manifest.json");
    let _ = fs::write(&variant_manifest_file, &serialized);

    // Write to temp file then rename for atomic manifest swap
    let tmp_manifest = base_out_dir.join(format!("ridx_manifest.json.tmp.{}", std::process::id()));
    if let Err(e) = fs::write(&tmp_manifest, serialized) {
        return Err(format!(
            "failed to write temporary variant manifest {:?}: {e}",
            tmp_manifest
        ));
    }

    if let Err(e) = fs::rename(&tmp_manifest, &manifest_file) {
        let _ = fs::remove_file(&tmp_manifest);
        return Err(format!(
            "failed to publish active variant manifest {:?}: {e}",
            manifest_file
        ));
    }

    eprintln!(
        "\n{} Indexed workspace in {:.2} s (active variant: {})",
        "Success:".green().bold(),
        elapsed.as_secs_f64(),
        variant_id.cyan().bold()
    );

    eprintln!(
        "Indexed {} crate(s), {} total definitions, {} references, {} types ({:.2} KB total index size)",
        stats.total_crates.to_string().bold(),
        stats.total_defs.to_string().green(),
        stats.total_refs.to_string().green(),
        stats.total_types.to_string().green(),
        stats.total_bytes as f64 / 1024.0
    );

    if !dep_shards.is_empty() {
        eprintln!(
            "Linked {} external dependency shard(s) from global cache",
            dep_shards.len().to_string().cyan().bold()
        );
    }

    Ok(PublishedVariant {
        base_out_dir,
        variant_out_dir,
        variant_id,
    })
}

pub fn run_indexing(opts: IndexOptions) -> PublishedVariant {
    match try_run_indexing(opts) {
        Ok(pv) => pv,
        Err(e) => {
            eprintln!("{}: {e}", "Error".red().bold());
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) static INDEXER_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    pub(crate) fn lock_indexer_test() -> std::sync::MutexGuard<'static, ()> {
        INDEXER_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    struct TestDir(PathBuf);
    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn new_test_dir(prefix: &str) -> TestDir {
        static CNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let id = CNT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!("{}_{}_{}", prefix, std::process::id(), id));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        TestDir(path)
    }

    #[test]
    fn test_extract_config_target() {
        let content1 = r#"
            # Some comments
            [build]
            target = "x86_64-unknown-linux-gnu"
        "#;
        assert_eq!(
            extract_config_target(content1),
            Some("x86_64-unknown-linux-gnu".to_string())
        );

        let content_single_quote = r#"
            [build]
            target = 'aarch64-apple-darwin'
        "#;
        assert_eq!(
            extract_config_target(content_single_quote),
            Some("aarch64-apple-darwin".to_string())
        );

        let content_other_section = r#"
            [target.x86_64-unknown-linux-gnu]
            rustflags = ["-C", "opt-level=3"]
        "#;
        assert_eq!(extract_config_target(content_other_section), None);

        let content_empty = "";
        assert_eq!(extract_config_target(content_empty), None);
    }

    #[test]
    fn test_resolve_cargo_home() {
        let _lock = lock_indexer_test();
        let home = resolve_cargo_home();
        assert!(home.is_some());
    }

    #[test]
    fn test_find_cargo_configs() {
        let test_dir = new_test_dir("ridx_cfg_test");
        let cargo_dir = test_dir.0.join(".cargo");
        fs::create_dir_all(&cargo_dir).unwrap();
        let cfg_file = cargo_dir.join("config.toml");
        fs::write(&cfg_file, "[build]\ntarget = \"x86_64-linux-android\"\n").unwrap();

        let configs = find_cargo_configs(&test_dir.0);
        assert!(configs.contains(&cfg_file));
    }

    #[test]
    fn test_collect_all_workspace_manifests() {
        let test_dir = new_test_dir("ridx_manifest_scan");
        let root_cargo = test_dir.0.join("Cargo.toml");
        fs::write(&root_cargo, "[workspace]\nmembers = [\"crate_a\"]\n").unwrap();

        let sub_crate = test_dir.0.join("crate_a");
        fs::create_dir_all(&sub_crate).unwrap();
        let sub_cargo = sub_crate.join("Cargo.toml");
        fs::write(
            &sub_cargo,
            "[package]\nname = \"crate_a\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();

        let target_dir = test_dir.0.join("target/sub");
        fs::create_dir_all(&target_dir).unwrap();
        let target_cargo = target_dir.join("Cargo.toml");
        fs::write(&target_cargo, "invalid").unwrap();

        let mut manifests = Vec::new();
        collect_all_workspace_manifests(&test_dir.0, &mut manifests);

        assert!(manifests.contains(&root_cargo));
        assert!(manifests.contains(&sub_cargo));
        assert!(!manifests.contains(&target_cargo));
    }

    #[test]
    fn test_compute_variant_id_permutations() {
        let _lock = lock_indexer_test();
        let test_dir = new_test_dir("ridx_variant_test");
        let root_cargo = test_dir.0.join("Cargo.toml");
        fs::write(
            &root_cargo,
            "[package]\nname = \"foo\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();

        let v1 = compute_variant_id("dev", None, None, None, false, false, &root_cargo);
        let v2 = compute_variant_id("release", None, None, None, false, false, &root_cargo);
        assert_ne!(v1, v2);

        let v3 = compute_variant_id("dev", Some("x86_64"), None, None, false, false, &root_cargo);
        assert_ne!(v1, v3);

        let v4 = compute_variant_id("dev", None, Some("foo"), None, false, false, &root_cargo);
        assert_ne!(v1, v4);

        let v5 = compute_variant_id("dev", None, None, Some("feat_a"), false, false, &root_cargo);
        assert_ne!(v1, v5);

        let v6 = compute_variant_id("dev", None, None, None, true, false, &root_cargo);
        assert_ne!(v1, v6);

        let v7 = compute_variant_id("dev", None, None, None, false, true, &root_cargo);
        assert_ne!(v1, v7);

        // Idempotency: same inputs give identical variant id
        let v1_again = compute_variant_id("dev", None, None, None, false, false, &root_cargo);
        assert_eq!(v1, v1_again);
    }

    #[test]
    fn test_find_driver_binary() {
        let _ = find_driver_binary();
    }

    #[test]
    fn test_compute_variant_id_with_lock_and_env() {
        let _lock = lock_indexer_test();
        let test_dir = new_test_dir("ridx_variant_lock");
        let root_cargo = test_dir.0.join("Cargo.toml");
        fs::write(
            &root_cargo,
            "[package]\nname = \"foo\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        let root_lock = test_dir.0.join("Cargo.lock");
        fs::write(&root_lock, "# Lock file content\nversion = 3\n").unwrap();

        let v = compute_variant_id("dev", Some("wasm32"), None, None, false, false, &root_cargo);
        assert!(!v.is_empty());
    }

    #[test]
    fn test_published_variant_and_options() {
        let opts = IndexOptions {
            path: PathBuf::from("."),
            out_dir_opt: None,
            package: Some("test_pkg".to_string()),
            release: true,
            features: Some("feat".to_string()),
            no_default_features: true,
            all_features: false,
            target: Some("x86_64".to_string()),
        };
        assert_eq!(opts.package.as_deref(), Some("test_pkg"));

        let pv = PublishedVariant {
            base_out_dir: PathBuf::from("/target/ridx"),
            variant_out_dir: PathBuf::from("/target/ridx/variants/dev-123"),
            variant_id: "dev-123".to_string(),
        };
        let debug_str = format!("{pv:?}");
        assert!(debug_str.contains("dev-123"));
    }

    #[test]
    fn test_run_indexing_on_sample_crate() {
        let _lock = lock_indexer_test();
        let test_dir = new_test_dir("ridx_run_index_test");
        let cargo_toml = test_dir.0.join("Cargo.toml");
        fs::write(
            &cargo_toml,
            "[package]\nname = \"sample_crate\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        let src_dir = test_dir.0.join("src");
        fs::create_dir_all(&src_dir).unwrap();
        fs::write(
            src_dir.join("lib.rs"),
            "pub fn add(a: i32, b: i32) -> i32 { a + b }\n",
        )
        .unwrap();

        if find_driver_binary().is_some() {
            let out = run_indexing(IndexOptions {
                path: test_dir.0.clone(),
                out_dir_opt: None,
                package: None,
                release: false,
                features: None,
                no_default_features: false,
                all_features: false,
                target: None,
            });
            assert!(out.variant_out_dir.exists());

            // Also test passing manifest file directly
            let out_file = run_indexing(IndexOptions {
                path: cargo_toml,
                out_dir_opt: Some(test_dir.0.join("custom_out")),
                package: None,
                release: false,
                features: None,
                no_default_features: false,
                all_features: false,
                target: None,
            });
            assert!(out_file.variant_out_dir.exists());
        }
    }

    #[test]
    fn test_try_run_indexing_error_cases() {
        let test_dir = new_test_dir("ridx_err_cases");
        let non_existent = test_dir.0.join("non_existent");
        let res_missing = try_run_indexing(IndexOptions {
            path: non_existent,
            out_dir_opt: None,
            package: None,
            release: false,
            features: None,
            no_default_features: false,
            all_features: false,
            target: None,
        });
        assert!(res_missing.is_err());

        let not_cargo = test_dir.0.join("not_cargo.txt");
        fs::write(&not_cargo, "hello").unwrap();
        let res_not_cargo = try_run_indexing(IndexOptions {
            path: not_cargo,
            out_dir_opt: None,
            package: None,
            release: false,
            features: None,
            no_default_features: false,
            all_features: false,
            target: None,
        });
        assert!(res_not_cargo.is_err());

        let empty_dir = test_dir.0.join("empty_dir");
        fs::create_dir_all(&empty_dir).unwrap();
        let res_empty = try_run_indexing(IndexOptions {
            path: empty_dir,
            out_dir_opt: None,
            package: None,
            release: false,
            features: None,
            no_default_features: false,
            all_features: false,
            target: None,
        });
        assert!(res_empty.is_err());
    }

    #[test]
    fn test_try_run_indexing_with_options_and_deps() {
        let _lock = lock_indexer_test();
        let test_dir = new_test_dir("ridx_opts_test");
        let cargo_toml = test_dir.0.join("Cargo.toml");
        fs::write(
            &cargo_toml,
            "[package]\nname = \"opt_crate\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[features]\nfeat_a = []\n",
        )
        .unwrap();
        let src_dir = test_dir.0.join("src");
        fs::create_dir_all(&src_dir).unwrap();
        fs::write(
            src_dir.join("lib.rs"),
            "#[cfg(feature = \"feat_a\")]\npub fn feat() {}\n",
        )
        .unwrap();

        if find_driver_binary().is_some() {
            let res = try_run_indexing(IndexOptions {
                path: test_dir.0.clone(),
                out_dir_opt: None,
                package: Some("opt_crate".to_string()),
                release: false,
                features: Some("feat_a".to_string()),
                no_default_features: true,
                all_features: true,
                target: None,
            });
            assert!(res.is_ok());

            // Create mock .deps files in variant_out_dir to test external dependency shard parsing and validation
            let pv = res.unwrap();
            let deps_dir = pv.variant_out_dir.join(".deps");
            fs::create_dir_all(&deps_dir).unwrap();

            let dep_ridx = test_dir.0.join("external_dep.ridx");
            let mut writer = ridx_format::IndexWriter::new("external_dep", "1.0.0");
            writer.set_unit_key("ext_key");
            writer.set_generation("gen123");
            let mut f = fs::File::create(&dep_ridx).unwrap();
            writer.write_to(&mut f).unwrap();
            drop(f);

            let mock_shard_info = serde_json::json!({
                "name": "external_dep",
                "version": "1.0.0",
                "unit_key": "ext_key",
                "object_path": dep_ridx.display().to_string(),
                "source_root": test_dir.0.display().to_string(),
                "is_dependency": true,
                "generation": "gen123",
                "file_hashes": { "src/lib.rs": 12345 },
                "dependencies": ["dep_b"],
                "extern_units": { "dep_b": "unit_b" }
            });
            fs::write(
                deps_dir.join("external_dep.json"),
                mock_shard_info.to_string(),
            )
            .unwrap();

            let mismatch_shard_info = serde_json::json!({
                "name": "mismatch_dep",
                "version": "1.0.0",
                "unit_key": "ext_key2",
                "object_path": dep_ridx.display().to_string(),
                "source_root": null,
                "is_dependency": true,
                "generation": "wrong_gen",
                "file_hashes": { "src/lib.rs": 99999 },
                "dependencies": ["dep_c"],
                "extern_units": { "dep_c": "unit_c" }
            });
            fs::write(
                deps_dir.join("mismatch_dep.json"),
                mismatch_shard_info.to_string(),
            )
            .unwrap();

            let hash_gen_shard_info = serde_json::json!({
                "name": "hash_gen_dep",
                "version": "1.0.0",
                "unit_key": "ext_key3",
                "object_path": dep_ridx.display().to_string(),
                "source_root": null,
                "is_dependency": true,
                "generation": null,
                "file_hashes": { "src/lib.rs": 12345 },
                "dependencies": [],
                "extern_units": {}
            });
            fs::write(
                deps_dir.join("hash_gen_dep.json"),
                hash_gen_shard_info.to_string(),
            )
            .unwrap();

            // Run again with matching options to cover dependency shard collection and validation logic
            let res2 = try_run_indexing(IndexOptions {
                path: test_dir.0.clone(),
                out_dir_opt: Some(pv.base_out_dir),
                package: Some("opt_crate".to_string()),
                release: false,
                features: Some("feat_a".to_string()),
                no_default_features: true,
                all_features: true,
                target: None,
            });
            assert!(res2.is_ok());
        }
    }

    #[test]
    fn test_indexer_helpers_and_error_paths() {
        let _lock = lock_indexer_test();
        // 1. try_run_indexing errors
        let non_existent = PathBuf::from("/nonexistent_path_for_indexer");
        assert!(try_run_indexing(IndexOptions {
            path: non_existent,
            out_dir_opt: None,
            package: None,
            release: false,
            features: None,
            no_default_features: false,
            all_features: false,
            target: None,
        })
        .is_err());

        let temp = new_test_dir("ridx_err_tests");
        let not_cargo = temp.0.join("not_cargo.txt");
        fs::write(&not_cargo, "hello").unwrap();
        assert!(try_run_indexing(IndexOptions {
            path: not_cargo,
            out_dir_opt: None,
            package: None,
            release: false,
            features: None,
            no_default_features: false,
            all_features: false,
            target: None,
        })
        .is_err());

        let no_manifest_dir = temp.0.join("empty_dir");
        fs::create_dir_all(&no_manifest_dir).unwrap();
        assert!(try_run_indexing(IndexOptions {
            path: no_manifest_dir,
            out_dir_opt: None,
            package: None,
            release: false,
            features: None,
            no_default_features: false,
            all_features: false,
            target: None,
        })
        .is_err());

        // 2. extract_config_target
        let config_toml = "[build]\ntarget = \"x86_64-unknown-linux-gnu\"\n";
        assert_eq!(
            extract_config_target(config_toml),
            Some("x86_64-unknown-linux-gnu".to_string())
        );
        assert_eq!(extract_config_target("[other]\ntarget = \"abc\""), None);
        assert_eq!(extract_config_target("# [build]\n# target = 1"), None);
        assert_eq!(extract_config_target("[build]\ntarget = \"\""), None);
        assert_eq!(extract_config_target("[build]\ntarget = ''"), None);

        // 3. find_cargo_configs & collect_all_workspace_manifests
        let ws_dir = temp.0.join("ws");
        let cargo_dir = ws_dir.join(".cargo");
        fs::create_dir_all(&cargo_dir).unwrap();
        fs::write(cargo_dir.join("config.toml"), config_toml).unwrap();
        let configs = find_cargo_configs(&ws_dir);
        assert!(!configs.is_empty());

        let member_dir = ws_dir.join("crates/member");
        fs::create_dir_all(&member_dir).unwrap();
        fs::write(
            member_dir.join("Cargo.toml"),
            "[package]\nname = \"member\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        let mut manifests = Vec::new();
        collect_all_workspace_manifests(&ws_dir, &mut manifests);
        assert!(!manifests.is_empty());

        let mut empty_manifests = Vec::new();
        collect_all_workspace_manifests(
            &PathBuf::from("/nonexistent_dir_for_test"),
            &mut empty_manifests,
        );
        assert!(empty_manifests.is_empty());

        // 4. resolve_cargo_home with CARGO_HOME and HOME variants
        let orig_cargo_home = std::env::var("CARGO_HOME").ok();
        let orig_home = std::env::var("HOME").ok();
        std::env::set_var("CARGO_HOME", "/custom/cargo/home");
        assert_eq!(
            resolve_cargo_home(),
            Some(PathBuf::from("/custom/cargo/home"))
        );
        std::env::remove_var("CARGO_HOME");
        std::env::set_var("HOME", "/custom/home");
        assert_eq!(
            resolve_cargo_home(),
            Some(PathBuf::from("/custom/home/.cargo"))
        );
        std::env::set_var("CARGO_HOME", "");
        std::env::set_var("HOME", "");
        assert_eq!(resolve_cargo_home(), None);
        std::env::set_var("CARGO_HOME", "   ");
        std::env::set_var("HOME", "   ");
        assert_eq!(resolve_cargo_home(), None);

        if let Some(ch) = orig_cargo_home {
            std::env::set_var("CARGO_HOME", ch);
        } else {
            std::env::remove_var("CARGO_HOME");
        }
        if let Some(h) = orig_home {
            std::env::set_var("HOME", h);
        } else {
            std::env::remove_var("HOME");
        }

        // 5. find_driver_binary with RIDX_DRIVER_BIN env var and fallbacks
        let dummy_driver = temp.0.join("mock_driver");
        fs::write(&dummy_driver, b"#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&dummy_driver, fs::Permissions::from_mode(0o755));
        }
        std::env::set_var("RIDX_DRIVER_BIN", &dummy_driver);
        let found = find_driver_binary();
        assert_eq!(found, dummy_driver.canonicalize().ok());
        std::env::remove_var("RIDX_DRIVER_BIN");
        let _ = find_driver_binary();

        // 6. Direct Cargo.toml path and relative out_dir_opt
        let direct_manifest = temp.0.join("direct_crate/Cargo.toml");
        fs::create_dir_all(direct_manifest.parent().unwrap()).unwrap();
        fs::write(
            &direct_manifest,
            "[package]\nname = \"direct_crate\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[lib]\npath = \"lib.rs\"\n",
        )
        .unwrap();
        fs::write(
            temp.0.join("direct_crate/lib.rs"),
            "pub fn direct_fn() {}\n",
        )
        .unwrap();

        std::env::set_var("RIDX_NO_DEPS", "1");
        let cur_d = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let rel_name = format!("rel_out_dir_for_test_{}", std::process::id());
        let expected_rel_dir = cur_d.join(&rel_name);
        let _ = try_run_indexing(IndexOptions {
            path: direct_manifest.clone(),
            out_dir_opt: Some(PathBuf::from(&rel_name)),
            package: None,
            release: true,
            features: None,
            no_default_features: false,
            all_features: false,
            target: Some("x86_64-unknown-linux-gnu".to_string()),
        });
        std::env::remove_var("RIDX_NO_DEPS");
        let _ = fs::remove_dir_all(&expected_rel_dir);
        let _ = fs::remove_dir_all(&rel_name);

        // 7. Output directory creation failure (lines 327-330)
        let bad_out_file = temp.0.join("bad_out_file");
        fs::write(&bad_out_file, "file").unwrap();
        let err_out = try_run_indexing(IndexOptions {
            path: direct_manifest.clone(),
            out_dir_opt: Some(bad_out_file.join("sub")),
            package: None,
            release: false,
            features: None,
            no_default_features: false,
            all_features: false,
            target: None,
        });
        assert!(err_out.is_err());
        assert!(err_out
            .unwrap_err()
            .contains("failed to create output directory"));

        // 8. Driver binary not found error (line 337)
        std::env::set_var("RIDX_DRIVER_BIN", "/nonexistent/mock/driver/path");
        let orig_path = std::env::var("PATH").ok();
        std::env::set_var("PATH", "");
        let err_driver = try_run_indexing(IndexOptions {
            path: direct_manifest.clone(),
            out_dir_opt: Some(temp.0.join("out_test")),
            package: None,
            release: false,
            features: None,
            no_default_features: false,
            all_features: false,
            target: None,
        });
        if let Some(p) = orig_path {
            std::env::set_var("PATH", p);
        } else {
            std::env::remove_var("PATH");
        }
        std::env::remove_var("RIDX_DRIVER_BIN");
        assert!(err_driver.is_err());

        // 9. compute_variant_id with RUSTFLAGS and CARGO_ENCODED_RUSTFLAGS (lines 248-253)
        std::env::set_var("RUSTFLAGS", "-C opt-level=3");
        std::env::set_var("CARGO_ENCODED_RUSTFLAGS", "-C\x1fopt-level=3");
        let v_flags = compute_variant_id("dev", None, None, None, false, false, &direct_manifest);
        assert!(!v_flags.is_empty());
        std::env::remove_var("RUSTFLAGS");
        std::env::remove_var("CARGO_ENCODED_RUSTFLAGS");
    }

    #[test]
    fn test_indexer_dependency_shards_and_cargo_config() {
        let _lock = lock_indexer_test();
        let temp = new_test_dir("ridx_dep_shards_test");
        let work_dir = temp.0.join("sample_pkg");
        let src_dir = work_dir.join("src");
        fs::create_dir_all(&src_dir).unwrap();

        let manifest_file = work_dir.join("Cargo.toml");
        fs::write(
            &manifest_file,
            "[package]\nname = \"sample_pkg\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        fs::write(src_dir.join("lib.rs"), "pub fn sample() {}\n").unwrap();

        // 1. Cargo config with build target (lines 551-560)
        let dot_cargo = work_dir.join(".cargo");
        fs::create_dir_all(&dot_cargo).unwrap();
        fs::write(
            dot_cargo.join("config.toml"),
            "[build]\ntarget = \"x86_64-unknown-linux-gnu\"\n",
        )
        .unwrap();

        let out_dir = temp.0.join("target_ridx");
        let opts = IndexOptions {
            path: work_dir.clone(),
            out_dir_opt: Some(out_dir.clone()),
            package: None,
            release: false,
            features: None,
            no_default_features: false,
            all_features: false,
            target: None,
        };

        // Create mock dependency shard
        let dep_file = temp.0.join("dep.ridx");
        let mut w_dep = ridx_format::IndexWriter::new("dep_crate", "0.1.0");
        w_dep.set_unit_key("dep_crate_key");
        w_dep.set_generation("gen_alpha");
        let mut f_dep = fs::File::create(&dep_file).unwrap();
        w_dep.write_to(&mut f_dep).unwrap();
        drop(f_dep);

        // Prepopulate .deps directory with DependencyShardInfo (lines 506-548)
        let variant_id = compute_variant_id(
            "dev",
            Some("x86_64-unknown-linux-gnu"),
            None,
            None,
            false,
            false,
            &manifest_file,
        );
        let variant_out_dir = out_dir.join(format!("variants/{}", variant_id));
        let deps_dir = variant_out_dir.join(".deps");
        fs::create_dir_all(&deps_dir).unwrap();

        let dep_info = ridx_format::DependencyShardInfo {
            name: "dep_crate".to_string(),
            version: "0.1.0".to_string(),
            unit_key: "dep_crate_key".to_string(),
            generation: Some("gen_alpha".to_string()),
            object_path: dep_file.display().to_string(),
            source_root: Some(temp.0.display().to_string()),
            is_dependency: true,
            dependencies: vec![],
            extern_units: std::collections::HashMap::new(),
            file_hashes: std::collections::HashMap::new(),
        };
        fs::write(
            deps_dir.join("dep_crate.json"),
            serde_json::to_string(&dep_info).unwrap(),
        )
        .unwrap();

        // Shard with generation mismatch (lines 526-528)
        let dep_info_mismatch = ridx_format::DependencyShardInfo {
            name: "dep_mismatch".to_string(),
            version: "0.1.0".to_string(),
            unit_key: "dep_crate_key".to_string(),
            generation: Some("mismatched_gen".to_string()),
            object_path: dep_file.display().to_string(),
            source_root: Some(temp.0.display().to_string()),
            is_dependency: true,
            dependencies: vec!["some_dep".to_string()],
            extern_units: std::collections::HashMap::new(),
            file_hashes: std::collections::HashMap::new(),
        };
        fs::write(
            deps_dir.join("dep_mismatch.json"),
            serde_json::to_string(&dep_info_mismatch).unwrap(),
        )
        .unwrap();

        // Shard with None generation and empty file_hashes (line 523)
        let dep_info_empty_hashes = ridx_format::DependencyShardInfo {
            name: "dep_empty_h".to_string(),
            version: "0.1.0".to_string(),
            unit_key: "dep_crate_key".to_string(),
            generation: None,
            object_path: dep_file.display().to_string(),
            source_root: Some(temp.0.display().to_string()),
            is_dependency: true,
            dependencies: vec![],
            extern_units: std::collections::HashMap::new(),
            file_hashes: std::collections::HashMap::new(),
        };
        fs::write(
            deps_dir.join("dep_empty_h.json"),
            serde_json::to_string(&dep_info_empty_hashes).unwrap(),
        )
        .unwrap();

        // Shard with generation: None and computed digest matching bin_gen (lines 518-525)
        let mut f_hashes = std::collections::HashMap::new();
        f_hashes.insert("src/lib.rs".to_string(), 12345u64);
        let computed_digest = ridx_format::compute_source_snapshot_digest(&f_hashes);
        let computed_gen = format!("{:016x}", computed_digest);

        let dep_matching_file = temp.0.join("dep_matching.ridx");
        let mut w_matching = ridx_format::IndexWriter::new("dep_matching", "0.1.0");
        w_matching.set_unit_key("dep_matching_key");
        w_matching.set_generation(&computed_gen);
        let mut fm = fs::File::create(&dep_matching_file).unwrap();
        w_matching.write_to(&mut fm).unwrap();
        drop(fm);

        let dep_info_matching = ridx_format::DependencyShardInfo {
            name: "dep_matching".to_string(),
            version: "0.1.0".to_string(),
            unit_key: "dep_matching_key".to_string(),
            generation: None,
            object_path: dep_matching_file.display().to_string(),
            source_root: Some(temp.0.display().to_string()),
            is_dependency: true,
            dependencies: vec![],
            extern_units: std::collections::HashMap::new(),
            file_hashes: f_hashes.clone(),
        };
        fs::write(
            deps_dir.join("dep_matching.json"),
            serde_json::to_string(&dep_info_matching).unwrap(),
        )
        .unwrap();

        // Shard with generation: None and computed digest NOT matching bin_gen (lines 514-518)
        let dep_mismatch_gen_file = temp.0.join("dep_mismatch_gen.ridx");
        let mut w_mismatch_gen = ridx_format::IndexWriter::new("dep_mismatch_gen", "0.1.0");
        w_mismatch_gen.set_unit_key("dep_mismatch_gen_key");
        w_mismatch_gen.set_generation("some_other_bin_gen");
        let mut f_mg = fs::File::create(&dep_mismatch_gen_file).unwrap();
        w_mismatch_gen.write_to(&mut f_mg).unwrap();
        drop(f_mg);

        let dep_info_mismatch_gen = ridx_format::DependencyShardInfo {
            name: "dep_mismatch_gen".to_string(),
            version: "0.1.0".to_string(),
            unit_key: "dep_mismatch_gen_key".to_string(),
            generation: None,
            object_path: dep_mismatch_gen_file.display().to_string(),
            source_root: Some(temp.0.display().to_string()),
            is_dependency: true,
            dependencies: vec!["some_dep".to_string()],
            extern_units: std::collections::HashMap::new(),
            file_hashes: f_hashes.clone(),
        };
        fs::write(
            deps_dir.join("dep_mismatch_gen.json"),
            serde_json::to_string(&dep_info_mismatch_gen).unwrap(),
        )
        .unwrap();

        // Shard with is_dependency: false (lines 544-546)
        let dep_info_not_dep = ridx_format::DependencyShardInfo {
            name: "workspace_shard".to_string(),
            version: "0.1.0".to_string(),
            unit_key: "ws_shard_key".to_string(),
            generation: None,
            object_path: dep_file.display().to_string(),
            source_root: Some(temp.0.display().to_string()),
            is_dependency: false,
            dependencies: vec![],
            extern_units: std::collections::HashMap::new(),
            file_hashes: std::collections::HashMap::new(),
        };
        fs::write(
            deps_dir.join("dep_not_dep.json"),
            serde_json::to_string(&dep_info_not_dep).unwrap(),
        )
        .unwrap();

        // Shard with nonexistent object_path (lines 540-541)
        let dep_info_missing_obj = ridx_format::DependencyShardInfo {
            name: "dep_missing_obj".to_string(),
            version: "0.1.0".to_string(),
            unit_key: "missing_obj_key".to_string(),
            generation: None,
            object_path: "/nonexistent/object.ridx".to_string(),
            source_root: Some(temp.0.display().to_string()),
            is_dependency: true,
            dependencies: vec![],
            extern_units: std::collections::HashMap::new(),
            file_hashes: std::collections::HashMap::new(),
        };
        fs::write(
            deps_dir.join("dep_missing_obj.json"),
            serde_json::to_string(&dep_info_missing_obj).unwrap(),
        )
        .unwrap();

        // Non-json file in .deps (lines 548-549)
        fs::write(deps_dir.join("ignored.txt"), b"not json").unwrap();
        fs::write(deps_dir.join("invalid.json"), b"invalid json").unwrap();

        std::env::set_var("DYLD_FALLBACK_LIBRARY_PATH", "/usr/local/lib");

        if find_driver_binary().is_some() {
            let res = try_run_indexing(opts.clone());
            assert!(res.is_ok());
            let published = res.unwrap();
            let mf = fs::read_to_string(published.base_out_dir.join("ridx_manifest.json")).unwrap();
            assert!(mf.contains("dep_crate"));
            assert!(mf.contains("dep_mismatch"));
            assert!(mf.contains("dep_matching"));

            // Test atomic manifest write temp file failure (lines 610-616)
            let tmp_manifest_blocker =
                out_dir.join(format!("ridx_manifest.json.tmp.{}", std::process::id()));
            if fs::create_dir_all(&tmp_manifest_blocker).is_ok() {
                let err_tmp = try_run_indexing(opts.clone());
                assert!(err_tmp.is_err());
                assert!(err_tmp
                    .unwrap_err()
                    .contains("failed to write temporary variant manifest"));
                let _ = fs::remove_dir_all(&tmp_manifest_blocker);
            }

            // Test atomic manifest publish rename error (lines 618-622)
            let manifest_dest = out_dir.join("ridx_manifest.json");
            let _ = fs::remove_file(&manifest_dest);
            if fs::create_dir_all(manifest_dest.join("nested_blocker")).is_ok() {
                let err_res = try_run_indexing(opts);
                assert!(err_res.is_err());
                assert!(err_res
                    .unwrap_err()
                    .contains("failed to publish active variant manifest"));
                let _ = fs::remove_dir_all(&manifest_dest);
            }
        }

        // Test find_driver_binary fallback paths (lines 56-80)
        let orig_driver = std::env::var("RIDX_DRIVER_BIN").ok();
        std::env::remove_var("RIDX_DRIVER_BIN");
        let _ = find_driver_binary();
        let _ = find_driver_binary_internal(None);

        let exe_dir = temp.0.join("exe_test_dir/nested");
        fs::create_dir_all(&exe_dir).unwrap();
        let mock_driver_sibling = exe_dir.join("ridx-driver");
        fs::write(&mock_driver_sibling, b"").unwrap();
        let dummy_exe = exe_dir.join("dummy_exe");
        let found_sibling = find_driver_binary_internal(Some(&dummy_exe));
        assert_eq!(found_sibling, mock_driver_sibling.canonicalize().ok());
        let _ = fs::remove_file(&mock_driver_sibling);

        let mock_driver_parent = temp.0.join("exe_test_dir/ridx-driver");
        fs::write(&mock_driver_parent, b"").unwrap();
        let found_parent = find_driver_binary_internal(Some(&dummy_exe));
        assert_eq!(found_parent, mock_driver_parent.canonicalize().ok());
        let _ = fs::remove_file(&mock_driver_parent);
        let _ = find_driver_binary_internal(Some(&dummy_exe));

        // Also test with empty directory as cwd to trigger which and None fallback (lines 73-84)
        let orig_cwd = std::env::current_dir().ok();
        let temp_empty = new_test_dir("empty_for_which");
        let mock_target_debug = temp_empty.0.join("target/debug");
        fs::create_dir_all(&mock_target_debug).unwrap();
        let mock_target_driver = mock_target_debug.join("ridx-driver");
        fs::write(&mock_target_driver, b"").unwrap();
        if std::env::set_current_dir(&temp_empty.0).is_ok() {
            let found_cand = find_driver_binary_internal(None);
            assert_eq!(found_cand, mock_target_driver.canonicalize().ok());
            let _ = fs::remove_file(&mock_target_driver);

            // Test which command resolution when driver is in PATH (lines 77-84)
            let mock_bin_dir = new_test_dir("mock_bin_for_which");
            let mock_bin_driver = mock_bin_dir.0.join("ridx-driver");
            fs::write(&mock_bin_driver, b"#!/bin/sh\nexit 0\n").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(&mock_bin_driver, fs::Permissions::from_mode(0o755));
            }
            let orig_path = std::env::var("PATH").ok();
            let new_path = format!(
                "{}:{}",
                mock_bin_dir.0.display(),
                orig_path.as_deref().unwrap_or("")
            );
            std::env::set_var("PATH", &new_path);
            let found_which = find_driver_binary_internal(None);
            assert!(found_which.is_some());
            if let Some(p) = orig_path.as_ref() {
                std::env::set_var("PATH", p);
            } else {
                std::env::remove_var("PATH");
            }

            // Test which command failure with empty PATH (line 84)
            std::env::set_var("PATH", "");
            let found_none = find_driver_binary_internal(None);
            assert!(found_none.is_none());
            if let Some(p) = orig_path {
                std::env::set_var("PATH", p);
            } else {
                std::env::remove_var("PATH");
            }

            let _ = find_driver_binary_internal(None);
            if let Some(c) = orig_cwd {
                let _ = std::env::set_current_dir(c);
            }
        }
        if let Some(d) = orig_driver {
            std::env::set_var("RIDX_DRIVER_BIN", d);
        }

        // Test resolve_cargo_home and find_cargo_configs (lines 85-96, 119-126)
        let orig_home = std::env::var("HOME").ok();
        let orig_cargo_home = std::env::var("CARGO_HOME").ok();

        let custom_home = temp.0.join("custom_home");
        let dot_cargo_home = custom_home.join(".cargo");
        fs::create_dir_all(&dot_cargo_home).unwrap();
        fs::write(dot_cargo_home.join("config.toml"), "[build]\ntarget = ''\n").unwrap();
        fs::write(dot_cargo_home.join("config"), "[build]\n").unwrap();
        std::env::set_var("HOME", custom_home.display().to_string());
        std::env::remove_var("CARGO_HOME");
        assert_eq!(resolve_cargo_home(), Some(dot_cargo_home.clone()));
        let cfgs = find_cargo_configs(&work_dir);
        assert!(cfgs.iter().any(|c| c.ends_with("config.toml")));
        assert_eq!(extract_config_target("[build]\ntarget = ''\n"), None);

        // Test collect_all_workspace_manifests with .git and target subdirs (lines 164-168)
        let sub_ignored = work_dir.join(".git");
        fs::create_dir_all(&sub_ignored).unwrap();
        fs::write(sub_ignored.join("Cargo.toml"), "").unwrap();
        let mut all_mf = Vec::new();
        collect_all_workspace_manifests(&work_dir, &mut all_mf);
        assert!(!all_mf.iter().any(|m| m.starts_with(&sub_ignored)));

        // Test resolve_cargo_home with empty string (line 96)
        std::env::set_var("HOME", "   ");
        std::env::remove_var("CARGO_HOME");
        assert_eq!(resolve_cargo_home(), None);

        if let Some(h) = orig_home {
            std::env::set_var("HOME", h);
        } else {
            std::env::remove_var("HOME");
        }
        if let Some(ch) = orig_cargo_home {
            std::env::set_var("CARGO_HOME", ch);
        } else {
            std::env::remove_var("CARGO_HOME");
        }

        // Test extract_config_target with empty target and other keys (lines 148-149)
        assert_eq!(
            extract_config_target("[build]\njobs = 4\ntarget = ''\n"),
            None
        );

        // Test collect_all_workspace_manifests with hidden dir (line 168)
        let hidden_dir = work_dir.join(".hidden_dir");
        let _ = fs::create_dir_all(&hidden_dir);
        let _ = fs::write(hidden_dir.join("Cargo.toml"), "");

        // Test compute_variant_id with external manifest_path (line 219)
        let outside_manifest = temp.0.join("outside/Cargo.toml");
        let _ = fs::create_dir_all(outside_manifest.parent().unwrap());
        let _ = fs::write(&outside_manifest, "");
        let v_outside =
            compute_variant_id("dev", None, None, None, false, false, &outside_manifest);
        assert!(!v_outside.is_empty());

        // Test compute_variant_id with multiple cargo configs where config_target is already Some (line 198)
        let sub_ws = work_dir.join("sub_ws");
        let _ = fs::create_dir_all(sub_ws.join(".cargo"));
        fs::write(
            sub_ws.join(".cargo/config.toml"),
            "[build]\ntarget = 'x86_64-unknown-linux-gnu'\n",
        )
        .unwrap();
        let v_multi_cfg = compute_variant_id("dev", None, None, None, false, false, &manifest_file);
        assert!(!v_multi_cfg.is_empty());

        // Test CARGO_BUILD_TARGET environment variable (lines 206-208)
        std::env::set_var("CARGO_BUILD_TARGET", "aarch64-unknown-linux-gnu");
        let v_env_target =
            compute_variant_id("dev", None, None, None, false, false, &manifest_file);
        assert!(!v_env_target.is_empty());
        std::env::remove_var("CARGO_BUILD_TARGET");

        // Test Cargo.lock reading in compute_variant_id (lines 238-241)
        let lock_file = work_dir.join("Cargo.lock");
        fs::write(&lock_file, b"version = 3\n").unwrap();
        let v_lock = compute_variant_id("dev", None, None, None, false, false, &manifest_file);
        assert!(!v_lock.is_empty());
        let _ = fs::remove_file(&lock_file);

        // Test RUSTFLAGS and CARGO_ENCODED_RUSTFLAGS in compute_variant_id (lines 253-258)
        std::env::set_var("RUSTFLAGS", "-C target-cpu=native");
        std::env::set_var("CARGO_ENCODED_RUSTFLAGS", "-C\x1ftarget-cpu=native");
        let v_rustflags = compute_variant_id("dev", None, None, None, false, false, &manifest_file);
        assert!(!v_rustflags.is_empty());
        std::env::remove_var("RUSTFLAGS");
        std::env::remove_var("CARGO_ENCODED_RUSTFLAGS");

        // Test try_run_indexing validation errors (lines 280-283, 311, 315)
        let not_cargo = temp.0.join("not_cargo.txt");
        fs::write(&not_cargo, b"hello").unwrap();
        let err_file = try_run_indexing(IndexOptions {
            path: not_cargo,
            out_dir_opt: None,
            package: None,
            release: false,
            features: None,
            no_default_features: false,
            all_features: false,
            target: None,
        });
        assert!(err_file
            .unwrap_err()
            .contains("is a file but not a Cargo.toml manifest"));

        let empty_dir = new_test_dir("empty_dir_no_toml");
        let err_dir = try_run_indexing(IndexOptions {
            path: empty_dir.0.clone(),
            out_dir_opt: None,
            package: None,
            release: false,
            features: None,
            no_default_features: false,
            all_features: false,
            target: None,
        });
        assert!(err_dir
            .unwrap_err()
            .contains("no Cargo.toml found in directory"));

        let err_nonexist = try_run_indexing(IndexOptions {
            path: PathBuf::from("/nonexistent/path/99999"),
            out_dir_opt: None,
            package: None,
            release: false,
            features: None,
            no_default_features: false,
            all_features: false,
            target: None,
        });
        assert!(err_nonexist.unwrap_err().contains("does not exist"));
    }

    #[test]
    fn test_indexer_additional_edge_cases() {
        let _lock = lock_indexer_test();

        let temp = new_test_dir("indexer_add_edges");
        let manifest_file = temp.0.join("Cargo.toml");
        fs::write(
            &manifest_file,
            "[package]\nname = \"add_edges\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();

        // 1. collect_all_workspace_manifests with subdirectory
        let sub = temp.0.join("member_pkg");
        fs::create_dir_all(&sub).unwrap();
        fs::write(
            sub.join("Cargo.toml"),
            "[package]\nname = \"sub\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        let mut manifests = Vec::new();
        collect_all_workspace_manifests(&temp.0, &mut manifests);
        assert_eq!(manifests.len(), 2);

        // 2. compute_variant_id with non-standard manifest path
        let custom_manifest = temp.0.join("Custom.toml");
        fs::write(&custom_manifest, "[package]\nname = \"cust\"\n").unwrap();
        let var_id = compute_variant_id("dev", None, None, None, false, false, &custom_manifest);
        assert!(!var_id.is_empty());

        // 3. run_indexing with base_out_dir having a file named "variants" (triggers variant directory creation failure)
        let out_dir_with_variants_file = temp.0.join("out_dir_var_err");
        fs::create_dir_all(&out_dir_with_variants_file).unwrap();
        fs::write(
            out_dir_with_variants_file.join("variants"),
            b"blocking file",
        )
        .unwrap();
        let err_mkdir = try_run_indexing(IndexOptions {
            path: manifest_file.clone(),
            out_dir_opt: Some(out_dir_with_variants_file),
            package: None,
            release: false,
            features: None,
            no_default_features: false,
            all_features: false,
            target: None,
        });
        assert!(err_mkdir
            .unwrap_err()
            .contains("failed to create variant directory"));

        // 4. run_indexing when driver binary is not found
        let orig_driver = std::env::var("RIDX_DRIVER_BIN").ok();
        std::env::set_var("RIDX_DRIVER_BIN", "/nonexistent/ridx-driver-bogus");
        let err_no_driver = try_run_indexing(IndexOptions {
            path: manifest_file,
            out_dir_opt: None,
            package: None,
            release: false,
            features: None,
            no_default_features: false,
            all_features: false,
            target: None,
        });
        assert!(err_no_driver
            .unwrap_err()
            .contains("could not find ridx-driver binary"));
        if let Some(d) = orig_driver {
            std::env::set_var("RIDX_DRIVER_BIN", d);
        } else {
            std::env::remove_var("RIDX_DRIVER_BIN");
        }
    }
}
