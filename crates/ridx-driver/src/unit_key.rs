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

pub(crate) fn extract_target_disambiguator(args: &[String]) -> Option<String> {
    for i in 0..args.len() {
        let arg = &args[i];
        if let Some(rest) = arg.strip_prefix("-Cextra-filename=") {
            if !rest.is_empty() {
                return Some(rest.to_string());
            }
        }
        if arg == "-C" && i + 1 < args.len() {
            if let Some(rest) = args[i + 1].strip_prefix("extra-filename=") {
                if !rest.is_empty() {
                    return Some(rest.to_string());
                }
            }
        }
    }
    for i in 0..args.len() {
        let arg = &args[i];
        if let Some(rest) = arg.strip_prefix("--crate-type=") {
            if !rest.is_empty() {
                return Some(format!("_{rest}"));
            }
        }
        if arg == "--crate-type" && i + 1 < args.len() {
            return Some(format!("_{}", args[i + 1]));
        }
    }
    None
}

pub(crate) fn hash_directory_contents(dir: &Path) -> Option<u64> {
    if !dir.is_dir() {
        return None;
    }
    let mut entries = Vec::new();
    fn visit(d: &Path, base: &Path, list: &mut Vec<(String, u64)>) {
        if let Ok(rd) = fs::read_dir(d) {
            for entry in rd.flatten() {
                let p = entry.path();
                if p.is_file() {
                    let rel = p.strip_prefix(base).unwrap_or(&p).to_string_lossy().to_string();
                    let hash = fs::read(&p).map(|b| ridx_format::fnv1a_64(&b)).unwrap_or(0);
                    list.push((rel, hash));
                } else if p.is_dir() {
                    visit(&p, base, list);
                }
            }
        }
    }
    visit(dir, dir, &mut entries);
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let mut hasher_input = Vec::new();
    for (rel, h) in entries {
        hasher_input.extend_from_slice(rel.as_bytes());
        hasher_input.extend_from_slice(&h.to_le_bytes());
    }
    Some(ridx_format::fnv1a_64(&hasher_input))
}

pub(crate) fn hash_file_contents(path: &Path) -> Option<u64> {
    let bytes = fs::read(path).ok()?;
    Some(ridx_format::fnv1a_64(&bytes))
}

pub(crate) fn compute_dependency_unit_key(crate_name: &str, crate_version: &str, disambiguator: Option<&str>, manifest_dir: Option<&Path>, compiler_args: &[String]) -> String {
    let mut key_str = format!(
        "{}:{}:{}:v{}:ext{}:drv={:016x}",
        crate_name,
        crate_version,
        disambiguator.unwrap_or(""),
        ridx_format::CURRENT_FORMAT_VERSION,
        ridx_format::EXTRACTOR_VERSION,
        ridx_format::get_driver_hash(),
    );
    if let Some(m) = manifest_dir {
        key_str.push(':');
        key_str.push_str(&m.display().to_string());
    }
    // Effective target: from args or env
    let mut target = std::env::var("TARGET").ok();
    for i in 0..compiler_args.len() {
        if compiler_args[i] == "--target" && i + 1 < compiler_args.len() {
            target = Some(compiler_args[i + 1].clone());
            break;
        } else if let Some(rest) = compiler_args[i].strip_prefix("--target=") {
            target = Some(rest.to_string());
            break;
        }
    }
    if let Some(ref t) = target {
        key_str.push(':');
        key_str.push_str(t);
    }

    // Extract and sort all `--cfg` flags
    let mut cfgs = Vec::new();
    for i in 0..compiler_args.len() {
        if compiler_args[i] == "--cfg" && i + 1 < compiler_args.len() {
            cfgs.push(compiler_args[i + 1].clone());
        } else if let Some(rest) = compiler_args[i].strip_prefix("--cfg=") {
            cfgs.push(rest.to_string());
        }
    }
    cfgs.sort();
    for cfg in cfgs {
        key_str.push_str(":cfg=");
        key_str.push_str(&cfg);
    }

    // Extract and sort all `--extern` flags, hashing dependency artifact content instead of physical target paths
    let mut externs = Vec::new();
    for i in 0..compiler_args.len() {
        if compiler_args[i] == "--extern" && i + 1 < compiler_args.len() {
            externs.push(compiler_args[i + 1].clone());
        } else if let Some(rest) = compiler_args[i].strip_prefix("--extern=") {
            externs.push(rest.to_string());
        }
    }
    externs.sort();
    for ext in externs {
        if let Some((name_part, path_str)) = ext.split_once('=') {
            let p = Path::new(path_str);
            if p.is_file() {
                let digest = hash_file_contents(p).unwrap_or(0);
                key_str.push_str(&format!(":ext={name_part}:digest={:016x}", digest));
            } else {
                key_str.push_str(&format!(":ext={name_part}:missing"));
            }
        } else {
            key_str.push_str(&format!(":ext={ext}"));
        }
    }

    // Hash OUT_DIR contents if present
    if let Ok(out_dir_str) = std::env::var("OUT_DIR") {
        let out_dir_path = Path::new(&out_dir_str);
        if let Some(out_hash) = hash_directory_contents(out_dir_path) {
            key_str.push_str(&format!(":outdir={:016x}", out_hash));
        }
    }

    format!("{:016x}", ridx_format::fnv1a_64(key_str.as_bytes()))
}
