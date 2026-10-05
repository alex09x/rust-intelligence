use clap::{Parser, Subcommand};
use colored::*;
use ridx_format::WorkspaceIndex;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

mod compare;
mod lsp;

#[derive(Parser)]
#[command(name = "ridx")]
#[command(about = "Rust Intelligence: Ultra-fast, zero-memory code intelligence CLI", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Index a Cargo crate or workspace using ridx-driver and cargo check
    Index {
        /// Path to Cargo workspace or crate directory (default: current directory)
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Output directory for .ridx index files (default: target/ridx)
        #[arg(long, short)]
        out_dir: Option<PathBuf>,

        /// Optional package filter (e.g. -p my_crate)
        #[arg(long, short)]
        package: Option<String>,

        /// Run cargo check in release mode
        #[arg(long)]
        release: bool,

        /// Space or comma separated list of features to activate
        #[arg(long)]
        features: Option<String>,

        /// Do not activate the `default` feature
        #[arg(long)]
        no_default_features: bool,

        /// Activate all available features
        #[arg(long)]
        all_features: bool,

        /// Build for the target triple
        #[arg(long)]
        target: Option<String>,
    },

    /// Inspect binary header, record counts, and memory layout of a .ridx index file or workspace
    Inspect {
        /// Path to .ridx index file or directory (default: auto-discover target/ridx)
        index_path: Option<PathBuf>,

        /// Output result as JSON
        #[arg(long)]
        json: bool,
    },

    /// Go to definition for a symbol path or cursor location (file:line:col)
    Def {
        /// First argument: index file/dir, or target symbol/location if index is omitted
        arg1: String,
        /// Second argument: target symbol/location if first argument was index path
        arg2: Option<String>,

        /// Output result as JSON
        #[arg(long)]
        json: bool,
    },

    /// Find all references to a given symbol path or cursor location (file:line:col)
    Refs {
        /// First argument: index file/dir, or target symbol/location if index is omitted
        arg1: String,
        /// Second argument: target symbol/location if first argument was index path
        arg2: Option<String>,

        /// Output result as JSON
        #[arg(long)]
        json: bool,
    },

    /// Show inferred expression type at a cursor location (file:line:col)
    TypeAt {
        /// First argument: index file/dir, or location if index is omitted
        arg1: String,
        /// Second argument: location if first argument was index path
        arg2: Option<String>,

        /// Output result as JSON
        #[arg(long)]
        json: bool,
    },

    /// Show combined hover information (signature + inferred type) at cursor location
    Hover {
        /// First argument: index file/dir, or location if index is omitted
        arg1: String,
        /// Second argument: location if first argument was index path
        arg2: Option<String>,

        /// Output result as JSON
        #[arg(long)]
        json: bool,
    },

    /// Search symbol definitions matching a query string across the workspace
    Symbols {
        /// First argument: index file/dir, or query if index is omitted
        arg1: String,
        /// Second argument: query if first argument was index path
        arg2: Option<String>,

        /// Output result as JSON
        #[arg(long)]
        json: bool,
    },

    /// Compare accuracy and latency against prod-code (rust-analyzer)
    Compare {
        /// First argument: index file/dir, or target symbol/location
        arg1: String,
        /// Second argument: target symbol/location if first argument was index path
        arg2: Option<String>,
        /// Optional explicit symbol flag
        #[arg(long, short)]
        symbol: Option<String>,

        /// Output result as JSON
        #[arg(long)]
        json: bool,
    },

    /// Run as a Language Server Protocol (LSP) server over stdio
    Lsp {
        /// Path to .ridx index file or workspace directory (default: auto-discover)
        #[arg(long, short)]
        index_path: Option<PathBuf>,
    },
}

fn parse_location(loc: &str) -> Option<(&str, u32, u32)> {
    let parts: Vec<&str> = loc.rsplitn(3, ':').collect();
    if parts.len() == 3 {
        let col: u32 = parts[0].parse().ok()?;
        let line: u32 = parts[1].parse().ok()?;
        let file = parts[2];
        Some((file, line, col))
    } else {
        None
    }
}

