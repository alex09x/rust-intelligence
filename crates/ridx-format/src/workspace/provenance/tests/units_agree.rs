/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Whether the compilation units of a file recorded the same source text.

use super::*;
use std::fs;

/// The two units of one crate (`lib` and `lib test`), each recording `hash_lib` / `hash_test` for `src/lib.rs`.
fn two_units(root: &std::path::Path, hash_lib: u64, hash_test: u64) -> WorkspaceIndex {
    let crate_dir = root.join("shop");
    fs::create_dir_all(crate_dir.join("src")).unwrap();
    fs::write(crate_dir.join("src/lib.rs"), "pub fn a() {}\n").unwrap();
    let var_dir = root.join("target/ridx/variants/v1");
    fs::create_dir_all(var_dir.join(".deps")).unwrap();
    for (i, (unit, hash)) in [("uk_lib", hash_lib), ("uk_test", hash_test)].into_iter().enumerate() {
        let mut w = IndexWriter::new("shop", "0.1.0");
        w.set_unit_key(unit);
        w.get_or_insert_file("src/lib.rs");
        w.add_file_hash("src/lib.rs", hash);
        let object = var_dir.join(format!("0{i}_shop.ridx"));
        w.write_to(&mut File::create(&object).unwrap()).unwrap();
        let sidecar = serde_json::json!({ "name": "shop", "version": "0.1.0", "unit_key": unit, "object_path": object.display().to_string(),
            "source_root": crate_dir.display().to_string(), "is_dependency": false, "file_hashes": { "src/lib.rs": hash } });
        fs::write(var_dir.join(format!(".deps/0{i}_shop.json")), sidecar.to_string()).unwrap();
    }
    let manifest = serde_json::json!({ "version": "2.0", "format_version": crate::format::CURRENT_FORMAT_VERSION, "extractor_version": crate::format::EXTRACTOR_VERSION,
        "active_variant_id": "v1", "variant_dir": "variants/v1", "workspace_root": root.display().to_string(), "dependency_shards": [] });
    fs::write(root.join("target/ridx/ridx_manifest.json"), manifest.to_string()).unwrap();
    WorkspaceIndex::open(root).unwrap()
}

/// In Zed: saved with an error in `mod tests`, the `lib` unit built the new text while `lib test` failed and kept the
/// text before. An open buffer rebased onto the new text read the test module's data on the wrong lines (its
/// parameter hints went missing); it is rebased only when the units agree.
#[test]
fn test_units_agree_on_source() {
    let agree = tempfile::tempdir().unwrap();
    let ws = two_units(agree.path(), 0xAAAA, 0xAAAA);
    let file = agree.path().join("shop/src/lib.rs").display().to_string();
    assert_eq!(ws.source_hash_for_file(&file), Some(0xAAAA));
    assert!(ws.units_agree_on_source(&file));

    let split = tempfile::tempdir().unwrap();
    let ws = two_units(split.path(), 0xAAAA, 0xBBBB);
    let file = split.path().join("shop/src/lib.rs").display().to_string();
    assert!(ws.source_hash_for_file(&file).is_some());
    assert!(!ws.units_agree_on_source(&file), "one unit holds the text before");
}

/// A unit: its crate directory, the files its shard holds, the hashes and `own_sources` its sidecar records.
struct Unit<'a> {
    key: &'a str,
    krate: &'a str,
    shard: &'a [(&'a str, u64)],
    hashes: serde_json::Value,
    own: Vec<String>,
}

/// A crate named `.` is the workspace root package, named `w`.
fn name_of<'a>(u: &Unit<'a>) -> &'a str {
    if u.krate == "." {
        "w"
    } else {
        u.krate
    }
}

fn dir_of(root: &std::path::Path, u: &Unit) -> std::path::PathBuf {
    if u.krate == "." {
        root.to_path_buf()
    } else {
        root.join(u.krate)
    }
}

fn sidecar_of(root: &std::path::Path, i: usize, u: &Unit) -> (std::path::PathBuf, serde_json::Value) {
    let var_dir = root.join("target/ridx/variants/v1");
    let object = var_dir.join(format!("0{i}_{}.ridx", name_of(u)));
    let sidecar = serde_json::json!({ "name": name_of(u), "version": "0.1.0", "unit_key": u.key, "object_path": object.display().to_string(),
        "source_root": dir_of(root, u).display().to_string(), "is_dependency": false, "file_hashes": u.hashes, "own_sources": u.own });
    (var_dir.join(format!(".deps/0{i}_{}.json", name_of(u))), sidecar)
}

