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
