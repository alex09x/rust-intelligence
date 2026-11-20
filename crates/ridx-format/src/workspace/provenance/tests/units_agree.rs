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

fn sidecar_of(root: &std::path::Path, i: usize, u: &Unit) -> (std::path::PathBuf, serde_json::Value) {
    let var_dir = root.join("target/ridx/variants/v1");
    let object = var_dir.join(format!("0{i}_{}.ridx", u.krate));
    let sidecar = serde_json::json!({ "name": u.krate, "version": "0.1.0", "unit_key": u.key, "object_path": object.display().to_string(),
        "source_root": root.join(u.krate).display().to_string(), "is_dependency": false, "file_hashes": u.hashes, "own_sources": u.own });
    (var_dir.join(format!(".deps/0{i}_{}.json", u.krate)), sidecar)
}

fn units(root: &std::path::Path, units: &[Unit]) -> WorkspaceIndex {
    let var_dir = root.join("target/ridx/variants/v1");
    fs::create_dir_all(var_dir.join(".deps")).unwrap();
    for (i, u) in units.iter().enumerate() {
        fs::create_dir_all(root.join(u.krate).join("src")).unwrap();
        let mut w = IndexWriter::new(u.krate, "0.1.0");
        w.set_unit_key(u.key);
        for (f, h) in u.shard {
            w.get_or_insert_file(f);
            w.add_file_hash(f, *h);
        }
        w.write_to(&mut File::create(var_dir.join(format!("0{i}_{}.ridx", u.krate))).unwrap()).unwrap();
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
