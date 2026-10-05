//! Cargo indexing orchestration, variant hashing, driver invocation, and manifest publishing.

use colored::*;
use ridx_format::WorkspaceIndex;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

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
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join("ridx-driver");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    for candidate in [
        PathBuf::from("target/release/ridx-driver"),
        PathBuf::from("target/debug/ridx-driver"),
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

pub fn run_indexing(opts: IndexOptions) -> PublishedVariant {
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
            eprintln!(
                "{}: target path {:?} is a file but not a Cargo.toml manifest.",
                "Error".red().bold(),
                path
            );
            std::process::exit(1);
        }
        let canon = match path.canonicalize() {
            Ok(c) => c,
            Err(e) => {
                eprintln!(
                    "{}: failed to canonicalize manifest path {:?}: {e}",
                    "Error".red().bold(),
                    path
                );
                std::process::exit(1);
            }
        };
        let dir = canon
            .parent()
            .unwrap_or_else(|| Path::new("/"))
            .to_path_buf();
        (dir, canon)
    } else if path.is_dir() {
        let canon = match path.canonicalize() {
            Ok(c) => c,
            Err(e) => {
                eprintln!(
                    "{}: failed to canonicalize workspace path {:?}: {e}",
                    "Error".red().bold(),
                    path
                );
                std::process::exit(1);
            }
        };
        let manifest = canon.join("Cargo.toml");
        if !manifest.is_file() {
            eprintln!(
                "{}: no Cargo.toml found in directory {:?}.",
                "Error".red().bold(),
                path
            );
            std::process::exit(1);
        }
        (canon, manifest)
    } else {
        eprintln!("{}: path {:?} does not exist.", "Error".red().bold(), path);
        std::process::exit(1);
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
        eprintln!(
            "{}: failed to create output directory {:?}: {e}",
            "Error".red().bold(),
            base_out_dir
        );
        std::process::exit(1);
    }
    let base_out_dir = base_out_dir.canonicalize().unwrap_or(base_out_dir);

    let driver_bin = match find_driver_binary() {
        Some(b) => b,
        None => {
            eprintln!(
                "{}: could not find ridx-driver binary.",
                "Error".red().bold()
            );
            eprintln!("Please build ridx-driver first (e.g. cargo build -p ridx-driver) or place it in PATH.");
            std::process::exit(1);
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
        eprintln!(
            "{}: failed to create variant directory {:?}: {e}",
            "Error".red().bold(),
            variant_out_dir
        );
        std::process::exit(1);
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
            eprintln!(
                "{}: failed to execute cargo check: {e}",
                "Error".red().bold()
            );
            std::process::exit(1);
        }
    };

    let elapsed = start.elapsed();

    if !status.success() {
        eprintln!(
            "{}: cargo check exited with status {status}",
            "Error".red().bold()
        );
        std::process::exit(1);
    }

    let ws = match WorkspaceIndex::open(&variant_out_dir) {
        Ok(w) => w,
        Err(e) => {
            eprintln!(
                "{}: failed to open generated variant index {:?}: {e}",
                "Error".red().bold(),
                variant_out_dir
            );
            std::process::exit(1);
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

    let serialized = match serde_json::to_string_pretty(&manifest_data) {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "{}: failed to serialize variant manifest: {e}",
                "Error".red().bold()
            );
            std::process::exit(1);
        }
    };

    // Also save variant-specific manifest inside variant_out_dir
    let variant_manifest_file = variant_out_dir.join("variant_manifest.json");
    let _ = fs::write(&variant_manifest_file, &serialized);

    // Write to temp file then rename for atomic manifest swap
    let tmp_manifest = base_out_dir.join(format!("ridx_manifest.json.tmp.{}", std::process::id()));
    if let Err(e) = fs::write(&tmp_manifest, serialized) {
        eprintln!(
            "{}: failed to write temporary variant manifest {:?}: {e}",
            "Error".red().bold(),
            tmp_manifest
        );
        std::process::exit(1);
    }

    if let Err(e) = fs::rename(&tmp_manifest, &manifest_file) {
        let _ = fs::remove_file(&tmp_manifest);
        eprintln!(
            "{}: failed to publish active variant manifest {:?}: {e}",
            "Error".red().bold(),
            manifest_file
        );
        std::process::exit(1);
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

    PublishedVariant {
        base_out_dir,
        variant_out_dir,
        variant_id,
    }
}
