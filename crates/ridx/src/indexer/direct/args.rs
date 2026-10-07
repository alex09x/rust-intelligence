/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Compiler argument sanitization and output directory isolation for direct indexing.

use std::collections::HashSet;
use std::path::Path;

/// Rewrites compiler arguments to prevent mutating Cargo's build directory:
/// - Replaces `--out-dir` with `direct_deps_dir`
/// - Replaces `-o` with `direct_deps_dir/<filename>`
/// - Replaces `-C incremental=...` with `direct_incr_dir`
/// - Replaces `--emit=...` with `--emit=metadata`
/// - Prepends `-L dependency=<direct_deps_dir>` so rustc discovers newly emitted workspace .rmeta
/// - Rewrites `--extern <crate>=<path>` for crates in `recompiled_crates` to point to `<direct_deps_dir>/<file_name>`
pub fn sanitize_compiler_args_for_direct(
    args: &[String],
    direct_deps_dir: &Path,
    direct_incr_dir: &Path,
    recompiled_crates: &HashSet<String>,
) -> Vec<String> {
    let mut new_args = Vec::with_capacity(args.len() + 4);
    let mut emit_replaced = false;
    let mut out_dir_replaced = false;

    // Prepend -L dependency=<direct_deps_dir> so rustc finds newly indexed workspace rmeta
    new_args.push("-L".to_string());
    new_args.push(format!("dependency={}", direct_deps_dir.display()));

    let mut i = 1; // skip args[0] (which is the driver or rustc binary)
    while i < args.len() {
        let arg = &args[i];

        if arg == "--out-dir" {
            new_args.push("--out-dir".to_string());
            new_args.push(direct_deps_dir.display().to_string());
            out_dir_replaced = true;
            i += 2;
            continue;
        } else if arg.starts_with("--out-dir=") {
            new_args.push(format!("--out-dir={}", direct_deps_dir.display()));
            out_dir_replaced = true;
            i += 1;
            continue;
        }

        if arg == "-o" && i + 1 < args.len() {
            let file_name = Path::new(&args[i + 1])
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("output");
            new_args.push("-o".to_string());
            new_args.push(direct_deps_dir.join(file_name).display().to_string());
            i += 2;
            continue;
        }

        if arg == "-C" && i + 1 < args.len() && args[i + 1].starts_with("incremental=") {
            new_args.push("-C".to_string());
            new_args.push(format!("incremental={}", direct_incr_dir.display()));
            i += 2;
            continue;
        } else if arg.starts_with("-Cincremental=") {
            new_args.push(format!("-Cincremental={}", direct_incr_dir.display()));
            i += 1;
            continue;
        }

        if arg == "--emit" && i + 1 < args.len() {
            new_args.push("--emit=metadata".to_string());
            emit_replaced = true;
            i += 2;
            continue;
        } else if arg.starts_with("--emit=") {
            new_args.push("--emit=metadata".to_string());
            emit_replaced = true;
            i += 1;
            continue;
        }

        if arg == "--extern" && i + 1 < args.len() {
            let next_arg = &args[i + 1];
            let rewritten = rewrite_extern_arg(next_arg, direct_deps_dir, recompiled_crates);
            new_args.push("--extern".to_string());
            new_args.push(rewritten);
            i += 2;
            continue;
        } else if let Some(rest) = arg.strip_prefix("--extern=") {
            let rewritten = rewrite_extern_arg(rest, direct_deps_dir, recompiled_crates);
            new_args.push(format!("--extern={rewritten}"));
            i += 1;
            continue;
        }

        new_args.push(arg.clone());
        i += 1;
    }

    if !out_dir_replaced {
        new_args.push("--out-dir".to_string());
        new_args.push(direct_deps_dir.display().to_string());
    }

    if !emit_replaced {
        new_args.push("--emit=metadata".to_string());
    }

    new_args
}

