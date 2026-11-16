/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! What a variant keeps after a check, by unit: a package's lib that rebuilt must not take its failed bin's shard
//! along (both share the crate name), a dependency's sidecar set aside by the invalidation comes back when it did not
//! rebuild, and a superseded shard leaves with its sidecar.

use crate::indexer::last_good::settle_after_check;
use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

fn sidecar(at: &Path, name: &str, kind: &str, root: &str, obj: &Path) {
    let j = serde_json::json!({ "name": name, "target_kind": kind, "source_root": root, "object_path": obj.display().to_string() });
    fs::write(at, j.to_string()).unwrap();
}

fn names(dir: &Path, ext: &str) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.ends_with(ext)).collect();
    v.sort();
    v
}

/// prev/, cur/ (with .stale) and cas/ under a fresh root, with the state a check finds when it starts.
fn setup(tag: &str) -> (std::path::PathBuf, SystemTime) {
    let root = std::env::temp_dir().join(format!("ridx_last_good_{tag}_{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let (prev, cur, cas) = (root.join("prev"), root.join("cur"), root.join("cas"));
    for d in [prev.join(".deps"), cur.join(".deps"), cur.join(".stale/.deps"), cas.clone()] {
        fs::create_dir_all(d).unwrap();
    }
    // The app's lib and bin were indexed; both were invalidated (their sidecars set aside), shards left in place.
    fs::write(cur.join("app-lib1.ridx"), b"old lib").unwrap();
    fs::write(cur.join("app-bin1.ridx"), b"old bin").unwrap();
    sidecar(&cur.join(".stale/.deps/app-lib1.json"), "app", "lib", "/ws/app", &cur.join("app-lib1.ridx"));
    sidecar(&cur.join(".stale/.deps/app-bin1.json"), "app", "bin", "/ws/app", &cur.join("app-bin1.ridx"));
    // A dependency, invalidated too: its object lives in the shared cache.
    fs::write(cas.join("dep.ridx"), b"dep").unwrap();
    sidecar(&cur.join(".stale/.deps/dep.json"), "dep", "lib", "/reg/dep-1.0", &cas.join("dep.ridx"));
    // A unit the previous variant has and this one never had.
    fs::write(prev.join("tests-aa.ridx"), b"prev").unwrap();
    sidecar(&prev.join(".deps/tests-aa.json"), "tests", "test", "/ws/app", &prev.join("tests-aa.ridx"));
    let started = SystemTime::now();
    std::thread::sleep(Duration::from_millis(30));
    // This check rebuilt the lib only, into a new shard.
    fs::write(cur.join("app-lib2.ridx"), b"new lib").unwrap();
    sidecar(&cur.join(".deps/app-lib2.json"), "app", "lib", "/ws/app", &cur.join("app-lib2.ridx"));
    (root, started)
}

#[test]
fn test_a_failed_check_keeps_each_unit_it_did_not_rebuild() {
    let (root, started) = setup("failed");
    let cur = root.join("cur");
    settle_after_check(&cur, started, false, &cur.join(".stale"), Some(&root.join("prev")));
    assert_eq!(names(&cur, ".ridx"), ["app-bin1.ridx", "app-lib2.ridx", "tests-aa.ridx"], "the bin kept, the old lib gone, the prev unit adopted");
    assert_eq!(names(&cur.join(".deps"), ".json"), ["app-bin1.json", "app-lib2.json", "dep.json", "tests-aa.json"], "the dependency's sidecar is back");
    let adopted: serde_json::Value = serde_json::from_slice(&fs::read(cur.join(".deps/tests-aa.json")).unwrap()).unwrap();
    assert_eq!(adopted["object_path"], cur.join("tests-aa.ridx").display().to_string());
    assert!(root.join("cas/dep.ridx").is_file(), "a dependency's object is never removed");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_a_check_that_succeeded_drops_what_it_superseded() {
    let (root, started) = setup("ok");
    let cur = root.join("cur");
    // An earlier lib sidecar still in .deps, superseded by the rebuild.
    fs::write(cur.join("app-lib0.ridx"), b"older").unwrap();
    sidecar(&cur.join(".deps/app-lib0.json"), "app", "lib", "/ws/app", &cur.join("app-lib0.ridx"));
    let old = SystemTime::now() - Duration::from_secs(60);
    fs::File::options().write(true).open(cur.join(".deps/app-lib0.json")).unwrap().set_modified(old).unwrap();
    settle_after_check(&cur, started, true, &cur.join(".stale"), Some(&root.join("prev")));
    assert_eq!(names(&cur, ".ridx"), ["app-lib2.ridx"], "set-aside and superseded shards go, nothing adopted");
    assert_eq!(names(&cur.join(".deps"), ".json"), ["app-lib2.json"]);
    let _ = fs::remove_dir_all(&root);
}