fn units(root: &std::path::Path, units: &[Unit]) -> WorkspaceIndex {
    let var_dir = root.join("target/ridx/variants/v1");
    fs::create_dir_all(var_dir.join(".deps")).unwrap();
    for (i, u) in units.iter().enumerate() {
        fs::create_dir_all(dir_of(root, u).join("src")).unwrap();
        let mut w = IndexWriter::new(name_of(u), "0.1.0");
        w.set_unit_key(u.key);
        for (f, h) in u.shard {
            w.get_or_insert_file(f);
            w.add_file_hash(f, *h);
        }
        w.write_to(&mut File::create(var_dir.join(format!("0{i}_{}.ridx", name_of(u)))).unwrap()).unwrap();
        let (path, sidecar) = sidecar_of(root, i, u);
        fs::write(path, sidecar.to_string()).unwrap();
    }
    let manifest = serde_json::json!({ "version": "2.0", "format_version": crate::format::CURRENT_FORMAT_VERSION, "extractor_version": crate::format::EXTRACTOR_VERSION,
        "active_variant_id": "v1", "variant_dir": "variants/v1", "workspace_root": root.display().to_string(), "dependency_shards": [] });
    fs::write(root.join("target/ridx/ridx_manifest.json"), manifest.to_string()).unwrap();
    WorkspaceIndex::open(root).unwrap()
}

/// Review of 2f7a84e3: whether a unit compiled a file comes from the sidecar loaded with its hashes, not from the one
/// on disk now. `app` read `shop/src/gen.rs` through `shop`'s metadata: its hash there is not `app`'s text of it, even
/// after the sidecar on disk was rewritten (a newer build) while the loaded index is unchanged.
#[test]
fn test_ownership_comes_from_the_loaded_sidecar() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("shop/src")).unwrap();
    fs::write(root.join("shop/src/gen.rs"), "pub fn g() {}\n").unwrap();
    let gen = root.join("shop/src/gen.rs").display().to_string();
    let app = Unit {
        key: "uk_app",
        krate: "app",
        shard: &[("src/main.rs", 0x11)],
        hashes: serde_json::json!({ (gen.clone()): 0xBBBB }),
        own: vec![root.join("app/src/main.rs").display().to_string()],
    };
    let shop = Unit { key: "uk_shop", krate: "shop", shard: &[("src/gen.rs", 0xAAAA)], hashes: serde_json::json!({ "src/gen.rs": 0xAAAA }), own: vec![gen.clone()] };
    let ws = units(root, &[app, shop]);
    assert!(ws.units_agree_on_source(&gen), "a hash read through a dependency is no unit's text of the file");

    let rewritten = Unit { key: "uk_app", krate: "app", shard: &[], hashes: serde_json::json!({ (gen.clone()): 0xBBBB }), own: vec![gen.clone()] };
    let (path, sidecar) = sidecar_of(root, 0, &rewritten);
    std::thread::sleep(std::time::Duration::from_millis(20));
    fs::write(path, sidecar.to_string()).unwrap();
    assert!(ws.units_agree_on_source(&gen), "the sidecar on disk now is not the one loaded");
}

/// Review of 2f7a84e3: a sidecar names a crate's files relative to the crate's own root, not the workspace's. `shop`'s
/// `lib test` failed with `src/gen.rs` holding the text before: its relative entry still counts.
#[test]
fn test_a_relative_sidecar_key_resolves_against_its_reader_root() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("shop/src")).unwrap();
    fs::write(root.join("shop/src/gen.rs"), "pub fn g() {}\n").unwrap();
    let gen = root.join("shop/src/gen.rs").display().to_string();
    let lib = Unit { key: "uk_lib", krate: "shop", shard: &[("src/gen.rs", 0xAAAA)], hashes: serde_json::json!({ "src/gen.rs": 0xAAAA }), own: vec![gen.clone()] };
    let test = Unit { key: "uk_test", krate: "shop", shard: &[("src/lib.rs", 0x22)], hashes: serde_json::json!({ "src/gen.rs": 0xBBBB }), own: vec![gen.clone()] };
    let ws = units(root, &[lib, test]);
    assert!(!ws.units_agree_on_source(&gen), "lib test holds the text before");
}

