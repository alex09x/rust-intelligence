/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Extraction of active crate `cfg` predicates, enabled Cargo features, target triple,
//! and `Cargo.lock` provenance hash for `.ridx` shards and per-crate unit keys.

use ridx_format::IndexWriter;
use rustc_middle::ty::TyCtxt;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

/// Resolves the canonical compilation target tuple from `tcx.sess`.
pub(crate) fn effective_target_triple(tcx: TyCtxt<'_>) -> String {
    let raw = tcx.sess.opts.target_triple.tuple();
    if raw.is_empty() {
        rustc_session::config::host_tuple().to_string()
    } else {
        raw.to_string()
    }
}

/// Parses a single `--cfg` specification (`key` or `key="value"` with `\"` / `\\` escapes).
pub(crate) fn parse_cfg_spec(raw: &str) -> Option<(String, Option<String>)> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    let Some((lhs, rhs)) = s.split_once('=') else {
        return Some((s.to_string(), None));
    };
    let key = lhs.trim();
    if key.is_empty() {
        return None;
    }
    let val_raw = rhs.trim();
    let unquoted = if val_raw.len() >= 2 && val_raw.starts_with('"') && val_raw.ends_with('"') {
        let inner = &val_raw[1..val_raw.len() - 1];
        if inner.contains('\\') {
            let mut out = String::with_capacity(inner.len());
            let mut chars = inner.chars();
            while let Some(c) = chars.next() {
                if c == '\\' {
                    if let Some(n) = chars.next() {
                        out.push(n);
                    }
                } else {
                    out.push(c);
                }
            }
            out
        } else {
            inner.to_string()
        }
    } else {
        val_raw.to_string()
    };
    Some((key.to_string(), Some(unquoted)))
}

/// Populates `writer`'s target triple and `CfgEntry` table from `tcx.sess` built-in cfgs,
/// `Callbacks::config` `crate_cfg`, and `--cfg` CLI flags.
///
/// Returns `(target_triple, sorted_enabled_features)`.
pub(crate) fn populate_crate_cfgs(tcx: TyCtxt<'_>, crate_cfg: &[String], compiler_args: &[String], writer: &mut IndexWriter, include_user_cfgs: bool) -> (String, Vec<String>) {
    let target = effective_target_triple(tcx);
    writer.set_target_triple(&target);

    let mut enabled_features = BTreeSet::new();
    let built = rustc_session::config::build_configuration(tcx.sess, Default::default());
    for (k, v) in &built {
        let ks = k.as_str();
        if !include_user_cfgs && matches!(ks, "feature" | "test") {
            continue;
        }
        let vs = v.as_ref().map(|sym| sym.as_str());
        if ks == "feature" {
            if let Some(feat) = vs {
                enabled_features.insert(feat.to_string());
            }
        }
        writer.add_cfg(ks, vs);
    }

    if include_user_cfgs {
        for spec in crate_cfg {
            if let Some((k, v)) = parse_cfg_spec(spec) {
                if k == "feature" {
                    if let Some(ref feat) = v {
                        enabled_features.insert(feat.clone());
                    }
                }
                writer.add_cfg(&k, v.as_deref());
            }
        }
        let mut i = 0;
        while i < compiler_args.len() {
            let raw_opt = if compiler_args[i] == "--cfg" && i + 1 < compiler_args.len() {
                i += 1;
                Some(compiler_args[i].as_str())
            } else {
                compiler_args[i].strip_prefix("--cfg=")
            };
            if let Some(raw) = raw_opt {
                if let Some((k, v)) = parse_cfg_spec(raw) {
                    if k == "feature" {
                        if let Some(ref feat) = v {
                            enabled_features.insert(feat.clone());
                        }
                    }
                    writer.add_cfg(&k, v.as_deref());
                }
            }
            i += 1;
        }
    }

    (target, enabled_features.into_iter().collect())
}

/// Resolves the workspace `Cargo.lock` 16-hex FNV-1a hash from `RIDX_CARGO_LOCK_HASH`,
/// `RIDX_WORKSPACE_ROOT/Cargo.lock`, or ancestor directories of `manifest_dir`.
pub(crate) fn resolve_workspace_cargo_lock_hash(manifest_dir: Option<&Path>) -> Option<String> {
    if let Ok(env_hash) = std::env::var("RIDX_CARGO_LOCK_HASH") {
        let trimmed = env_hash.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    if let Ok(ws_root) = std::env::var("RIDX_WORKSPACE_ROOT") {
        let lock_path = Path::new(&ws_root).join("Cargo.lock");
        if let Ok(bytes) = fs::read(&lock_path) {
            return Some(format!("{:016x}", ridx_format::fnv1a_64(&bytes)));
        }
    }
    let mut cur = manifest_dir;
    for _ in 0..5 {
        let dir = cur?;
        let lock_path = dir.join("Cargo.lock");
        if lock_path.is_file() {
            if let Ok(bytes) = fs::read(&lock_path) {
                return Some(format!("{:016x}", ridx_format::fnv1a_64(&bytes)));
            }
        }
        cur = dir.parent();
    }
    None
}
