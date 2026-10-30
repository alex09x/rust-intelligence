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
fn test_reader_corrupted_offsets_and_bounds() {
    let temp_dir = std::env::temp_dir().join(format!("ridx_rdr_corrupt_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&temp_dir);
    let idx_path = temp_dir.join("test.ridx");

    let mut writer = crate::writer::IndexWriter::new("corrupt_crate", "0.1.0");
    let mut f = std::fs::File::create(&idx_path).unwrap();
    writer.write_to(&mut f).unwrap();
    drop(f);

    let mut reader = IndexReader::open(&idx_path).unwrap();
    // Modify header offsets to exceed file bounds:
    reader.header.defs_offset = 1_000_000;
    reader.header.defs_count = 10;
    assert_eq!(reader.definitions().len(), 0);

    reader.header.defs_by_hash_offset = 1_000_000;
    reader.header.defs_by_hash_count = 10;
    assert_eq!(reader.defs_by_hash().len(), 0);

    reader.header.refs_offset = 1_000_000;
    reader.header.refs_count = 10;
    assert_eq!(reader.references().len(), 0);

    reader.header.refs_by_target_offset = 1_000_000;
    reader.header.refs_by_target_count = 10;
    assert_eq!(reader.refs_by_target().len(), 0);

    reader.header.types_offset = 1_000_000;
    reader.header.types_count = 10;
    assert_eq!(reader.types().len(), 0);

    let _ = std::fs::remove_dir_all(&temp_dir);

    // ResolvedDefinition resolved_path with None source_root (lines 45-47)
    let resolved = ResolvedDefinition {
        name: "test",
        path: "test::fn",
        kind: DefKind::Fn,
        signature: None,
        file: "src/lib.rs",
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 10,
        is_macro_expansion: false,
        is_reexport: false,
        source_root: None,
        unit_key: None,
        reader_idx: None,
    };
    assert_eq!(resolved.resolved_path(), std::path::PathBuf::from("src/lib.rs"));
}

#[test]
fn test_type_at_binary_search_vs_linear_property() {
    let temp_dir = std::env::temp_dir().join(format!("ridx_test_types_prop_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&temp_dir);
    let idx_path = temp_dir.join("nested_types.ridx");
    let mut writer = crate::writer::IndexWriter::new("test_types", "0.1.0");

    writer.add_definition(crate::writer::RawDefinition {
        def_id: 1,
        kind: crate::format::DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "dummy".to_string(),
        path: "test::dummy".to_string(),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 5,
    });

    // File 1: src/lib.rs
    // Outer expression: foo(...) spanning line 10, col 5 to line 12, col 20 (size: 2*80 + 20 = 180)
    writer.add_type(crate::writer::RawType { file: "src/lib.rs".to_string(), start_line: 10, start_col: 5, end_line: 12, end_col: 20, type_name: "TypeFoo".to_string() });
    // Middle expression: bar(...) spanning line 11, col 8 to line 11, col 30 (size: 22)
    writer.add_type(crate::writer::RawType { file: "src/lib.rs".to_string(), start_line: 11, start_col: 8, end_line: 11, end_col: 30, type_name: "TypeBar".to_string() });
    // Inner expression: baz(...) spanning line 11, col 12 to line 11, col 25 (size: 13)
    writer.add_type(crate::writer::RawType { file: "src/lib.rs".to_string(), start_line: 11, start_col: 12, end_line: 11, end_col: 25, type_name: "TypeBaz".to_string() });
    // Innermost expression: x spanning line 11, col 16 to line 11, col 17 (size: 1)
    writer.add_type(crate::writer::RawType { file: "src/lib.rs".to_string(), start_line: 11, start_col: 16, end_line: 11, end_col: 17, type_name: "TypeX".to_string() });
    // Sibling expression: y spanning line 11, col 20 to line 11, col 22 (size: 2)
    writer.add_type(crate::writer::RawType { file: "src/lib.rs".to_string(), start_line: 11, start_col: 20, end_line: 11, end_col: 22, type_name: "TypeY".to_string() });

    // Same start pos expressions: a.b.c()
    writer.add_type(crate::writer::RawType { file: "src/lib.rs".to_string(), start_line: 20, start_col: 5, end_line: 20, end_col: 6, type_name: "TypeA".to_string() });
    writer.add_type(crate::writer::RawType { file: "src/lib.rs".to_string(), start_line: 20, start_col: 5, end_line: 20, end_col: 8, type_name: "TypeAB".to_string() });
    writer.add_type(crate::writer::RawType { file: "src/lib.rs".to_string(), start_line: 20, start_col: 5, end_line: 20, end_col: 12, type_name: "TypeABC".to_string() });

    // File 2: src/other.rs with adjacent spans
    writer.add_type(crate::writer::RawType { file: "src/other.rs".to_string(), start_line: 5, start_col: 0, end_line: 5, end_col: 10, type_name: "OtherSpan1".to_string() });
    writer.add_type(crate::writer::RawType { file: "src/other.rs".to_string(), start_line: 5, start_col: 10, end_line: 5, end_col: 20, type_name: "OtherSpan2".to_string() });

    let mut f = std::fs::File::create(&idx_path).unwrap();
    writer.write_to(&mut f).unwrap();
    drop(f);

    let reader = IndexReader::open(&idx_path).unwrap();

    // Linear reference implementation (old behavior)
    let type_at_linear = |file: &str, line: u32, col: u32| -> Option<String> {
        let file_id = reader.find_file_id(file)?;
        let mut best: Option<(&TypeEntry, u64)> = None;
        for entry in reader.types() {
            let span = entry.span();
            if span.file_id() == file_id && span.contains(file_id, line, col) {
                let size = span.approximate_size();
                match best {
                    None => best = Some((entry, size)),
                    Some((_, best_size)) if size < best_size => best = Some((entry, size)),
                    _ => {}
                }
            }
        }
        best.map(|(e, _)| reader.get_str(e.type_offset()).to_string())
    };

    // Grid check across all coordinates in both files and missing file
    for file in &["src/lib.rs", "src/other.rs", "missing.rs"] {
        for line in 0..30 {
            for col in 0..40 {
                let expected = type_at_linear(file, line, col);
                let actual = reader.type_at(file, line, col).map(|s| s.to_string());
                assert_eq!(actual, expected, "Mismatch at file={file}, line={line}, col={col}: actual={actual:?}, expected={expected:?}");
            }
        }
    }

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_type_at_large_scale_benchmark() {
    let temp_dir = std::env::temp_dir().join(format!("ridx_test_types_scale_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&temp_dir);
    let idx_path = temp_dir.join("large_types.ridx");
    let mut writer = crate::writer::IndexWriter::new("test_scale", "0.1.0");

    writer.add_definition(crate::writer::RawDefinition {
        def_id: 1,
        kind: crate::format::DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "main".to_string(),
        path: "test::main".to_string(),
        signature: None,
        file: "src/main.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 5,
    });

    // Generate 100,000 synthetic types across multiple files
    let num_files = 50;
    let types_per_file = 2_000;
    for f in 0..num_files {
        let file_name = format!("src/file_{}.rs", f);
        for i in 0..types_per_file {
            let line = (i * 2 + 1) as u32;
            writer.add_type(crate::writer::RawType {
                file: file_name.clone(),
                start_line: line,
                start_col: 5,
                end_line: line,
                end_col: 25,
                type_name: format!("Type_{}_{}", f, i),
            });
        }
    }

    let mut f = std::fs::File::create(&idx_path).unwrap();
    writer.write_to(&mut f).unwrap();
    drop(f);

    let reader = IndexReader::open(&idx_path).unwrap();
    assert_eq!(reader.types().len(), 100_000);

    // Benchmark queries across 100 sample locations
    let samples: Vec<(String, u32, u32)> = (0..100)
        .map(|i| {
            let file = format!("src/file_{}.rs", (i * 7) % num_files);
            let line = ((i * 13) % types_per_file) as u32 * 2 + 1;
            (file, line, 10)
        })
        .collect();

    // 1. Time binary search (new implementation)
    let t0 = std::time::Instant::now();
    for (file, line, col) in &samples {
        let res = reader.type_at(file, *line, *col);
        assert!(res.is_some());
    }
    let dur_binary = t0.elapsed();

    // 2. Time linear scan (old implementation)
    let type_at_linear = |file: &str, line: u32, col: u32| -> Option<&str> {
        let file_id = reader.find_file_id(file)?;
        let mut best: Option<(&TypeEntry, u64)> = None;
        for entry in reader.types() {
            let span = entry.span();
            if span.file_id() == file_id && span.contains(file_id, line, col) {
                let size = span.approximate_size();
                match best {
                    None => best = Some((entry, size)),
                    Some((_, best_size)) if size < best_size => best = Some((entry, size)),
                    _ => {}
                }
            }
        }
        best.map(|(e, _)| reader.get_str(e.type_offset()))
    };

    let t1 = std::time::Instant::now();
    for (file, line, col) in &samples {
        let res = type_at_linear(file, *line, *col);
        assert!(res.is_some());
    }
    let dur_linear = t1.elapsed();

    eprintln!(
        "\n[SCALE BENCH] 100,000 Types table — 100 queries:\n  Linear Scan:   {:?}\n  Binary Search: {:?}\n  Speedup:       {:.1}x",
        dur_linear,
        dur_binary,
        dur_linear.as_nanos() as f64 / dur_binary.as_nanos().max(1) as f64
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}
