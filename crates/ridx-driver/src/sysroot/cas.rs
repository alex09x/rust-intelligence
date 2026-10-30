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
use std::io::BufWriter;
use std::path::Path;

use rustc_middle::ty::TyCtxt;

use super::{extract_sysroot_crate, SysrootExtractStats};

/// Computes the unique deterministic CAS key for a target triple and optional rustc version string:
/// `fnv1a_64(rustc version, target triple, format version, driver hash)`.
pub fn compute_sysroot_cas_key_str(version_str: Option<&str>, target_triple: &str) -> String {
    let mut bytes = Vec::new();
    if let Some(v) = version_str.or(rustc_interface::util::rustc_version_str()) {
        bytes.extend_from_slice(v.as_bytes());
    }
    bytes.extend_from_slice(target_triple.as_bytes());
    bytes.extend_from_slice(&ridx_format::CURRENT_FORMAT_VERSION.to_le_bytes());
    bytes.extend_from_slice(&ridx_format::get_driver_hash().to_le_bytes());
    let hash = ridx_format::fnv1a_64(&bytes);
    format!("{:016x}", hash)
}

/// Computes the unique deterministic CAS key for the current sysroot index:
/// `fnv1a_64(rustc version, target triple, format version, driver hash)`.
pub fn compute_sysroot_cas_key(tcx: TyCtxt<'_>) -> String {
    let ver = rustc_interface::util::rustc_version_str().unwrap_or(tcx.sess.cfg_version.as_ref());
    compute_sysroot_cas_key_str(Some(ver), &tcx.sess.opts.target_triple.to_string())
}

/// Ensures sysroot shards (`core.ridx`, `alloc.ridx`, `std.ridx`) are indexed and cached
/// in `~/.cache/ridx/v3/sysroot/<cas_key>/`. Uses atomic rename into CAS cache directory.
pub fn ensure_sysroot_indexed<'tcx>(tcx: TyCtxt<'tcx>, sysroot_src: Option<&Path>) -> Vec<(String, SysrootExtractStats)> {
    let cas_key = compute_sysroot_cas_key(tcx);
    let cache_dir = ridx_format::resolve_cache_dir().join("sysroot").join(&cas_key);
    let _ = fs::create_dir_all(&cache_dir);

    // Clean up orphaned temporary files from crashed processes
    if let Ok(entries) = fs::read_dir(&cache_dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                if name.contains(".ridx.tmp.") {
                    let _ = fs::remove_file(&p);
                }
            }
        }
    }

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

        if let Some((mut writer, mut stats)) = extract_sysroot_crate(tcx, crate_name, sysroot_src) {
            let tmp_path = cache_dir.join(format!("{crate_name}.ridx.tmp.{pid}"));
            if let Ok(file) = File::create(&tmp_path) {
                let mut buf_writer = BufWriter::new(file);
                if writer.write_to(&mut buf_writer).is_ok() {
                    drop(buf_writer);
                    if fs::rename(&tmp_path, &target_path).is_ok() {
                        stats.index_bytes = fs::metadata(&target_path).map(|m| m.len() as usize).unwrap_or(0);
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
