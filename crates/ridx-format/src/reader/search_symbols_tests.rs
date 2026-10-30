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
fn test_contains_ignore_case_ascii_unit() {
    assert!(contains_ignore_case_ascii("hello world", b""));
    assert!(contains_ignore_case_ascii("hello world", b"hello"));
    assert!(contains_ignore_case_ascii("HELLO WORLD", b"hello"));
    assert!(contains_ignore_case_ascii("Hello World", b"world"));
    assert!(contains_ignore_case_ascii("Hello World", b"o w"));
    assert!(!contains_ignore_case_ascii("short", b"longer needle"));
    assert!(!contains_ignore_case_ascii("hello world", b"xyz"));
    assert!(contains_ignore_case_ascii("tokio::fs::File", b"file"));
    assert!(contains_ignore_case_ascii("tokio::fs::File", b"fs"));
    assert!(contains_ignore_case_ascii("tokio::fs::File", b"tokio"));
    // Haystack with non-ASCII UTF-8 (German/Greek)
    assert!(contains_ignore_case_ascii("Grüße World", b"world"));
    assert!(!contains_ignore_case_ascii("Grüße World", b"hello"));
    assert!(contains_ignore_case_ascii("Γειά World", b"world"));
    assert!(!contains_ignore_case_ascii("Γειά World", b"hello"));
}

