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
    reindex_with(root, &root.join("no-driver"))
}

fn reindex_with(root: &Path, driver: &Path) -> bool {
    let base = root.join("target/ridx");
    let variant = base.join(VARIANT);
    let opts = IndexOptions::default();
    try_direct_crate_indexing(
        &variant.join(".deps"),
        &variant,
        &base,
        root,
        &root.join("Cargo.toml"),
        driver,
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

/// prod-code, issue #7: an edit that broke the build fell back to Cargo after the direct compile failed (36 to 40 s on
/// their workspace), and Cargo stopped on the same errors. The driver indexes a workspace crate with errors and
/// rewrites its sidecar; only a compile that left no index goes to Cargo.
#[cfg(unix)]
#[test]
fn test_an_edit_with_errors_is_indexed_without_cargo() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let driver = |name: &str, body: &str| {
        let path = root.join(name);
        fs::write(&path, format!("#!/bin/sh\n{body}\nexit 1\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    };
    let indexes_with_errors = driver("errors-driver", "touch \"$RIDX_OUT_DIR/.deps/w-1.json\"");
    let leaves_nothing = driver("failing-driver", "");

    indexed_workspace(&root, VARIANT);
    // The crate's shard, which the publication opens.
    let mut shard = ridx_format::IndexWriter::new("w", "0.1.0");
    shard.write_to(&mut fs::File::create(root.join("target/ridx").join(VARIANT).join("w-1.ridx")).unwrap()).unwrap();
    fs::write(root.join("src/lib.rs"), "pub fn a() -> u32 { \"x\" }\n").unwrap();
    assert!(!reindex_with(&root, &leaves_nothing), "a compile that indexed nothing goes to Cargo");
    // A sidecar time changes past the filesystem's resolution.
    std::thread::sleep(std::time::Duration::from_millis(20));
    assert!(reindex_with(&root, &indexes_with_errors), "a crate with errors indexed by the driver is published");
    let manifest: serde_json::Value = serde_json::from_slice(&fs::read(root.join("target/ridx/ridx_manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["complete"], true, "the edit fixing it is compiled here too");
}

/// Found by the live probe of the change above: `mod missing;` indexed with errors, its integration test left out,
/// then the line removed again. The test's sidecar lists the lib's file as it imported it, with the hash of the text
/// before the error, and when it was read first the revert was taken for "up to date" over an index of the broken text.
#[test]
fn test_a_revert_after_errors_is_compiled_whichever_sidecar_is_read_first() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    indexed_workspace(&root, VARIANT);
    let lib = root.join("src/lib.rs");
    let (good, broken) = (b"pub fn a() {}\n".as_slice(), b"pub fn a() {}\nmod missing;\n".as_slice());
    let deps = root.join("target/ridx").join(VARIANT).join(".deps");
    let sidecar = |name: &str, own: &Path, hashes: serde_json::Value| {
        let j = serde_json::json!({
            "name": name,
            "compiler_args": ["--crate-name", name, own.display().to_string()],
            "env": { "CARGO_MANIFEST_DIR": root.display().to_string(), "CARGO_PKG_NAME": "w" },
            "own_sources": [own.display().to_string()],
            "file_hashes": hashes,
        });
        fs::write(deps.join(format!("{name}-1.json")), serde_json::to_vec(&j).unwrap()).unwrap();
    };
    let test_file = root.join("tests/t.rs");
    fs::create_dir_all(test_file.parent().unwrap()).unwrap();
    fs::write(&test_file, "fn t() {}\n").unwrap();
    // The lib indexed with errors; its test, left out, still lists the lib's text from before.
    sidecar("w", &lib, serde_json::json!({ lib.display().to_string(): ridx_format::normalized_source_hash(broken) }));
    sidecar(
        "t",
        &test_file,
        serde_json::json!({
            test_file.display().to_string(): ridx_format::normalized_source_hash(b"fn t() {}\n"),
            lib.display().to_string(): ridx_format::normalized_source_hash(good),
        }),
    );
    fs::write(&lib, good).unwrap();
    assert!(!reindex(&root), "the lib's own record is of the broken text: compiled, not up to date");
    // And the imported file is no change of the test's own: with the lib's record current, nothing is compiled.
    sidecar("w", &lib, serde_json::json!({ lib.display().to_string(): ridx_format::normalized_source_hash(good) }));
    sidecar(
        "t",
        &test_file,
        serde_json::json!({
            test_file.display().to_string(): ridx_format::normalized_source_hash(b"fn t() {}\n"),
            lib.display().to_string(): ridx_format::normalized_source_hash(broken),
        }),
    );
    assert!(reindex(&root), "up to date");
}

/// prod-code, issue #7: the first edit after a Cargo index compiled with an incremental cache of the direct path's own,
/// empty. A unit goes on with the cache its last compile recorded, Cargo's after a Cargo index; one outside the
/// workspace's `target` is not written to.
#[cfg(unix)]
#[test]
fn test_a_unit_keeps_the_incremental_cache_it_was_compiled_with() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let driver = root.join("recording-driver");
    fs::write(&driver, "#!/bin/sh\nprintf '%s\\n' \"$@\" > args.txt\ntouch \"$RIDX_OUT_DIR/.deps/w-1.json\"\n").unwrap();
    fs::set_permissions(&driver, fs::Permissions::from_mode(0o755)).unwrap();
    let outside = tempfile::tempdir().unwrap();
    let cargo_cache = root.join("target/ridx_cargo/dev/debug/incremental");
    let own_cache = root.join("target/ridx-direct/incremental");
    for (recorded, used) in [(cargo_cache.clone(), cargo_cache), (outside.path().to_path_buf(), own_cache)] {
        indexed_workspace(&root, VARIANT);
        let variant = root.join("target/ridx").join(VARIANT);
        // The crate's shard, which the publication opens.
        let mut shard = ridx_format::IndexWriter::new("w", "0.1.0");
        shard.write_to(&mut fs::File::create(variant.join("w-1.ridx")).unwrap()).unwrap();
        let sidecar = variant.join(".deps/w-1.json");
        let mut j: serde_json::Value = serde_json::from_slice(&fs::read(&sidecar).unwrap()).unwrap();
        j["compiler_args"] = serde_json::json!(["rustc", "src/lib.rs", "-C", format!("incremental={}", recorded.display())]);
        fs::write(&sidecar, serde_json::to_vec(&j).unwrap()).unwrap();
        fs::write(root.join("src/lib.rs"), "pub fn a() { let _x = 1; }\n").unwrap();
        assert!(reindex_with(&root, &driver));
        let args = fs::read_to_string(root.join("args.txt")).unwrap();
        let expected = format!("incremental={}", used.display());
        assert!(args.lines().any(|a| a == expected), "{expected} among {args}");
    }
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
    let now = super::units::source_hashes(crlf);
    assert!(hash_matches(now, ridx_format::normalized_source_hash(crlf)), "the driver records the normalized hash");
    assert!(hash_matches(now, ridx_format::fnv1a_64(crlf)) && !hash_matches(super::units::source_hashes(b"fn b() {}\n"), ridx_format::fnv1a_64(crlf)));
}
