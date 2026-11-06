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
    // A reference to a build no reader holds matches the build the reader links for that crate.
    let linked = reader_extern_units.get(reader_idx).and_then(|m| m.get(target_crate)).map(|u| u.as_str());
    let links_target_instead = !unit_keys.iter().any(|u| u.as_deref() == Some(ref_target_unit))
        && linked.is_some_and(|l| readers.iter().enumerate().any(|(j, r)| r.crate_name() == target_crate && unit_keys.get(j).and_then(|u| u.as_deref()) == Some(l)));
    let version_of = |uk: &str| {
        readers.iter().enumerate().find(|(j, r)| r.crate_name() == target_crate && unit_keys.get(*j).and_then(|u| u.as_deref()) == Some(uk)).map(|(_, r)| r.crate_version())
    };
    if links_target_instead {
        // Stands for the linked build: same version as the target when the crate has several.
        return target_versions_count <= 1 || (version_of(target_unit).is_some() && version_of(target_unit) == linked.and_then(version_of));
    }
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

#[test]
fn test_unit_match_fast_file_and_crate_lookups() {
    let temp = TempDir::new().unwrap();
    let specs = [("alpha", "0.1.0", "u_alpha", "src/lib.rs"), ("beta", "0.2.0", "u_beta_1", "src/main.rs"), ("beta", "0.2.0", "u_beta_2", "src/./helper.rs")];
    let mut readers = Vec::new();
    let mut unit_keys = Vec::new();
    for (i, &(name, ver, ukey, file_path)) in specs.iter().enumerate() {
        let path = temp.path().join(format!("fast_{i}.ridx"));
        let mut writer = IndexWriter::new(name, ver);
        writer.get_or_insert_file(file_path);
        let mut f = File::create(&path).unwrap();
        writer.write_to(&mut f).unwrap();
        drop(f);
        readers.push(IndexReader::open(&path).unwrap());
        unit_keys.push(Some(ukey.to_string()));
    }
    let reader_extern_units = vec![HashMap::new(); 3];
    let idx = UnitMatchIndex::build(&readers, &unit_keys, &reader_extern_units);

    assert!(idx.is_valid_for_readers(&readers));
    assert_eq!(idx.readers_for_crate("alpha"), Some(&[0][..]));
    assert_eq!(idx.readers_for_crate("beta"), Some(&[1, 2][..]));
    assert_eq!(idx.readers_for_crate("gamma"), None);
    assert_eq!(idx.readers_for_unit("u_beta_1"), Some(&[1][..]));
    assert_eq!(idx.readers_for_unit("missing"), None);

    assert_eq!(idx.find_file_id_fast(0, &readers[0], "src/lib.rs"), Some(Some(0)));
    assert_eq!(idx.find_file_id_fast(0, &readers[0], "./src/lib.rs"), Some(Some(0)));
    assert_eq!(idx.find_file_id_fast(0, &readers[0], "src/other.rs"), Some(None));
    assert_eq!(idx.find_file_id_fast(2, &readers[2], "src/helper.rs"), None);
    assert_eq!(readers[2].find_file_id("src/helper.rs"), Some(0));

    // Replacing a middle shard invalidates is_valid_for_readers
    let mid_path = temp.path().join("fast_mid_replacement.ridx");
    let mut writer = IndexWriter::new("delta", "0.3.0");
    writer.get_or_insert_file("src/delta.rs");
    writer.write_to(&mut File::create(&mid_path).unwrap()).unwrap();
    readers[1] = IndexReader::open(&mid_path).unwrap();
    assert!(!idx.is_valid_for_readers(&readers));
    assert_eq!(idx.find_file_id_fast(1, &readers[1], "src/main.rs"), None);
}

#[test]
fn test_unit_match_same_version_multi_unit_with_older_dep_version() {
    let temp = TempDir::new().unwrap();
    let specs = [("pkg", "2.0.0", "u_pkg_lib"), ("pkg", "2.0.0", "u_pkg_test"), ("pkg", "1.0.0", "u_pkg_old_dep")];
    let mut readers = Vec::new();
    let mut unit_keys = Vec::new();
    for (i, &(name, ver, ukey)) in specs.iter().enumerate() {
        let path = temp.path().join(format!("multi_{i}.ridx"));
        let mut writer = IndexWriter::new(name, ver);
        writer.write_to(&mut File::create(&path).unwrap()).unwrap();
        readers.push(IndexReader::open(&path).unwrap());
        unit_keys.push(Some(ukey.to_string()));
    }
    let reader_extern_units = vec![HashMap::new(); 3];
    let idx = UnitMatchIndex::build(&readers, &unit_keys, &reader_extern_units);

    assert_eq!(idx.crate_versions_count("pkg"), 2);
    assert!(idx.matches_unit(0, "u_pkg_test", "u_pkg_lib", "pkg", &reader_extern_units));
    assert!(idx.matches_unit(1, "u_pkg_lib", "u_pkg_test", "pkg", &reader_extern_units));
    assert!(!idx.matches_unit(0, "u_pkg_old_dep", "u_pkg_lib", "pkg", &reader_extern_units));
    assert!(!idx.matches_unit(2, "u_pkg_lib", "u_pkg_old_dep", "pkg", &reader_extern_units));
}

#[test]
fn test_warm_background_caches_populates_reachability_and_assoc_ty() {
    let temp = TempDir::new().unwrap();
    let p0 = temp.path().join("c0.ridx");
    let p1 = temp.path().join("c1.ridx");
    let mut w0 = IndexWriter::new("c0", "0.1.0");
    w0.set_unit_key("u0");
    w0.write_to(&mut File::create(&p0).unwrap()).unwrap();
    let mut w1 = IndexWriter::new("c1", "0.1.0");
    w1.set_unit_key("u1");
    w1.write_to(&mut File::create(&p1).unwrap()).unwrap();

    let mut ws = super::WorkspaceIndex::open(temp.path()).unwrap();
    let idx0 = ws.readers.iter().position(|r| r.crate_name() == "c0").unwrap();
    let idx1 = ws.readers.iter().position(|r| r.crate_name() == "c1").unwrap();
    ws.reader_dependencies[idx0].push("u1".to_string());
    ws.rebuild_unit_match();

    assert!(ws.unit_match.reachable_reader_bits.get().is_none());
    ws.warm_background_caches();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while ws.unit_match.reachable_reader_bits.get().is_none() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(ws.unit_match.reachable_reader_bits.get().is_some());
    assert!(ws.is_reader_in_dep_graph(&[idx0], idx1));
    assert!(!ws.is_reader_in_dep_graph(&[idx1], idx0));
}