#[test]
fn test_search_symbols_parity_with_naive() {
    let temp_dir = std::env::temp_dir().join(format!("ridx_search_syms_parity_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();
    let idx_path = temp_dir.join("test.ridx");

    let mut writer = crate::writer::IndexWriter::new("test_crate", "0.1.0");
    let raw_defs = [
        ("MyStruct", "test_crate::MyStruct"),
        ("my_function", "test_crate::my_function"),
        ("CONSTANT_VAL", "test_crate::CONSTANT_VAL"),
        ("inner_func", "test_crate::module::inner_func"),
        ("AnotherStruct", "test_crate::types::AnotherStruct"),
        ("GreekenFunktion", "test_crate::unicode::ΓειάΣας"),
    ];

    for (idx, (name, path)) in raw_defs.iter().enumerate() {
        writer.add_definition(crate::writer::RawDefinition {
            def_id: idx as u32 + 1,
            kind: crate::format::DefKind::Fn,
            visibility: 0,
            is_macro_expansion: false,
            name: name.to_string(),
            path: path.to_string(),
            file: "src/lib.rs".to_string(),
            start_line: 10,
            start_col: 1,
            end_line: 10,
            end_col: 20,
            signature: None,
        });
    }

    let mut f = std::fs::File::create(&idx_path).unwrap();
    writer.write_to(&mut f).unwrap();
    drop(f);

    let reader = IndexReader::open(&idx_path).unwrap();

    // Baseline naive matching function
    let naive_search = |q: &str| -> Vec<String> {
        let q_lower = q.to_lowercase();
        reader
            .definitions()
            .iter()
            .filter(|d| {
                let n = reader.get_str(d.name_offset()).to_lowercase();
                let p = reader.get_str(d.path_offset()).to_lowercase();
                n.contains(&q_lower) || p.contains(&q_lower)
            })
            .map(|d| reader.get_str(d.name_offset()).to_string())
            .collect()
    };

    let queries = ["", "struct", "STRUCT", "my", "val", "CONSTANT", "module", "another", "nonexistent", "Γειά", "Σας"];

    for q in queries {
        let expected = naive_search(q);
        let actual: Vec<String> = reader.search_symbols(q).into_iter().map(|d| reader.get_str(d.name_offset()).to_string()).collect();
        assert_eq!(actual, expected, "Query {:?} failed parity with naive search", q);
    }

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_search_symbols_scale_benchmark() {
    let temp_dir = std::env::temp_dir().join(format!("ridx_search_syms_scale_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();
    let idx_path = temp_dir.join("scale.ridx");

    let mut writer = crate::writer::IndexWriter::new("scale_crate", "0.1.0");
    let n_defs = 10_000;
    for i in 0..n_defs {
        writer.add_definition(crate::writer::RawDefinition {
            def_id: i as u32 + 1,
            kind: crate::format::DefKind::Fn,
            visibility: 0,
            is_macro_expansion: false,
            name: format!("SymbolName_{}_{}", i % 50, i),
            path: format!("scale_crate::module_{}::SymbolName_{}_{}", i % 20, i % 50, i),
            file: format!("src/mod_{}.rs", i % 30),
            start_line: (i % 500 + 1) as u32,
            start_col: 1,
            end_line: (i % 500 + 1) as u32,
            end_col: 25,
            signature: None,
        });
    }

    let mut f = std::fs::File::create(&idx_path).unwrap();
    writer.write_to(&mut f).unwrap();
    drop(f);

    let reader = IndexReader::open(&idx_path).unwrap();
    let queries = ["symbolname", "module_5", "name_25", "xyz_nonexistent", "scale_crate"];

    // 1. Time new implementation (zero-alloc ASCII)
    let t0 = std::time::Instant::now();
    for _ in 0..20 {
        for q in &queries {
            let res = reader.search_symbols(q);
            assert!(!res.is_empty() || *q == "xyz_nonexistent");
        }
    }
    let dur_new = t0.elapsed();

    // 2. Time old naive implementation
    let naive_search = |q: &str| -> Vec<&DefEntry> {
        let q_lower = q.to_lowercase();
        let mut matches = Vec::new();
        for def in reader.definitions() {
            let name = reader.get_str(def.name_offset());
            let path = reader.get_str(def.path_offset());
            if name.to_lowercase().contains(&q_lower) || path.to_lowercase().contains(&q_lower) {
                matches.push(def);
            }
        }
        matches
    };

    let t1 = std::time::Instant::now();
    for _ in 0..20 {
        for q in &queries {
            let res = naive_search(q);
            assert!(!res.is_empty() || *q == "xyz_nonexistent");
        }
    }
    let dur_old = t1.elapsed();

    eprintln!(
        "\n[SCALE BENCH] 10,000 Defs — 100 queries:\n  Naive (to_lowercase): {:?}\n  Zero-Alloc ASCII:    {:?}\n  Speedup:              {:.1}x",
        dur_old,
        dur_new,
        dur_old.as_nanos() as f64 / dur_new.as_nanos().max(1) as f64
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_search_symbols_and_fallback_ignore_typaram() {
    let temp_dir = std::env::temp_dir().join(format!("ridx_typaram_search_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();
    let idx_path = temp_dir.join("test.ridx");

    let mut writer = crate::writer::IndexWriter::new("serde", "1.0.0");
    writer.add_definition(crate::writer::RawDefinition {
        def_id: 1,
        kind: crate::format::DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "deserialize".to_string(),
        path: "serde::de::deserialize".to_string(),
        file: "src/de.rs".to_string(),
        start_line: 10,
        start_col: 1,
        end_line: 10,
        end_col: 20,
        signature: Some("fn deserialize<'de, D>(deserializer: D) -> Result<T, D::Error>".to_string()),
    });
    writer.add_definition(crate::writer::RawDefinition {
        def_id: 2,
        kind: crate::format::DefKind::TyParam,
        visibility: 0,
        is_macro_expansion: false,
        name: "D".to_string(),
        path: "serde::de::deserialize::D".to_string(),
        file: "src/de.rs".to_string(),
        start_line: 10,
        start_col: 21,
        end_line: 10,
        end_col: 22,
        signature: None,
    });
    writer.add_definition(crate::writer::RawDefinition {
        def_id: 3,
        kind: crate::format::DefKind::TyParam,
        visibility: 0,
        is_macro_expansion: false,
        name: "T".to_string(),
        path: "serde::de::deserialize::T".to_string(),
        file: "src/de.rs".to_string(),
        start_line: 10,
        start_col: 23,
        end_line: 10,
        end_col: 24,
        signature: None,
    });
    writer.add_definition(crate::writer::RawDefinition {
        def_id: 4,
        kind: crate::format::DefKind::Struct,
        visibility: 0,
        is_macro_expansion: false,
        name: "Deserializer".to_string(),
        path: "serde::de::Deserializer".to_string(),
        file: "src/de.rs".to_string(),
        start_line: 30,
        start_col: 1,
        end_line: 30,
        end_col: 25,
        signature: None,
    });

    let mut f = std::fs::File::create(&idx_path).unwrap();
    writer.write_to(&mut f).unwrap();
    drop(f);

    let reader = IndexReader::open(&idx_path).unwrap();

    // 1. search_symbols for "deserialize" must return Fn and Struct, but NOT TyParam D or T
    let matches = reader.search_symbols("deserialize");
    let matched_names: Vec<&str> = matches.iter().map(|d| reader.get_str(d.name_offset())).collect();
    assert_eq!(matched_names, vec!["deserialize", "Deserializer"]);

    // 2. search_symbols_ascii for "d" must return "deserialize" and "Deserializer", NOT "D"
    let matches_d = reader.search_symbols_ascii(b"d");
    let names_d: Vec<&str> = matches_d.iter().map(|d| reader.get_str(d.name_offset())).collect();
    assert!(!names_d.contains(&"D"), "TyParam D must not be returned in symbol search");
    assert!(names_d.contains(&"deserialize"));
    assert!(names_d.contains(&"Deserializer"));

    // 3. search_symbols for "T" must return empty, NOT TyParam T
    let matches_t = reader.search_symbols("T");
    assert!(matches_t.is_empty(), "TyParam T must not be returned in symbol search");

    // 4. for_each_symbol_ascii with needle "deserialize":
    // Budget must NOT be consumed by TyParam entries
    let mut budget = 10usize;
    let mut collected = Vec::new();
    reader.for_each_symbol_ascii(b"deserialize", &mut budget, |def| {
        collected.push(reader.get_str(def.name_offset()).to_string());
        true
    });
    assert_eq!(collected, vec!["deserialize", "Deserializer"]);
    // Only 2 real items consumed budget: 10 - 2 = 8. TyParam entries did NOT consume budget!
    assert_eq!(budget, 8, "TyParam entries must not consume search budget");

    // 5. for_each_symbol_ascii_budgeted:
    // When record_budget is strictly smaller than total definitions, it stops early and returns false.
    let mut match_budget = 10usize;
    let mut record_budget = 1usize;
    let mut collected_budgeted = Vec::new();
    let completed = reader.for_each_symbol_ascii_budgeted(b"deserialize", &mut match_budget, &mut record_budget, |def| {
        collected_budgeted.push(reader.get_str(def.name_offset()).to_string());
        true
    });
    assert!(!completed, "Must return false when record budget is exhausted");
    assert_eq!(record_budget, 0, "Record budget must be fully exhausted");

    let _ = std::fs::remove_dir_all(&temp_dir);
}
