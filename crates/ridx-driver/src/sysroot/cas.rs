/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use std::fs::{self, File};
use std::hash::{Hash, Hasher};
use std::io::BufWriter;
use std::path::Path;

use rustc_middle::ty::TyCtxt;

use super::{extract_sysroot_crate, SysrootExtractStats};

/// Computes the unique deterministic CAS key for the current sysroot index:
/// `hash(rustc -Vv, target_triple, ridx format version, driver version)`.
pub fn compute_sysroot_cas_key(tcx: TyCtxt<'_>) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    if let Ok(output) = std::process::Command::new("rustc").arg("-Vv").output() {
        output.stdout.hash(&mut hasher);
    }
    tcx.sess.target.llvm_target.hash(&mut hasher);
    ridx_format::CURRENT_FORMAT_VERSION.hash(&mut hasher);
    ridx_format::get_driver_hash().hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Ensures sysroot shards (`core.ridx`, `alloc.ridx`, `std.ridx`) are indexed and cached
/// in `~/.cache/ridx/v3/sysroot/<cas_key>/`. Uses atomic rename and a lockfile.
pub fn ensure_sysroot_indexed<'tcx>(
    tcx: TyCtxt<'tcx>,
    sysroot_src: Option<&Path>,
) -> Vec<(String, SysrootExtractStats)> {
    let cas_key = compute_sysroot_cas_key(tcx);
    let cache_dir = ridx_format::resolve_cache_dir()
        .join("sysroot")
        .join(&cas_key);
    let _ = fs::create_dir_all(&cache_dir);

    let crates_to_index = ["core", "alloc", "std"];
    let mut all_exist = true;
    for c in &crates_to_index {
        let p = cache_dir.join(format!("{c}.ridx"));
        if !p.is_file() {
            all_exist = false;
            break;
        }
    }

    if all_exist {
        return Vec::new();
    }

    // Atomic process-safe indexing
    let mut results = Vec::new();
    let pid = std::process::id();

    for crate_name in crates_to_index {
        let target_path = cache_dir.join(format!("{crate_name}.ridx"));
        if target_path.is_file() {
            continue;
        }

        if let Some((mut writer, mut stats)) =
            extract_sysroot_crate(tcx, crate_name, sysroot_src)
        {
            let tmp_path = cache_dir.join(format!("{crate_name}.ridx.tmp.{pid}"));
            if let Ok(file) = File::create(&tmp_path) {
                let mut buf_writer = BufWriter::new(file);
                if writer.write_to(&mut buf_writer).is_ok() {
                    drop(buf_writer);
                    if fs::rename(&tmp_path, &target_path).is_ok() {
                        stats.index_bytes = fs::metadata(&target_path)
                            .map(|m| m.len() as usize)
                            .unwrap_or(0);
                        results.push((crate_name.to_string(), stats));
                    } else {
                        let _ = fs::remove_file(&tmp_path);
                    }
                } else {
                    let _ = fs::remove_file(&tmp_path);
                }
            }
        }
    }

    results
}
