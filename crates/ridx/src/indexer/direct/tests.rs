/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use super::config::compute_workspace_config_hashes;
use super::try_direct_crate_indexing;
use crate::indexer::types::IndexOptions;
use std::fs;
use std::path::Path;
use std::time::Instant;

const VARIANT: &str = "variants/dev-1";

/// An indexed one-crate workspace: its manifest names `active` as the published variant, from a check that built
/// every crate.
fn indexed_workspace(root: &Path, active: &str) {
    partly_indexed_workspace(root, active, true);
}

fn partly_indexed_workspace(root: &Path, active: &str, complete: bool) {
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("Cargo.toml"), "[package]\nname = \"w\"\nversion = \"0.1.0\"\n").unwrap();
    let lib = root.join("src/lib.rs");
    fs::write(&lib, "pub fn a() {}\n").unwrap();
    let deps = root.join("target/ridx").join(VARIANT).join(".deps");
    fs::create_dir_all(&deps).unwrap();
    let sidecar = serde_json::json!({
        "name": "w",
        "compiler_args": ["--crate-name", "w", "src/lib.rs"],
        "env": { "CARGO_MANIFEST_DIR": root.display().to_string(), "CARGO_PKG_NAME": "w" },
        "file_hashes": { lib.display().to_string(): ridx_format::fnv1a_64(b"pub fn a() {}\n") },
    });
    fs::write(deps.join("w-1.json"), serde_json::to_vec(&sidecar).unwrap()).unwrap();
    let manifest = serde_json::json!({ "variant_dir": active, "config_hashes": compute_workspace_config_hashes(root), "complete": complete });
    fs::write(root.join("target/ridx/ridx_manifest.json"), serde_json::to_vec(&manifest).unwrap()).unwrap();
}

/// Runs the fast path with a driver that does not exist: any compilation fails it.
fn reindex(root: &Path) -> bool {
    let base = root.join("target/ridx");
    let variant = base.join(VARIANT);
    let missing_driver = root.join("no-driver");
    let opts = IndexOptions::default();
    try_direct_crate_indexing(
        &variant.join(".deps"),
        &variant,
        &base,
        root,
        &root.join("Cargo.toml"),
        &missing_driver,
        &root.join("cache"),
        None,
        "dev-1",
        VARIANT,
        "dev",
        &opts,
        Instant::now(),
    )
    .is_some()
}

/// On BTCR a watched-file report of the whole tree (Zed after a rescan) ran a full `cargo check` for an index nothing
/// had changed: about 70 s, with rustc and ridx-driver at 1.5–3 GB.
#[test]
fn test_an_unchanged_workspace_is_not_rebuilt() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    indexed_workspace(&root, VARIANT);
    let manifest = root.join("target/ridx/ridx_manifest.json");
    let before = fs::read(&manifest).unwrap();
    assert!(reindex(&root), "nothing changed: up to date, no compilation");
    assert_eq!(fs::read(&manifest).unwrap(), before, "the published manifest is left as it is");

    fs::write(root.join("src/lib.rs"), "pub fn a() { let _x = 1; }\n").unwrap();
    assert!(!reindex(&root), "an edited source is compiled");

    indexed_workspace(&root, VARIANT);
    fs::write(root.join("Cargo.toml"), "[package]\nname = \"w\"\nversion = \"0.2.0\"\n").unwrap();
    assert!(!reindex(&root), "a changed Cargo.toml goes to Cargo");

    indexed_workspace(&root, "variants/release-1");
    assert!(!reindex(&root), "another variant is published: this one is built and published");

    // Review of 3690a98d: a crate that failed in the last check has no sidecar, so nothing here sees it change.
    partly_indexed_workspace(&root, VARIANT, false);
    assert!(!reindex(&root), "the last check did not build every crate: Cargo runs again");
}

/// On BTCR an edit of one file compiled 53 sidecars: each of ~20 units under its two or three names, and the bare
/// `name.json`, never rewritten by the driver, kept the hashes of the text before.
#[test]
fn test_a_unit_is_compiled_once_and_all_its_names_follow() {
    use super::units::{group_by_unit, hash_matches, refresh_aliases};
    let dir = tempfile::tempdir().unwrap();
    let entry = |stem: &str| (stem.to_string(), "w".to_string(), Vec::new(), Default::default(), Default::default(), dir.path().join(format!("{stem}.json")));
    let obj = |stem: &str| Some(format!("/v/{stem}.ridx"));
    let sidecars = vec![
        (entry("w"), obj("w-aa")),
        (entry("w_00000000000000ff"), obj("w-aa")),
        (entry("w-aa"), obj("w-aa")),
        (entry("w-bb"), obj("w-bb")),
        (entry("w_00000000000000ee"), obj("w-bb")),
        (entry("old"), None),
    ];
    let (units, aliases) = group_by_unit(sidecars);
    let mut ids: Vec<&str> = units.iter().map(|u| u.0.as_str()).collect();
    ids.sort();
    assert_eq!(ids, ["old", "w-aa", "w-bb"], "one entry per object, described by the sidecar named after it");
    let names = |id: &str| aliases[id].iter().map(|p| p.file_stem().unwrap().to_string_lossy().to_string()).collect::<Vec<_>>();
    assert_eq!(names("w-aa"), ["w_00000000000000ff", "w"]);
    assert_eq!(names("w-bb"), ["w_00000000000000ee"]);
    // Review of 3690a98d: without the sidecar named after the object, the bare `name.json` the driver never rewrites
    // is not the one copied over the rest.
    let (units, _) = group_by_unit(vec![(entry("w"), obj("w-cc")), (entry("w_00000000000000dd"), obj("w-cc"))]);
    assert_eq!(units[0].0, "w_00000000000000dd");

    fs::write(dir.path().join("w-aa.json"), "{\"fresh\":1}").unwrap();
    fs::write(dir.path().join("w.json"), "{\"stale\":1}").unwrap();
    refresh_aliases(&dir.path().join("w-aa.json"), &aliases["w-aa"]);
    assert_eq!(fs::read_to_string(dir.path().join("w.json")).unwrap(), "{\"fresh\":1}");
    assert_eq!(fs::read_to_string(dir.path().join("w_00000000000000ff.json")).unwrap(), "{\"fresh\":1}");

    let crlf = b"fn a() {}\r\n";
    assert!(hash_matches(crlf, ridx_format::normalized_source_hash(crlf)), "the driver records the normalized hash");
    assert!(hash_matches(crlf, ridx_format::fnv1a_64(crlf)) && !hash_matches(b"fn b() {}\n", ridx_format::fnv1a_64(crlf)));
}