fn resolve_workspace(path_opt: Option<PathBuf>) -> WorkspaceIndex {
    match path_opt {
        Some(p) => match WorkspaceIndex::open(&p) {
            Ok(ws) => ws,
            Err(e) => {
                eprintln!("{}: Failed to open index {:?}: {e}", "Error".red().bold(), p);
                std::process::exit(1);
            }
        },
        None => match WorkspaceIndex::auto_discover() {
            Ok(ws) => ws,
            Err(e) => {
                eprintln!("{}: {e}", "Error".red().bold());
                eprintln!("Specify the path to a .ridx file or directory, or run from a workspace with target/ridx/.");
                std::process::exit(1);
            }
        },
    }
}

fn resolve_index_and_target(arg1: String, arg2: Option<String>) -> (WorkspaceIndex, String) {
    match arg2 {
        Some(target) => (resolve_workspace(Some(PathBuf::from(arg1))), target),
        None => (resolve_workspace(None), arg1),
    }
}

fn find_driver_binary() -> Option<PathBuf> {
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


fn find_cargo_configs(work_dir: &Path) -> Vec<PathBuf> {
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
    if let Ok(cargo_home) = std::env::var("CARGO_HOME") {
        let home_dir = Path::new(&cargo_home);
        for name in ["config.toml", "config"] {
            let p = home_dir.join(name);
            if p.is_file() && !configs.contains(&p) {
                configs.push(p);
            }
        }
    }
    configs
}

fn extract_config_target(content: &str) -> Option<String> {
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

fn collect_all_workspace_manifests(dir: &Path, manifests: &mut Vec<PathBuf>) {
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

fn compute_variant_id(
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
        m.strip_prefix(work_dir).unwrap_or(&m).display().to_string().hash(&mut hasher);
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

    let hash = hasher.finish();
    format!("{profile}-{:016x}", hash)
}

struct IndexOptions {
    path: PathBuf,
    out_dir_opt: Option<PathBuf>,
    package: Option<String>,
    release: bool,
    features: Option<String>,
    no_default_features: bool,
    all_features: bool,
    target: Option<String>,
}

fn run_indexing(opts: IndexOptions) {
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
            eprintln!("{}: target path {:?} is a file but not a Cargo.toml manifest.", "Error".red().bold(), path);
            std::process::exit(1);
        }
        let canon = match path.canonicalize() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("{}: failed to canonicalize manifest path {:?}: {e}", "Error".red().bold(), path);
                std::process::exit(1);
            }
        };
        let dir = canon.parent().unwrap_or_else(|| Path::new("/")).to_path_buf();
        (dir, canon)
    } else if path.is_dir() {
        let canon = match path.canonicalize() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("{}: failed to canonicalize workspace path {:?}: {e}", "Error".red().bold(), path);
                std::process::exit(1);
            }
        };
        let manifest = canon.join("Cargo.toml");
        if !manifest.is_file() {
            eprintln!("{}: no Cargo.toml found in directory {:?}.", "Error".red().bold(), path);
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
                std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")).join(d)
            }
        }
        None => work_dir.join("target/ridx"),
    };
    if let Err(e) = fs::create_dir_all(&base_out_dir) {
        eprintln!("{}: failed to create output directory {:?}: {e}", "Error".red().bold(), base_out_dir);
        std::process::exit(1);
    }
    let base_out_dir = base_out_dir.canonicalize().unwrap_or(base_out_dir);

    let driver_bin = match find_driver_binary() {
        Some(b) => b,
        None => {
            eprintln!("{}: could not find ridx-driver binary.", "Error".red().bold());
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
        eprintln!("{}: failed to create variant directory {:?}: {e}", "Error".red().bold(), variant_out_dir);
        std::process::exit(1);
    }
    let variant_out_dir = variant_out_dir.canonicalize().unwrap_or(variant_out_dir);

    // Isolate Cargo target cache per variant so Cargo unconditionally executes wrapper on variant switch
    let cargo_target_dir = work_dir.join("target/ridx_cargo").join(&variant_id);
    let _ = fs::create_dir_all(&cargo_target_dir);

    println!("{}", "═══════════════════════════════════════════════════════════════════════".dimmed());
    println!("  {} {}", "RIDX Workspace Indexer:".bold(), work_dir.display().to_string().cyan());
    println!("{}", "═══════════════════════════════════════════════════════════════════════".dimmed());
    println!("  Work Directory : {}", work_dir.display().to_string().green());
    println!("  Manifest Path  : {}", manifest_path.display().to_string().green());
    println!("  Driver Binary  : {}", driver_bin.display().to_string().green());
    println!("  Index Output   : {}", variant_out_dir.display().to_string().green());
    println!("  Active Variant : {}", variant_id.cyan().bold());
    println!("  Build Profile  : {}", profile_name.cyan());
    if let Some(ref feats) = features {
        println!("  Features       : {}", feats.cyan());
    }
    if no_default_features {
        println!("  No Default Feat: true");
    }
    if all_features {
        println!("  All Features   : true");
    }
    if let Some(ref tgt) = target {
        println!("  Target Triple  : {}", tgt.cyan());
    }
    println!("  Action         : cargo check with RUSTC_WORKSPACE_WRAPPER...");

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

    cmd.env("RUSTC_WORKSPACE_WRAPPER", &driver_bin);
    cmd.env("RIDX_OUT_DIR", &variant_out_dir);
    cmd.env("CARGO_TARGET_DIR", &cargo_target_dir);
    cmd.env("RUSTC_BOOTSTRAP", "1");
    cmd.env("RUSTC_WRAPPER", "");

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
            eprintln!("{}: failed to execute cargo check: {e}", "Error".red().bold());
            std::process::exit(1);
        }
    };

    let elapsed = start.elapsed();

    if !status.success() {
        eprintln!("{}: cargo check exited with status {status}", "Error".red().bold());
        std::process::exit(1);
    }

    let ws = match WorkspaceIndex::open(&variant_out_dir) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("{}: failed to open generated variant index {:?}: {e}", "Error".red().bold(), variant_out_dir);
            std::process::exit(1);
        }
    };

    let stats = ws.stats();

    // Record build variant fingerprint manifest atomically in base_out_dir
    let manifest_file = base_out_dir.join("ridx_manifest.json");
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
        "active_variant_id": variant_id,
        "variant_dir": variant_rel_dir,
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
        "crates": stats.crate_summaries.iter().map(|c| serde_json::json!({
            "name": c.name,
            "version": c.version,
            "file": c.path.file_name().and_then(|s| s.to_str()).unwrap_or(""),
            "file_size": c.file_size,
        })).collect::<Vec<_>>()
    });

    let serialized = match serde_json::to_string_pretty(&manifest_data) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{}: failed to serialize variant manifest: {e}", "Error".red().bold());
            std::process::exit(1);
        }
    };

    // Write to temp file then rename for atomic manifest swap
    let tmp_manifest = base_out_dir.join(format!("ridx_manifest.json.tmp.{}", std::process::id()));
    if let Err(e) = fs::write(&tmp_manifest, serialized) {
        eprintln!("{}: failed to write temporary variant manifest {:?}: {e}", "Error".red().bold(), tmp_manifest);
        std::process::exit(1);
    }

    if let Err(e) = fs::rename(&tmp_manifest, &manifest_file) {
        let _ = fs::remove_file(&tmp_manifest);
        eprintln!("{}: failed to publish active variant manifest {:?}: {e}", "Error".red().bold(), manifest_file);
        std::process::exit(1);
    }

    println!(
        "\n{} Indexed workspace in {:.2} s (active variant: {})",
        "Success:".green().bold(),
        elapsed.as_secs_f64(),
        variant_id.cyan().bold()
    );

    println!(
        "Indexed {} crate(s), {} total definitions, {} references, {} types ({:.2} KB total index size)",
        stats.total_crates.to_string().bold(),
        stats.total_defs.to_string().green(),
        stats.total_refs.to_string().green(),
        stats.total_types.to_string().green(),
        stats.total_bytes as f64 / 1024.0
    );
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Index {
            path,
            out_dir,
            package,
            release,
            features,
            no_default_features,
            all_features,
            target,
        } => {
            run_indexing(IndexOptions {
                path,
                out_dir_opt: out_dir,
                package,
                release,
                features,
                no_default_features,
                all_features,
                target,
            });
        }

        Commands::Inspect { index_path, json } => {
            let ws = resolve_workspace(index_path);
            let stats = ws.stats();

            if json {
                let out = serde_json::json!({
                    "total_crates": stats.total_crates,
                    "total_bytes": stats.total_bytes,
                    "total_files": stats.total_files,
                    "total_defs": stats.total_defs,
                    "total_refs": stats.total_refs,
                    "total_types": stats.total_types,
                    "active_variant": stats.active_variant,
                    "crates": stats.crate_summaries.iter().map(|c| serde_json::json!({
                        "name": c.name,
                        "version": c.version,
                        "file_size": c.file_size,
                        "defs_count": c.defs_count,
                        "refs_count": c.refs_count,
                        "types_count": c.types_count,
                        "files_count": c.files_count,
                        "path": c.path.display().to_string(),
                    })).collect::<Vec<_>>()
                });
                println!("{}", serde_json::to_string_pretty(&out).unwrap());
                return;
            }

            println!("{}", "═══════════════════════════════════════════════════════════════════════".dimmed());
            println!("  {} {} crate(s) loaded", "RIDX Workspace Index:".bold(), stats.total_crates.to_string().cyan());
            if let Some(ref v) = stats.active_variant {
                println!("  Active Variant        : {}", v.cyan().bold());
            }
            println!("{}", "═══════════════════════════════════════════════════════════════════════".dimmed());
            println!("{:<28}: {}", "Total Crates", stats.total_crates.to_string().bold());
            println!("{:<28}: {} bytes ({:.2} KB / {:.2} MB)", "Total Index Size", stats.total_bytes, stats.total_bytes as f64 / 1024.0, stats.total_bytes as f64 / (1024.0 * 1024.0));
            println!("{:<28}: {}", "Indexed Files", stats.total_files);
            println!("{:<28}: {}", "Definitions Count", stats.total_defs.to_string().green().bold());
            println!("{:<28}: {}", "References Count", stats.total_refs.to_string().green().bold());
            println!("{:<28}: {}", "Inferred Types Count", stats.total_types.to_string().green().bold());
            println!("{:<28}: {}", "Zero-Copy Lookup Ready", "YES (Mmap)".green().bold());
            println!("{:<28}: {}", "Daemon Memory Required", "0 MB (pure disk index)".green().bold());
            println!("{}", "───────────────────────────────────────────────────────────────────────".dimmed());
            println!("  Crate Breakdown:");
            for c in &stats.crate_summaries {
                println!(
                    "  • {:<24} v{:<8} ({:>4} defs, {:>5} refs, {:>5} types, {:>6.1} KB) -> {}",
                    c.name.cyan(),
                    c.version,
                    c.defs_count,
                    c.refs_count,
                    c.types_count,
                    c.file_size as f64 / 1024.0,
                    c.path.display()
                );
            }
            println!("{}", "═══════════════════════════════════════════════════════════════════════".dimmed());
        }

        Commands::Def { arg1, arg2, json } => {
            let (ws, target) = resolve_index_and_target(arg1, arg2);
            let start = Instant::now();

            let res = if let Some((file, line, col)) = parse_location(&target) {
                ws.find_definition_at(file, line, col)
            } else {
                ws.find_definition_by_path(&target)
            };
            let elapsed = start.elapsed();

            if json {
                let out = match res {
                    Some(def) => serde_json::json!({
                        "found": true,
                        "name": def.name,
                        "path": def.path,
                        "kind": format!("{:?}", def.kind),
                        "signature": def.signature,
                        "file": def.file,
                        "start_line": def.start_line,
                        "start_col": def.start_col,
                        "end_line": def.end_line,
                        "end_col": def.end_col,
                        "latency_us": elapsed.as_micros(),
                    }),
                    None => serde_json::json!({
                        "found": false,
                        "target": target,
                        "latency_us": elapsed.as_micros(),
                    }),
                };
                println!("{}", serde_json::to_string_pretty(&out).unwrap());
                return;
            }

            match res {
                Some(def) => {
                    println!(
                        "{} {} at {}:{}:{} (kind: {:?}) [{:.1} µs]",
                        "Definition:".green().bold(),
                        def.name.bold(),
                        def.file,
                        def.start_line,
                        def.start_col,
                        def.kind,
                        elapsed.as_micros()
                    );
                    println!("  Full Path: {}", def.path.cyan());
                    if let Some(sig) = def.signature {
                        println!("  Signature: {}", sig.yellow());
                    }
                }
                None => {
                    println!(
                        "{} No definition found for {} [{:.1} µs]",
                        "NotFound:".yellow().bold(),
                        target,
                        elapsed.as_micros()
                    );
                }
            }
        }

        Commands::Refs { arg1, arg2, json } => {
            let (ws, target) = resolve_index_and_target(arg1, arg2);
            let start = Instant::now();

            if let Some((file, line, col)) = parse_location(&target) {
                let loc_refs = ws.find_references_at(file, line, col);
                let elapsed = start.elapsed();

                if json {
                    let out = match loc_refs {
                        Some(lr) => serde_json::json!({
                            "target_symbol": lr.target_symbol,
                            "target_kind": format!("{:?}", lr.target_kind),
                            "target_path": lr.target_path,
                            "target_file": lr.target_file,
                            "target_line": lr.target_line,
                            "target_col": lr.target_col,
                            "references_count": lr.references.len(),
                            "latency_us": elapsed.as_micros(),
                            "references": lr.references.iter().map(|r| serde_json::json!({
                                "crate": r.crate_name,
                                "file": r.file,
                                "start_line": r.span.start_line(),
                                "start_col": r.span.start_col(),
                                "end_line": r.span.end_line(),
                                "end_col": r.span.end_col(),
                                "kind": format!("{:?}", r.kind),
                                "is_write": r.is_write,
                            })).collect::<Vec<_>>()
                        }),
                        None => serde_json::json!({
                            "target_symbol": null,
                            "references_count": 0,
                            "latency_us": elapsed.as_micros(),
                            "references": [],
                        }),
                    };
                    println!("{}", serde_json::to_string_pretty(&out).unwrap());
                    return;
                }

                match loc_refs {
                    Some(lr) => {
                        println!(
                            "{} Symbol {} ({:?}) at {}:{}:{} — {} reference(s) [{:.1} µs]:",
                            "Found:".green().bold(),
                            lr.target_symbol.cyan().bold(),
                            lr.target_kind,
                            lr.target_file,
                            lr.target_line,
                            lr.target_col,
                            lr.references.len(),
                            elapsed.as_micros()
                        );

                        for (i, r) in lr.references.iter().enumerate() {
                            let span = r.span;
                            println!(
                                "  [{}] ({}) {}:{}:{}-{}:{} (kind: {:?})",
                                i + 1,
                                r.crate_name.cyan(),
                                r.file,
                                span.start_line(),
                                span.start_col(),
                                span.end_line(),
                                span.end_col(),
                                r.kind
                            );
                        }
                    }
                    None => {
                        println!(
                            "{} No references found at {} [{:.1} µs]",
                            "NotFound:".yellow().bold(),
                            target,
                            elapsed.as_micros()
                        );
                    }
                }
            } else {
                let refs = ws.find_references(&target);
                let elapsed = start.elapsed();

                if json {
                    let out = serde_json::json!({
                        "target_symbol": target,
                        "references_count": refs.len(),
                        "latency_us": elapsed.as_micros(),
                        "references": refs.iter().map(|r| serde_json::json!({
                            "crate": r.crate_name,
                            "file": r.file,
                            "start_line": r.span.start_line(),
                            "start_col": r.span.start_col(),
                            "end_line": r.span.end_line(),
                            "end_col": r.span.end_col(),
                            "kind": format!("{:?}", r.kind),
                            "is_write": r.is_write,
                        })).collect::<Vec<_>>()
                    });
                    println!("{}", serde_json::to_string_pretty(&out).unwrap());
                    return;
                }

                println!(
                    "{} {} reference(s) to {} across workspace [{:.1} µs]:",
                    "Found:".green().bold(),
                    refs.len(),
                    target.cyan(),
                    elapsed.as_micros()
                );

                for (i, r) in refs.iter().enumerate() {
                    let span = r.span;
                    println!(
                        "  [{}] ({}) {}:{}:{}-{}:{} (kind: {:?})",
                        i + 1,
                        r.crate_name.cyan(),
                        r.file,
                        span.start_line(),
                        span.start_col(),
                        span.end_line(),
                        span.end_col(),
                        r.kind
                    );
                }
            }
        }

        Commands::TypeAt { arg1, arg2, json } => {
            let (ws, location) = resolve_index_and_target(arg1, arg2);
            let Some((file, line, col)) = parse_location(&location) else {
                eprintln!(
                    "{}: Location must be formatted as 'file_path:line:col'",
                    "Error".red().bold()
                );
                std::process::exit(1);
            };

            let start = Instant::now();
            let ty = ws.type_at(file, line, col);
            let elapsed = start.elapsed();

            if json {
                let out = serde_json::json!({
                    "location": location,
                    "inferred_type": ty,
                    "latency_us": elapsed.as_micros(),
                });
                println!("{}", serde_json::to_string_pretty(&out).unwrap());
                return;
            }

            match ty {
                Some(t) => {
                    println!(
                        "{} Inferred type at {}:{}:{} [{:.1} µs]",
                        "Type:".green().bold(),
                        file,
                        line,
                        col,
                        elapsed.as_micros()
                    );
                    println!("  {}", t.cyan().bold());
                }
                None => {
                    println!(
                        "{} No type information found at {}:{}:{} [{:.1} µs]",
                        "NotFound:".yellow().bold(),
                        file,
                        line,
                        col,
                        elapsed.as_micros()
                    );
                }
            }
        }

        Commands::Hover { arg1, arg2, json } => {
            let (ws, location) = resolve_index_and_target(arg1, arg2);
            let Some((file, line, col)) = parse_location(&location) else {
                eprintln!(
                    "{}: Location must be formatted as 'file_path:line:col'",
                    "Error".red().bold()
                );
                std::process::exit(1);
            };

            let start = Instant::now();
            let hover = ws.hover_at(file, line, col);
            let elapsed = start.elapsed();

            if json {
                let out = match hover {
                    Some(h) => serde_json::json!({
                        "found": true,
                        "location": location,
                        "definition": h.definition.map(|d| serde_json::json!({
                            "name": d.name,
                            "path": d.path,
                            "kind": format!("{:?}", d.kind),
                            "signature": d.signature,
                            "file": d.file,
                            "start_line": d.start_line,
                            "start_col": d.start_col,
                            "end_line": d.end_line,
                            "end_col": d.end_col,
                        })),
                        "inferred_type": h.inferred_type,
                        "latency_us": elapsed.as_micros(),
                    }),
                    None => serde_json::json!({
                        "found": false,
                        "location": location,
                        "latency_us": elapsed.as_micros(),
                    }),
                };
                println!("{}", serde_json::to_string_pretty(&out).unwrap());
                return;
            }

            match hover {
                Some(h) => {
                    println!(
                        "{} Hover at {} [{:.1} µs]",
                        "Hover:".green().bold(),
                        location.cyan(),
                        elapsed.as_micros()
                    );
                    if let Some(ref d) = h.definition {
                        println!(
                            "  Definition : {} ({:?}) at {}:{}:{}",
                            d.name.bold(),
                            d.kind,
                            d.file,
                            d.start_line,
                            d.start_col
                        );
                        println!("  Full Path  : {}", d.path.cyan());
                        if let Some(sig) = d.signature {
                            println!("  Signature  : {}", sig.yellow());
                        }
                    }
                    if let Some(ty) = h.inferred_type {
                        println!("  Inferred Ty: {}", ty.bold());
                    }
                }
                None => {
                    println!(
                        "{} No hover information at {} [{:.1} µs]",
                        "NotFound:".yellow().bold(),
                        location,
                        elapsed.as_micros()
                    );
                }
            }
        }

        Commands::Symbols { arg1, arg2, json } => {
            let (ws, query) = resolve_index_and_target(arg1, arg2);
            let start = Instant::now();
            let matches = ws.search_symbols(&query);
            let elapsed = start.elapsed();

            if json {
                let out = serde_json::json!({
                    "query": query,
                    "count": matches.len(),
                    "latency_us": elapsed.as_micros(),
                    "symbols": matches.iter().map(|s| serde_json::json!({
                        "crate": s.crate_name,
                        "name": s.name,
                        "path": s.path,
                        "kind": format!("{:?}", s.kind),
                        "file": s.file,
                        "start_line": s.span.start_line(),
                        "start_col": s.span.start_col(),
                        "signature": s.signature,
                    })).collect::<Vec<_>>()
                });
                println!("{}", serde_json::to_string_pretty(&out).unwrap());
                return;
            }

            println!(
                "{} {} symbol(s) matching '{}' across workspace [{:.1} µs]:",
                "Found:".green().bold(),
                matches.len(),
                query.cyan(),
                elapsed.as_micros()
            );

            for sym in matches.iter().take(50) {
                let span = sym.span;
                println!(
                    "  [{}] [{:?}] {} — {}:{}:{}",
                    sym.crate_name.dimmed(),
                    sym.kind,
                    sym.path.cyan(),
                    sym.file,
                    span.start_line(),
                    span.start_col()
                );
                if let Some(sig) = sym.signature {
                    println!("       sig: {}", sig.yellow());
                }
            }
        }

        Commands::Compare {
            arg1,
            arg2,
            symbol,
            json,
        } => {
            let (ws, target) = match symbol {
                Some(s) => {
                    let ws = resolve_workspace(Some(PathBuf::from(arg1)));
                    (ws, s)
                }
                None => resolve_index_and_target(arg1, arg2),
            };

            let cmp = compare::compare_definition(&ws, &target);
            if json {
                let speedup = if cmp.ridx_latency_micros > 0 {
                    format!(
                        "{:.1}x",
                        (cmp.prodcode_latency_micros as f64) / (cmp.ridx_latency_micros as f64)
                    )
                } else {
                    "inf".to_string()
                };
                let out = serde_json::json!({
                    "target": cmp.symbol,
                    "ridx": {
                        "found": cmp.ridx_found,
                        "span": cmp.ridx_span,
                        "latency_us": cmp.ridx_latency_micros,
                    },
                    "prodcode": {
                        "found": cmp.prodcode_found,
                        "span": cmp.prodcode_span,
                        "latency_us": cmp.prodcode_latency_micros,
                    },
                    "speedup": speedup,
                });
                println!("{}", serde_json::to_string_pretty(&out).unwrap());
                return;
            }
            compare::print_comparison(&cmp);
        }

        Commands::Lsp { index_path } => {
            if let Err(e) = lsp::run_lsp_server(index_path) {
                eprintln!("{}: LSP server exited with error: {e}", "Error".red().bold());
                std::process::exit(1);
            }
        }
    }
}
