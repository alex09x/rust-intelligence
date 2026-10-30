/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Driver binary hashing, fingerprinting, and disk cache management.

use crate::format::{fnv1a_64, CURRENT_FORMAT_VERSION, EXTRACTOR_VERSION};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::time::SystemTime;

type DriverHashCache = HashMap<PathBuf, (u64, Option<SystemTime>, u64)>;

static DRIVER_HASH_CACHE: RwLock<Option<DriverHashCache>> = RwLock::new(None);

fn disk_cache_path(explicit_cache_dir: Option<&Path>) -> PathBuf {
    if let Some(cd) = explicit_cache_dir {
        cd.join("driver_hash.cache")
    } else if let Ok(cd) = std::env::var("RIDX_CACHE_DIR") {
        PathBuf::from(cd).join("driver_hash.cache")
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".cache").join("ridx").join("v3").join("driver_hash.cache")
    } else {
        std::env::temp_dir().join("ridx").join("v3").join("driver_hash.cache")
    }
}

fn file_sig(meta: &fs::Metadata) -> (u64, u64, u64) {
    let len = meta.len();
    let mtime_secs = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
    #[cfg(unix)]
    let ino = std::os::unix::fs::MetadataExt::ino(meta);
    #[cfg(not(unix))]
    let ino = 0u64;
    (len, mtime_secs, ino)
}

fn lookup_disk_cache(explicit_cache_dir: Option<&Path>, p_str: &str, len: u64, mtime_secs: u64, ino: u64) -> Option<u64> {
    let cp = disk_cache_path(explicit_cache_dir);
    let content = fs::read_to_string(&cp).ok()?;
    for line in content.lines() {
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() == 5
            && parts[0] == p_str
            && parts[1].parse::<u64>().ok() == Some(len)
            && parts[2].parse::<u64>().ok() == Some(mtime_secs)
            && parts[3].parse::<u64>().ok() == Some(ino)
            && parts[4].len() == 16
        {
            if let Ok(hash) = u64::from_str_radix(parts[4], 16) {
                return Some(hash);
            }
        }
    }
    None
}

fn append_disk_cache(explicit_cache_dir: Option<&Path>, p_str: &str, len: u64, mtime_secs: u64, ino: u64, hash: u64) {
    let cp = disk_cache_path(explicit_cache_dir);
    if let Some(parent) = cp.parent() {
        let _ = fs::create_dir_all(parent);
    }

    // Auto-compaction: if file exceeds 64 lines, deduplicate by path preserving latest entries
    if let Ok(content) = fs::read_to_string(&cp) {
        let lines: Vec<&str> = content.lines().collect();
        if lines.len() >= 64 {
            let mut latest_by_path: HashMap<&str, &str> = HashMap::new();
            for l in &lines {
                if let Some((p, _)) = l.split_once('\t') {
                    latest_by_path.insert(p, l);
                }
            }
            let tmp_path = cp.with_extension(format!("tmp.{}", std::process::id()));
            if let Ok(mut f) = fs::File::create(&tmp_path) {
                use std::io::Write;
                for entry_line in latest_by_path.values() {
                    let _ = writeln!(f, "{}", entry_line);
                }
                let _ = f.flush();
                let _ = fs::rename(&tmp_path, &cp);
            }
        }
    }

    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(&cp) {
        use std::io::Write;
        let _ = writeln!(f, "{}\t{}\t{}\t{}\t{:016x}", p_str, len, mtime_secs, ino, hash);
    }
}

fn hash_file_cached_with(p: &Path, explicit_cache_dir: Option<&Path>) -> Option<u64> {
    let meta = fs::metadata(p).ok()?;
    if !meta.is_file() {
        return None;
    }
    let len = meta.len();
    let mtime = meta.modified().ok();

    // 1. In-process cache check
    if let Ok(guard) = DRIVER_HASH_CACHE.read() {
        if let Some(map) = guard.as_ref() {
            if let Some(&(cached_len, cached_mtime, cached_hash)) = map.get(p) {
                if cached_len == len && cached_mtime == mtime {
                    return Some(cached_hash);
                }
            }
        }
    }

    // 2. Persistent disk cache check (persists across cargo RUSTC_WRAPPER invocations)
    let (file_len, mtime_secs, ino) = file_sig(&meta);
    let p_str = p.to_string_lossy();
    if let Some(disk_hash) = lookup_disk_cache(explicit_cache_dir, &p_str, file_len, mtime_secs, ino) {
        if let Ok(mut guard) = DRIVER_HASH_CACHE.write() {
            let map = guard.get_or_insert_with(HashMap::new);
            map.insert(p.to_path_buf(), (len, mtime, disk_hash));
        }
        return Some(disk_hash);
    }

    // 3. Cache miss: read bytes and compute fnv1a_64
    let bytes = fs::read(p).ok()?;
    let hash = fnv1a_64(&bytes);

    append_disk_cache(explicit_cache_dir, &p_str, file_len, mtime_secs, ino, hash);

    if let Ok(mut guard) = DRIVER_HASH_CACHE.write() {
        let map = guard.get_or_insert_with(HashMap::new);
        map.insert(p.to_path_buf(), (len, mtime, hash));
    }

    Some(hash)
}