fn rewrite_extern_arg(
    extern_spec: &str,
    direct_deps_dir: &Path,
    recompiled_crates: &HashSet<String>,
) -> String {
    if let Some((prefix, cpath)) = extern_spec.split_once('=') {
        let cname = prefix.rsplit(':').next().unwrap_or(prefix);
        let cname_norm = cname.replace('-', "_");
        let p = Path::new(cpath);
        if let Some(file_name_os) = p.file_name() {
            let file_name = file_name_os.to_string_lossy();
            let direct_candidate = direct_deps_dir.join(&*file_name);

            let is_recompiled = recompiled_crates.contains(cname)
                || recompiled_crates.contains(prefix)
                || recompiled_crates
                    .iter()
                    .any(|r| r.replace('-', "_") == cname_norm);

            let should_rewrite = is_recompiled || direct_candidate.is_file();

            if should_rewrite {
                let target_path = if direct_candidate.is_file() {
                    direct_candidate
                } else if let Some(base) = file_name.strip_suffix(".rlib") {
                    direct_deps_dir.join(format!("{base}.rmeta"))
                } else {
                    direct_candidate
                };
                return format!("{prefix}={}", target_path.display());
            }
        }
    }
    extern_spec.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_compiler_args_isolates_output_and_incremental() {
        let original = vec![
            "rustc".to_string(),
            "lib.rs".to_string(),
            "--out-dir".to_string(),
            "/target/debug/deps".to_string(),
            "-C".to_string(),
            "incremental=/target/debug/incremental".to_string(),
            "--emit=dep-info,link".to_string(),
        ];
        let deps = Path::new("/target/ridx-direct/deps");
        let incr = Path::new("/target/ridx-direct/incremental");
        let recompiled = HashSet::new();

        let sanitized = sanitize_compiler_args_for_direct(&original, deps, incr, &recompiled);

        assert_eq!(sanitized[0], "-L");
        assert_eq!(sanitized[1], "dependency=/target/ridx-direct/deps");
        assert!(sanitized.contains(&"--out-dir".to_string()));
        assert!(sanitized.contains(&"/target/ridx-direct/deps".to_string()));
        assert!(!sanitized.contains(&"/target/debug/deps".to_string()));
        assert!(sanitized.contains(&"incremental=/target/ridx-direct/incremental".to_string()));
        assert!(!sanitized.contains(&"incremental=/target/debug/incremental".to_string()));
        assert!(sanitized.contains(&"--emit=metadata".to_string()));
        assert!(!sanitized.contains(&"--emit=dep-info,link".to_string()));
    }

    #[test]
    fn test_sanitize_compiler_args_rewrites_recompiled_externs() {
        let original = vec![
            "rustc".to_string(),
            "src/main.rs".to_string(),
            "--extern".to_string(),
            "serde=/old/target/debug/deps/libserde-123.rmeta".to_string(),
            "--extern=syn=/old/target/debug/deps/libsyn-456.rmeta".to_string(),
            "--extern".to_string(),
            "noprelude:serde_core=/old/target/debug/deps/libserde_core-789.rmeta".to_string(),
        ];
        let deps = Path::new("/target/ridx-direct/deps");
        let incr = Path::new("/target/ridx-direct/incremental");
        let mut recompiled = HashSet::new();
        recompiled.insert("serde".to_string());
        recompiled.insert("serde_core".to_string());

        let sanitized = sanitize_compiler_args_for_direct(&original, deps, incr, &recompiled);

        assert!(
            sanitized.contains(&"serde=/target/ridx-direct/deps/libserde-123.rmeta".to_string())
        );
        assert!(
            sanitized.contains(&"--extern=syn=/old/target/debug/deps/libsyn-456.rmeta".to_string())
        );
        assert!(sanitized.contains(
            &"noprelude:serde_core=/target/ridx-direct/deps/libserde_core-789.rmeta".to_string()
        ));
    }

    #[test]
    fn test_sanitize_compiler_args_rewrites_rlib_to_rmeta_and_handles_dashes() {
        let original = vec![
            "rustc".to_string(),
            "src/main.rs".to_string(),
            "--extern".to_string(),
            "my_crate=/old/target/debug/deps/libmy_crate-abc.rlib".to_string(),
        ];
        let deps = Path::new("/target/ridx-direct/deps");
        let incr = Path::new("/target/ridx-direct/incremental");
        let mut recompiled = HashSet::new();
        recompiled.insert("my-crate".to_string());

        let sanitized = sanitize_compiler_args_for_direct(&original, deps, incr, &recompiled);

        assert!(sanitized
            .contains(&"my_crate=/target/ridx-direct/deps/libmy_crate-abc.rmeta".to_string()));
    }
}
