/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use super::unit_match::UnitMatchIndex;
use crate::reader::IndexReader;
use crate::writer::IndexWriter;
use std::collections::HashMap;
use std::fs::File;
use tempfile::TempDir;

/// Original linear implementation of `matches_unit` used as an oracle for verification.
fn matches_unit_oracle<B: AsRef<[u8]>>(
    readers: &[IndexReader<B>],
    unit_keys: &[Option<String>],
    reader_extern_units: &[HashMap<String, String>],
    reader_idx: usize,
    ref_target_unit: &str,
    target_unit: &str,
    target_crate: &str,
) -> bool {
    if ref_target_unit == target_unit {
        return true;
    }
    let target_unit_in_crate = readers.iter().enumerate().any(|(j, r)| r.crate_name() == target_crate && unit_keys.get(j).and_then(|u| u.as_deref()) == Some(target_unit))
        || reader_extern_units.iter().any(|m| m.get(target_crate).map(|u| u.as_str()) == Some(target_unit));
    if !target_unit_in_crate {
        return false;
    }

    let is_reader_extern_target = reader_extern_units.get(reader_idx).and_then(|m| m.get(target_crate)).map(|u| u.as_str()) == Some(ref_target_unit);
    let target_versions_count =
        readers.iter().filter(|r| r.crate_name() == target_crate).map(|r| r.crate_version()).filter(|v| !v.is_empty()).collect::<std::collections::HashSet<_>>().len();
    let belongs_to_target_crate =
        is_reader_extern_target || readers.iter().enumerate().any(|(j, r)| r.crate_name() == target_crate && unit_keys.get(j).and_then(|u| u.as_deref()) == Some(ref_target_unit));
    belongs_to_target_crate && target_versions_count <= 1
}

#[test]
fn test_unit_match_oracle_equivalence() {
    let temp = TempDir::new().unwrap();

    // Setup 4 index files:
    // 0: foo v1.0.0 (unit_foo_1)
    // 1: foo v2.0.0 (unit_foo_2) - multi-version crate
    // 2: bar v0.1.0 (unit_bar_lib)
    // 3: bar v0.1.0 (unit_bar_bin) - single-version crate with multiple units
    let specs = [("foo", "1.0.0", "unit_foo_1"), ("foo", "2.0.0", "unit_foo_2"), ("bar", "0.1.0", "unit_bar_lib"), ("bar", "0.1.0", "unit_bar_bin")];

    let mut readers = Vec::new();
    let mut unit_keys = Vec::new();

    for (i, &(name, ver, ukey)) in specs.iter().enumerate() {
        let path = temp.path().join(format!("idx_{i}.ridx"));
        let mut writer = IndexWriter::new(name, ver);
        let mut f = File::create(&path).unwrap();
        writer.write_to(&mut f).unwrap();
        drop(f);
        readers.push(IndexReader::open(&path).unwrap());
        unit_keys.push(Some(ukey.to_string()));
    }

    let mut reader_extern_units = vec![HashMap::new(); 4];
    // reader 0 has extern alias: baz -> unit_baz_ext
    reader_extern_units[0].insert("baz".to_string(), "unit_baz_ext".to_string());
    // reader 2 has extern alias: foo -> unit_foo_1
    reader_extern_units[2].insert("foo".to_string(), "unit_foo_1".to_string());

    let unit_index = UnitMatchIndex::build(&readers, &unit_keys, &reader_extern_units);

    // Multi-version crate checks
    assert_eq!(unit_index.crate_versions_count("foo"), 2);
    assert_eq!(unit_index.crate_versions_count("bar"), 1);
    assert_eq!(unit_index.crate_versions_count("baz"), 0);

    // Exhaustive test across all combinations of:
    // reader_idx, ref_target_unit, target_unit, target_crate
    let test_units = ["unit_foo_1", "unit_foo_2", "unit_bar_lib", "unit_bar_bin", "unit_baz_ext", "unknown_unit"];
    let test_crates = ["foo", "bar", "baz", "quux"];

    for r_idx in 0..readers.len() {
        for &ref_uk in &test_units {
            for &target_uk in &test_units {
                for &target_crate in &test_crates {
                    let oracle_res = matches_unit_oracle(&readers, &unit_keys, &reader_extern_units, r_idx, ref_uk, target_uk, target_crate);
                    let index_res = unit_index.matches_unit(r_idx, ref_uk, target_uk, target_crate, &reader_extern_units);
                    assert_eq!(index_res, oracle_res, "Mismatch for r_idx={r_idx}, ref_uk={ref_uk}, target_uk={target_uk}, target_crate={target_crate}");
                }
            }
        }
    }
}