/// Clears the driver binary hash cache (primarily used in tests).
#[doc(hidden)]
pub fn clear_driver_hash_cache() {
    clear_driver_hash_cache_in(None);
}

/// Clears the driver binary hash cache with optional isolated cache dir.
#[doc(hidden)]
pub fn clear_driver_hash_cache_in(cache_dir: Option<&Path>) {
    if let Ok(mut guard) = DRIVER_HASH_CACHE.write() {
        *guard = None;
    }
    if let Some(cd) = cache_dir {
        let _ = fs::remove_file(cd.join("driver_hash.cache"));
    } else if std::env::var("RIDX_CACHE_DIR").is_ok() {
        let _ = fs::remove_file(disk_cache_path(None));
    }
}

/// Returns the 64-bit fingerprint of the active `ridx-driver` binary and runtime environment.
///
/// Precedence:
/// 1. `$RIDX_DRIVER_HASH`: explicit hex or string override (for tests and deterministic builds).
/// 2. `$RIDX_DRIVER_BIN`: path to driver binary if provided; hashes binary contents (cached by path, size, mtime).
/// 3. Current executable binary: hashes `/proc/self/exe` or `current_exe()` (cached by path, size, mtime).
/// 4. Fallback: hash of `CARGO_PKG_VERSION` + format version.
pub fn get_driver_hash() -> u64 {
    get_driver_hash_with(None, None)
}

/// Computes or retrieves cached driver hash with optional explicit binary path and cache directory injection.
pub fn get_driver_hash_with(explicit_driver_bin: Option<&Path>, explicit_cache_dir: Option<&Path>) -> u64 {
    if let Ok(override_val) = std::env::var("RIDX_DRIVER_HASH") {
        let trimmed = override_val.trim();
        if let Ok(val) = u64::from_str_radix(trimmed.trim_start_matches("0x"), 16) {
            return val;
        }
        return fnv1a_64(trimmed.as_bytes());
    }
    if let Some(p) = explicit_driver_bin {
        if let Some(hash) = hash_file_cached_with(p, explicit_cache_dir) {
            return hash;
        }
    } else if let Ok(driver_bin) = std::env::var("RIDX_DRIVER_BIN") {
        let p = Path::new(driver_bin.trim());
        if let Some(hash) = hash_file_cached_with(p, explicit_cache_dir) {
            return hash;
        }
    }
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(hash) = hash_file_cached_with(&exe_path, explicit_cache_dir) {
            return hash;
        }
    }
    let fallback = format!("{}:{}", CURRENT_FORMAT_VERSION, EXTRACTOR_VERSION);
    fnv1a_64(fallback.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lookup_disk_cache_rejects_truncated_hex() {
        let temp_dir = tempfile::tempdir().unwrap();
        let cache_file = temp_dir.path().join("driver_hash.cache");
        // Write line with only 12 hex characters instead of 16
        fs::write(&cache_file, "/test/bin\t100\t200\t300\t123456789abc\n").unwrap();

        let res = lookup_disk_cache(Some(temp_dir.path()), "/test/bin", 100, 200, 300);
        assert_eq!(res, None, "Truncated hash must be rejected");

        // Write line with exact 16 hex characters
        fs::write(&cache_file, "/test/bin\t100\t200\t300\t0123456789abcdef\n").unwrap();
        let res = lookup_disk_cache(Some(temp_dir.path()), "/test/bin", 100, 200, 300);
        assert_eq!(res, Some(0x0123456789abcdef));
    }

    #[test]
    fn test_append_disk_cache_compaction_preserves_latest() {
        let temp_dir = tempfile::tempdir().unwrap();
        for i in 0..70 {
            append_disk_cache(Some(temp_dir.path()), "/test/bin", 100, 200 + i, 300, 0x0123456789abcdef + i);
        }

        let res = lookup_disk_cache(Some(temp_dir.path()), "/test/bin", 100, 269, 300);
        assert_eq!(res, Some(0x0123456789abcdef + 69));
    }
}
