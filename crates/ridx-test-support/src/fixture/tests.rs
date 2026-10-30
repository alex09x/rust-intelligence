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

#[test]
fn test_fixture_builder_unicode_geometry() {
    let mut builder = FixtureIndexBuilder::new("unicode_test", "0.1.0");
    // "/* 🦀 */ fn «test»() {}"
    // Chars before «test»: "/* " (3) + "🦀" (1) + " */ fn " (7) = 11 chars.
    // 1-based column for driver = 11 + 1 = 12.
    let clean = builder.add_defs_from_fixture("/* 🦀 */ fn «test»() {}", "/src/lib.rs", "unicode_test", DefKind::Fn);
    assert_eq!(clean, "/* 🦀 */ fn test() {}");

    let temp = tempfile::tempdir().unwrap();
    let ridx_path = temp.path().join("unicode.ridx");
    builder.write_to_file(&ridx_path);

    let reader = ridx_format::IndexReader::open(&ridx_path).unwrap();
    let defs = reader.definitions();
    assert_eq!(defs.len(), 1);
    let def = &defs[0];
    let name = reader.get_str(def.name_offset);
    assert_eq!(name, "test");
    assert_eq!(def.span.start_line(), 1);
    // Verify 1-based char column exactly matches driver:
    assert_eq!(def.span.start_col(), 12);
    assert_eq!(def.span.end_col(), 16);
}

#[test]
fn test_fixture_builder_auto_linked_references() {
    let mut builder = FixtureIndexBuilder::new("link_test", "0.1.0");
    let (clean_lib, def_ids) = builder.add_defs_with_ids("pub fn «compute»() {}", "/src/lib.rs", "link_test", DefKind::Fn);
    assert_eq!(clean_lib, "pub fn compute() {}");
    assert_eq!(def_ids.len(), 1);

    let clean_caller = builder.add_refs_to_def("fn main() { link_test::«compute»(); }", "/src/main.rs", def_ids[0]);
    assert_eq!(clean_caller, "fn main() { link_test::compute(); }");

    let temp = tempfile::tempdir().unwrap();
    let ridx_path = temp.path().join("link.ridx");
    builder.write_to_file(&ridx_path);

    let reader = ridx_format::IndexReader::open(&ridx_path).unwrap();
    let refs = reader.references();
    assert_eq!(refs.len(), 1);
    let r = &refs[0];
    assert_eq!(r.target_def_id, def_ids[0]);
    assert_eq!(reader.get_str(r.target_path_offset), "link_test::compute");
    assert_eq!(r.target_line, 1);
    assert_eq!(r.target_col, 8); // "pub fn " is 7 chars, 1-based = 8
}

#[test]
fn test_fixture_builder_defs_and_self_refs() {
    let mut builder = FixtureIndexBuilder::new("self_ref_test", "0.1.0");
    let (clean, def_ids) = builder.add_defs_and_self_refs("pub struct «Config» { val: i32 }", "/src/lib.rs", "self_ref_test", DefKind::Struct);
    assert_eq!(clean, "pub struct Config { val: i32 }");
    assert_eq!(def_ids.len(), 1);

    let temp = tempfile::tempdir().unwrap();
    let ridx_path = temp.path().join("self_ref.ridx");
    builder.write_to_file(&ridx_path);

    let reader = ridx_format::IndexReader::open(&ridx_path).unwrap();
    let defs = reader.definitions();
    let refs = reader.references();
    assert_eq!(defs.len(), 1);
    assert_eq!(refs.len(), 1);
    assert_eq!(defs[0].span.start_col(), 12); // "pub struct " is 11 chars -> 1-based 12
    assert_eq!(defs[0].span.end_col(), 18);
    assert_eq!(refs[0].span.start_col(), 12);
    assert_eq!(refs[0].span.end_col(), 18);
    assert_eq!(refs[0].target_def_id, def_ids[0]);
}

#[test]
fn test_synthetic_workspace_coordinates() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("core_pkg", "0.1.0").unwrap();

    let code = "/* 🦀 */ pub fn «grüßen»() {}\n";
    let (clean, def_ids) = ws.add_source_file_with_defs("core_pkg", "src/lib.rs", code, DefKind::Fn).unwrap();

    assert_eq!(clean, "/* 🦀 */ pub fn grüßen() {}\n");
    assert_eq!(def_ids.len(), 1);

    assert_eq!(ws.crate_version("core_pkg"), Some("0.1.0"));

    let ridx_path = ws.build_crate_index("core_pkg").unwrap();
    assert!(ridx_path.is_file());

    let reader = ridx_format::IndexReader::open(&ridx_path).unwrap();
    let defs = reader.definitions();
    assert_eq!(defs.len(), 1);

    let def = &defs[0];
    let name = reader.get_str(def.name_offset);
    assert_eq!(name, "grüßen");
    assert_eq!(def.span.start_line(), 1);
    assert_eq!(def.span.start_col(), 16);
    assert_eq!(def.span.end_col(), 22);

    // Verify state is preserved across repeated builds
    let ridx_path2 = ws.build_crate_index("core_pkg").unwrap();
    let reader2 = ridx_format::IndexReader::open(&ridx_path2).unwrap();
    assert_eq!(reader2.definitions().len(), 1);

    // Verify sorted build_all_indices and write_workspace_manifest
    ws.add_crate("extra_pkg", "0.2.0").unwrap();
    let manifest = ws.write_workspace_manifest().unwrap();
    assert!(manifest.is_file());
    let manifest_text = std::fs::read_to_string(&manifest).unwrap();
    assert!(manifest_text.contains("\"core_pkg\""));
    assert!(manifest_text.contains("\"extra_pkg\""));

    let all_shards = ws.build_all_indices().unwrap();
    assert_eq!(all_shards.len(), 2);
}

#[test]
fn test_synthetic_workspace_foreign_references() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("provider_pkg", "0.1.0").unwrap();
    ws.add_crate("consumer_pkg", "0.1.0").unwrap();

    let (_, def_ids) = ws.add_source_file_with_defs("provider_pkg", "src/lib.rs", "pub struct «ExportedCore»;\n", DefKind::Struct).unwrap();

    let clean = ws
        .add_refs_to_foreign_def("consumer_pkg", "src/lib.rs", "use provider_pkg::ExportedCore;\npub fn run() { let _ = «ExportedCore»; }\n", "provider_pkg", def_ids[0])
        .unwrap();

    assert!(clean.contains("let _ = ExportedCore;"));

    let shard = ws.build_crate_index("consumer_pkg").unwrap();
    let reader = ridx_format::IndexReader::open(&shard).unwrap();
    let refs = reader.references();
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].target_def_id, def_ids[0]);
    assert_eq!(reader.get_str(refs[0].target_path_offset), "provider_pkg::ExportedCore");
    assert_eq!(refs[0].kind, DefKind::Struct as u8);
}