/// Review of c94616ac: a hash is weighed with the ownership of the file it names, never of another file another
/// resolution of the query names. `member` imported `W/src/gen.rs` (its hash there is not its text of it) and owns its
/// own `W/member/src/gen.rs`, both only in its sidecar: asked as `src/gen.rs` or absolutely, `W/src/gen.rs` has one text.
#[test]
fn test_one_alias_does_not_prove_ownership_of_another() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::create_dir_all(root.join("member/src")).unwrap();
    fs::write(root.join("src/gen.rs"), "pub fn g() {}\n").unwrap();
    fs::write(root.join("member/src/gen.rs"), "pub fn m() {}\n").unwrap();
    let gen = root.join("src/gen.rs").display().to_string();
    let own_member = root.join("member/src/gen.rs").display().to_string();
    let a = Unit { key: "uk_w", krate: ".", shard: &[("src/gen.rs", 0x111)], hashes: serde_json::json!({ "src/gen.rs": 0x111 }), own: vec![gen.clone()] };
    let b =
        Unit { key: "uk_member", krate: "member", shard: &[("src/lib.rs", 0x3)], hashes: serde_json::json!({ (gen.clone()): 0x222, "src/gen.rs": 0x333 }), own: vec![own_member] };
    let ws = units(root, &[a, b]);
    assert!(ws.units_agree_on_source(&gen), "member only imported W/src/gen.rs");
    assert!(ws.units_agree_on_source("src/gen.rs"), "the relative query names the same file");
}

/// Review of c94616ac: a file asked for through a symlink is the file it leads to: a unit recording its hash only
/// relative to its root (`src/gen.rs`) still counts.
#[cfg(unix)]
#[test]
fn test_a_symlinked_query_finds_a_relative_only_hash() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("shop/src")).unwrap();
    fs::write(root.join("shop/src/gen.rs"), "pub fn g() {}\n").unwrap();
    std::os::unix::fs::symlink(root.join("shop/src"), root.join("alias")).unwrap();
    let gen = root.join("shop/src/gen.rs").display().to_string();
    let a = Unit { key: "uk_a", krate: "shop", shard: &[("src/lib.rs", 0x1)], hashes: serde_json::json!({ (gen.clone()): 0x111 }), own: vec![gen.clone()] };
    let b = Unit { key: "uk_b", krate: "shop", shard: &[("src/lib.rs", 0x2)], hashes: serde_json::json!({ "src/gen.rs": 0x222 }), own: vec![gen.clone()] };
    let ws = units(root, &[a, b]);
    assert!(!ws.units_agree_on_source(&gen), "asked directly");
    assert!(!ws.units_agree_on_source(&root.join("alias/gen.rs").display().to_string()), "asked through the symlink");
}

/// Review of 4d8906dc: routing may store a crate's file relative to the workspace (`shop/src/gen.rs`) while the crate's
/// root is `W/shop`. An absolute query stays the file asked for: the unit recording its text only in the sidecar,
/// under that workspace-relative name, still counts.
#[test]
fn test_a_workspace_relative_routed_key_keeps_its_owner() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("shop/src")).unwrap();
    fs::write(root.join("shop/src/gen.rs"), "pub fn g() {}\n").unwrap();
    let gen = root.join("shop/src/gen.rs").display().to_string();
    let a = Unit { key: "uk_a", krate: "shop", shard: &[("shop/src/gen.rs", 0x111)], hashes: serde_json::json!({}), own: vec![gen.clone()] };
    let b = Unit { key: "uk_b", krate: "shop", shard: &[("shop/src/gen.rs", 0)], hashes: serde_json::json!({ "shop/src/gen.rs": 0x222 }), own: vec![gen.clone()] };
    let ws = units(root, &[a, b]);
    assert_eq!(ws.target_file_for_reader(1, &gen).as_deref(), Some("shop/src/gen.rs"));
    assert!(!ws.units_agree_on_source(&gen), "B holds another text");
}

/// Review of 4d8906dc: routing a query through a symlink to the canonical entry keeps the query's own spelling among
/// the file's names: a sidecar recording the hash under it still counts.
#[cfg(unix)]
#[test]
fn test_the_query_spelling_survives_canonical_routing() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("shop/src")).unwrap();
    fs::write(root.join("shop/src/gen.rs"), "pub fn g() {}\n").unwrap();
    std::os::unix::fs::symlink(root.join("shop/src"), root.join("alias")).unwrap();
    let canonical = fs::canonicalize(root.join("shop/src/gen.rs")).unwrap().display().to_string();
    let alias = root.join("alias/gen.rs").display().to_string();
    let entry: &[(&str, u64)] = &[(canonical.as_str(), 0x111)];
    let zero: &[(&str, u64)] = &[(canonical.as_str(), 0)];
    let a = Unit { key: "uk_a", krate: "shop", shard: entry, hashes: serde_json::json!({}), own: vec![canonical.clone()] };
    let b = Unit { key: "uk_b", krate: "shop", shard: zero, hashes: serde_json::json!({ (alias.clone()): 0x222 }), own: vec![canonical.clone()] };
    let ws = units(root, &[a, b]);
    assert!(!ws.units_agree_on_source(&alias), "B holds another text");
}
