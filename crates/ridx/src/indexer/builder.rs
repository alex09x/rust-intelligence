/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Index builder selection and check command construction.

use super::types::IndexOptions;
use std::path::Path;

/// Build backend used to execute compiler check passes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum, serde::Serialize, serde::Deserialize)]
#[value(rename_all = "lowercase")]
pub enum IndexBuilder {
    /// Standard Cargo tool (`cargo check`).
    #[default]
    Cargo,
    /// Mitts distributed build tool (`mitts check`).
    Mitts,
}

impl IndexBuilder {
    /// Environment variable used to select index builder backend when unspecified on CLI.
    pub const ENV_VAR: &'static str = "RIDX_INDEX_BUILDER";

    /// Returns the canonical binary/program name for this builder.
    #[inline]
    pub fn as_str(&self) -> &'static str {
        match self {
            IndexBuilder::Cargo => "cargo",
            IndexBuilder::Mitts => "mitts",
        }
    }

    /// Resolves the index builder from an explicit CLI flag,
    /// falling back to the `RIDX_INDEX_BUILDER` environment variable,
    /// and falling back to Cargo.
    pub fn resolve(cli_flag: Option<IndexBuilder>) -> Self {
        Self::resolve_with_env(cli_flag, std::env::var(Self::ENV_VAR).ok().as_deref())
    }

    /// Pure resolution logic taking an optional CLI flag and an optional environment variable string.
    pub fn resolve_with_env(cli_flag: Option<IndexBuilder>, env_val: Option<&str>) -> Self {
        if let Some(builder) = cli_flag {
            return builder;
        }
        if let Some(env) = env_val {
            let trimmed = env.trim();
            if !trimmed.is_empty() {
                if let Ok(builder) = trimmed.parse::<IndexBuilder>() {
                    return builder;
                }
            }
        }
        IndexBuilder::Cargo
    }

    /// Pure resolution logic taking string slices for both CLI flag and environment variable.
    pub fn resolve_from_str(cli_flag: Option<&str>, env_val: Option<&str>) -> Self {
        if let Some(flag) = cli_flag {
            let trimmed = flag.trim();
            if !trimmed.is_empty() {
                if let Ok(builder) = trimmed.parse::<IndexBuilder>() {
                    return builder;
                }
            }
        }
        Self::resolve_with_env(None, env_val)
    }
}

impl std::fmt::Display for IndexBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for IndexBuilder {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "cargo" => Ok(IndexBuilder::Cargo),
            "mitts" => Ok(IndexBuilder::Mitts),
            other => Err(format!("unknown index builder '{other}', expected 'cargo' or 'mitts'")),
        }
    }
}

/// The check arguments for a builder release that does not know `--keep-going` yet.
pub fn without_keep_going(args: &[String]) -> Vec<String> {
    args.iter().filter(|a| *a != "--keep-going").cloned().collect()
}

/// The arguments of the check pass narrowed to library and binary targets.
///
/// A builder without `--keep-going` stops at the first unit that fails, so one broken test harness leaves
/// every crate downstream of it unindexed. Tests, examples and benches are leaves of the unit graph: a pass
/// over libraries and binaries alone builds everything those leaves could have blocked.
pub fn lib_and_bin_fallback_args(args: &[String]) -> Vec<String> {
    let mut out = Vec::with_capacity(args.len() + 1);
    for a in args {
        if a == "--keep-going" {
            continue;
        }
        if a == "--all-targets" {
            out.push("--lib".to_string());
            out.push("--bins".to_string());
        } else {
            out.push(a.clone());
        }
    }
    out
}

/// Pure function to build the program, argument vector, and environment variables
/// for the index check command.
#[allow(clippy::too_many_arguments)]
pub fn build_check_command(
    opts: &IndexOptions,
    builder: IndexBuilder,
    manifest_path: &Path,
    work_dir: &Path,
    driver_bin: &Path,
    variant_out_dir: &Path,
    cache_dir: &Path,
    cargo_target_dir: &Path,
    target_kinds_file: &Path,
    ld_path: Option<&str>,
) -> (String, Vec<String>, Vec<(String, String)>) {
    let no_deps = std::env::var("RIDX_NO_DEPS").is_ok();
    build_check_command_with_no_deps(opts, builder, manifest_path, work_dir, driver_bin, variant_out_dir, cache_dir, cargo_target_dir, target_kinds_file, ld_path, no_deps)
}

/// Pure function to build the program, argument vector, and environment variables
/// with an explicit `no_deps` flag.
#[allow(clippy::too_many_arguments)]
pub fn build_check_command_with_no_deps(
    opts: &IndexOptions,
    builder: IndexBuilder,
    manifest_path: &Path,
    work_dir: &Path,
    driver_bin: &Path,
    variant_out_dir: &Path,
    cache_dir: &Path,
    cargo_target_dir: &Path,
    target_kinds_file: &Path,
    ld_path: Option<&str>,
    no_deps: bool,
) -> (String, Vec<String>, Vec<(String, String)>) {
    let program = builder.as_str().to_string();

    let mut args = vec!["check".to_string(), "--manifest-path".to_string(), manifest_path.display().to_string(), "--all-targets".to_string()];
    // One crate that fails to build (a test harness needing another toolchain) must not leave the rest unindexed.
    args.push("--keep-going".to_string());
    if let Some(ref p) = opts.package {
        args.push("-p".to_string());
        args.push(p.clone());
    } else {
        args.push("--workspace".to_string());
    }
    if opts.release {
        args.push("--release".to_string());
    }
    if let Some(ref feats) = opts.features {
        args.push("--features".to_string());
        args.push(feats.clone());
    }
    if opts.no_default_features {
        args.push("--no-default-features".to_string());
    }
    if opts.all_features {
        args.push("--all-features".to_string());
    }
    if let Some(ref tgt) = opts.target {
        args.push("--target".to_string());
        args.push(tgt.clone());
    }

    let mut envs = vec![
        ("RIDX_OUT_DIR".to_string(), variant_out_dir.display().to_string()),
        ("RIDX_CACHE_DIR".to_string(), cache_dir.display().to_string()),
        ("RIDX_WORKSPACE_ROOT".to_string(), work_dir.display().to_string()),
        ("CARGO_TARGET_DIR".to_string(), cargo_target_dir.display().to_string()),
        ("RUSTC_BOOTSTRAP".to_string(), "1".to_string()),
        ("RIDX_TARGET_KINDS_FILE".to_string(), target_kinds_file.display().to_string()),
    ];

    if !no_deps {
        envs.push(("RUSTC_WRAPPER".to_string(), driver_bin.display().to_string()));
        envs.push(("RUSTC_WORKSPACE_WRAPPER".to_string(), String::new()));
    } else {
        envs.push(("RUSTC_WORKSPACE_WRAPPER".to_string(), driver_bin.display().to_string()));
        envs.push(("RUSTC_WRAPPER".to_string(), String::new()));
    }

    if let Some(ld) = ld_path {
        envs.push(("LD_LIBRARY_PATH".to_string(), ld.to_string()));
        envs.push(("DYLD_FALLBACK_LIBRARY_PATH".to_string(), ld.to_string()));
    }
    // The driver's own toolchain, over a workspace pin to another rustc (whose std it cannot load); an explicit
    // RUSTUP_TOOLCHAIN still wins.
    if std::env::var_os("RUSTUP_TOOLCHAIN").is_none() {
        if let Some(tc) = super::cargo::driver_toolchain() {
            envs.push(("RUSTUP_TOOLCHAIN".to_string(), tc));
        }
    }

    (program, args, envs)
}
