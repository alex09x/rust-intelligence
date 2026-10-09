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
    let clean = builder.add_defs_from_fixture(
        "/* 🦀 */ fn «test»() {}",
        "/src/lib.rs",
        "unicode_test",
        DefKind::Fn,
    );
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
    let (clean_lib, def_ids) = builder.add_defs_with_ids(
        "pub fn «compute»() {}",
        "/src/lib.rs",
        "link_test",
        DefKind::Fn,
    );
    assert_eq!(clean_lib, "pub fn compute() {}");
    assert_eq!(def_ids.len(), 1);

    let clean_caller = builder.add_refs_to_def(
        "fn main() { link_test::«compute»(); }",
        "/src/main.rs",
        def_ids[0],
    );
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
    let (clean, def_ids) = builder.add_defs_and_self_refs(
        "pub struct «Config» { val: i32 }",
        "/src/lib.rs",
        "self_ref_test",
        DefKind::Struct,
    );
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
