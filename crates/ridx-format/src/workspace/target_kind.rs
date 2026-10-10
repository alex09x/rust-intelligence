/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Target kinds and authoritative target classification for crate compilation units.

use serde::{Deserialize, Serialize};
use std::path::Path;

/// Target compilation unit classification within a crate or workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TargetKind {
    /// Library crate (rlib, dylib, cdylib, proc-macro).
    Lib,
    /// Executable binary.
    Bin,
    /// Integration or unit test harness.
    Test,
    /// Benchmark harness.
    Bench,
    /// Example binary or library.
    Example,
    /// Custom build script (build.rs).
    CustomBuild,
    /// Other target type.
    Other,
}

impl TargetKind {
    /// Returns the lowercase canonical identifier string for the target kind.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Lib => "lib",
            Self::Bin => "bin",
            Self::Test => "test",
            Self::Bench => "bench",
            Self::Example => "example",
            Self::CustomBuild => "custom-build",
            Self::Other => "other",
        }
    }

    /// Returns true if this target represents the primary library compilation unit.
    pub fn is_lib(&self) -> bool {
        matches!(self, Self::Lib)
    }

    /// Parses a string into a TargetKind, matching case-insensitively.
    pub fn parse(s: &str) -> Option<Self> {
        let trimmed = s.trim().to_ascii_lowercase();
        match trimmed.as_str() {
            "lib" | "rlib" | "dylib" | "cdylib" | "staticlib" | "proc-macro" => Some(Self::Lib),
            "bin" => Some(Self::Bin),
            "test" => Some(Self::Test),
            "bench" => Some(Self::Bench),
            "example" => Some(Self::Example),
            "custom-build" => Some(Self::CustomBuild),
            _ => None,
        }
    }
}

impl std::fmt::Display for TargetKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Authoritatively classifies a compilation unit based on its `cargo metadata` target kind,
/// the active rustc compiler arguments, and manifest directory context.
///
/// Strictly eliminates path substring heuristics (`/tests/`, `/examples/`, `/benches/`):
/// 1. If `base_kind` from `cargo metadata` is provided:
///    - `lib` or `bin` compiled with `--test` becomes `Test` (unit test harness).
///    - `lib` without `--test` remains `Lib` (even if in `examples/foo/src/lib.rs` or under `/tmp/x/tests/`).
///    - `bin` remains `Bin` (even if path is `examples/tool.rs`).
///    - `bench` remains `Bench` (even if compiled with `--crate-type bin` and `harness = false`).
///    - `test` remains `Test`.
///    - `example` remains `Example`.
///    - `custom-build` remains `CustomBuild`.
/// 2. If `base_kind` is absent (fallback for ad-hoc invocations):
///    - `--test` -> `Test`.
///    - If entrypoint relative to `manifest_dir` begins with `benches/`, `examples/`, `tests/`, or equals `build.rs` -> corresponding kind.
///    - `--crate-type bin` -> `Bin`.
///    - Default -> `Lib`.
pub fn classify_target_with_manifest(
    base_kind: Option<&str>,
    compiler_args: &[String],
    manifest_dir: Option<&Path>,
) -> TargetKind {
    let has_test_flag = compiler_args.iter().any(|a| a == "--test");

    if let Some(kind) = base_kind {
        let trimmed = kind.trim().to_ascii_lowercase();
        match trimmed.as_str() {
            "lib" | "rlib" | "dylib" | "cdylib" | "staticlib" | "proc-macro" => {
                if has_test_flag {
                    return TargetKind::Test;
                } else {
                    return TargetKind::Lib;
                }
            }
            "bin" => {
                if has_test_flag {
                    return TargetKind::Test;
                } else {
                    return TargetKind::Bin;
                }
            }
            "test" => return TargetKind::Test,
            "bench" => return TargetKind::Bench,
            "example" => return TargetKind::Example,
            "custom-build" => return TargetKind::CustomBuild,
            _ => {
                if has_test_flag {
                    return TargetKind::Test;
                } else {
                    return TargetKind::Lib;
                }
            }
        }
    }

    if has_test_flag {
        return TargetKind::Test;
    }

    if let Some(inferred) = infer_target_from_manifest(compiler_args, manifest_dir) {
        return inferred;
    }

    if compiler_args
        .windows(2)
        .any(|w| w[0] == "--crate-type" && w[1] == "bin")
        || compiler_args
            .iter()
            .any(|a| a.starts_with("--crate-type=bin"))
    {
        TargetKind::Bin
    } else {
        TargetKind::Lib
    }
}

/// Backwards-compatible target classifier without manifest directory context.
pub fn classify_target(base_kind: Option<&str>, compiler_args: &[String]) -> TargetKind {
    classify_target_with_manifest(base_kind, compiler_args, None)
}

fn infer_target_from_manifest(
    compiler_args: &[String],
    manifest_dir: Option<&Path>,
) -> Option<TargetKind> {
    let manifest = manifest_dir?;
    let entry_rs = compiler_args
        .iter()
        .find(|a| !a.starts_with('-') && a.ends_with(".rs"))?;

    let p = Path::new(entry_rs);
    let rel = if p.is_relative() {
        p.to_path_buf()
    } else {
        p.strip_prefix(manifest)
            .ok()
            .map(|p| p.to_path_buf())
            .or_else(|| {
                if let Ok(canon_p) = std::fs::canonicalize(p) {
                    if let Ok(canon_m) = std::fs::canonicalize(manifest) {
                        return canon_p.strip_prefix(canon_m).ok().map(|p| p.to_path_buf());
                    }
                }
                None
            })?
    };

    if rel.starts_with("benches") {
        Some(TargetKind::Bench)
    } else if rel.starts_with("examples") {
        Some(TargetKind::Example)
    } else if rel.starts_with("tests") {
        Some(TargetKind::Test)
    } else if rel == Path::new("build.rs") {
        Some(TargetKind::CustomBuild)
    } else {
        None
    }
}
