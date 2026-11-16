/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! The source text each workspace file had when it was indexed, kept by its hash under `target/ridx/sources`.
//!
//! The server maps a buffer onto the index through the text the index was compiled from: a line of the live buffer
//! to the compiled line. It knows that text when the buffer or the file on disk still hashes as the index records;
//! a file changed on disk before the server opened it (a checkout, a pull, a save made while no server ran) hashes
//! otherwise, and with no compiled text to map through, the whole file lost the index until the next reindex. The
//! snapshot is that text.

use ridx_format::{fnv1a_64, normalized_source_hash};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Where the snapshots of an index directory live.
fn snapshot_dir(base_out_dir: &Path) -> PathBuf {
    base_out_dir.join("sources")
}

fn snapshot_name(hash: u64) -> String {
    format!("{hash:016x}.rs")
}

/// Records the text of every workspace source file that is on disk as the variant compiled it, and removes the
/// snapshots the variant no longer names.
pub(crate) fn write_source_snapshots(base_out_dir: &Path, variant_out_dir: &Path, work_dir: &Path) {
    let dir = snapshot_dir(base_out_dir);
    if fs::create_dir_all(&dir).is_err() {
        return;
    }
    let mut kept = HashSet::new();
    let mut seen_files = HashSet::new();
    let mut sidecars: Vec<PathBuf> = fs::read_dir(variant_out_dir.join(".deps")).into_iter().flatten().flatten().map(|e| e.path()).collect();
    sidecars.sort();
    for sidecar in sidecars {
        let Some(side) = fs::read(&sidecar).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()) else { continue };
        if side["is_dependency"].as_bool() != Some(false) {
            continue;
        }
        let Some(hashes) = side["file_hashes"].as_object() else { continue };
        for (file, recorded) in hashes {
            let path = Path::new(file);
            // A file is done once one sidecar's hash matched it: an older sidecar left in `.deps` for the same file
            // records a hash from before the edit.
            if !path.is_absolute() || !path.starts_with(work_dir) || seen_files.contains(file) {
                continue;
            }
            let Ok(bytes) = fs::read(path) else { continue };
            let recorded = recorded.as_u64();
            let norm = normalized_source_hash(&bytes);
            if recorded != Some(fnv1a_64(&bytes)) && recorded != Some(norm) {
                continue; // changed since it was compiled: the index holds an older text
            }
            let name = snapshot_name(norm);
            let target = dir.join(&name);
            if !target.is_file() {
                let tmp = dir.join(format!("{name}.tmp.{}", std::process::id()));
                if fs::write(&tmp, &bytes).is_err() || fs::rename(&tmp, &target).is_err() {
                    let _ = fs::remove_file(&tmp);
                    continue;
                }
            }
            kept.insert(name);
            seen_files.insert(file.clone());
        }
    }
    for entry in fs::read_dir(&dir).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !kept.contains(&name) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// The text `file` had when it was compiled into the index that records its hash as `hash`, from the snapshots of
/// the nearest index directory above it.
pub fn compiled_source_snapshot(file: &str, hash: u64) -> Option<String> {
    let name = snapshot_name(hash);
    Path::new(file).ancestors().skip(1).map(|d| snapshot_dir(&d.join("target/ridx")).join(&name)).find(|p| p.is_file()).and_then(|p| fs::read_to_string(p).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_snapshots_keep_the_compiled_text_of_workspace_files_only() {
        let root = std::env::temp_dir().join(format!("ridx_snapshots_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let (base, variant) = (root.join("target/ridx"), root.join("target/ridx/variants/dev-x"));
        fs::create_dir_all(variant.join(".deps")).unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        let (lib, edited) = (root.join("src/lib.rs"), root.join("src/edited.rs"));
        fs::write(&lib, "pub fn a() {}\n").unwrap();
        fs::write(&edited, "pub fn b() {}\n").unwrap();
        let compiled_b = normalized_source_hash(b"pub fn b() {}\n");
        let side = serde_json::json!({ "is_dependency": false, "file_hashes": {
            lib.display().to_string(): fnv1a_64(b"pub fn a() {}\n"), "src/lib.rs": fnv1a_64(b"pub fn a() {}\n"),
            edited.display().to_string(): compiled_b,
        }});
        fs::write(variant.join(".deps/app.json"), side.to_string()).unwrap();
        fs::write(variant.join(".deps/dep.json"), serde_json::json!({ "is_dependency": true, "file_hashes": { "/elsewhere/x.rs": 1 } }).to_string()).unwrap();
        // An older sidecar of the same crate, read first, records `edited.rs` as it was before.
        let older = serde_json::json!({ "is_dependency": false, "file_hashes": { edited.display().to_string(): 42 } });
        fs::write(variant.join(".deps/aaa_app_older.json"), older.to_string()).unwrap();
        fs::create_dir_all(snapshot_dir(&base)).unwrap();
        fs::write(snapshot_dir(&base).join("0000000000000001.rs"), "old").unwrap();
        write_source_snapshots(&base, &variant, &root);

        // The file is edited on disk after the index was built: the snapshot still gives the text it was compiled from.
        fs::write(&edited, "pub fn b() {}\n// edited on disk\n").unwrap();
        assert_eq!(compiled_source_snapshot(&edited.display().to_string(), compiled_b).as_deref(), Some("pub fn b() {}\n"));
        let a = normalized_source_hash(b"pub fn a() {}\n");
        assert_eq!(compiled_source_snapshot(&lib.display().to_string(), a).as_deref(), Some("pub fn a() {}\n"));
        assert!(compiled_source_snapshot(&lib.display().to_string(), 7).is_none(), "no snapshot of another text");
        let mut names: Vec<String> = fs::read_dir(snapshot_dir(&base)).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        let mut expected = vec![snapshot_name(a), snapshot_name(compiled_b)];
        expected.sort();
        assert_eq!(names, expected, "one snapshot per compiled text, none left over");

        // The next run, from the edited file, drops the snapshot of the text no index names any more.
        let side = serde_json::json!({ "is_dependency": false, "file_hashes": { edited.display().to_string(): fnv1a_64(b"pub fn b() {}\n// edited on disk\n") } });
        fs::write(variant.join(".deps/app.json"), side.to_string()).unwrap();
        write_source_snapshots(&base, &variant, &root);
        assert!(compiled_source_snapshot(&edited.display().to_string(), compiled_b).is_none());
        let _ = fs::remove_dir_all(&root);
    }
}
