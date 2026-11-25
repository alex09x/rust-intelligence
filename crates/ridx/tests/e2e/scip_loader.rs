/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use protobuf::Message;
use ridx::compare::scip_loader::{run_scip_session, ScipIndex};
use ridx::compare::types::SymmetricScenario;
use scip::types::occurrence::Typed_range;
use scip::types::{Document, Index, MultiLineRange, Occurrence, SingleLineRange, SymbolInformation, SymbolRole};
use std::fs;

#[test]
fn test_scip_loader_and_queries() {
    let temp_dir = std::env::temp_dir().join(format!("scip_test_{}", std::process::id()));
    fs::create_dir_all(&temp_dir).unwrap();
    let scip_file = temp_dir.join("index.scip");

    let mut index = Index::new();
    let mut doc = Document::new();
    doc.relative_path = "src/main.rs".to_string();

    // Occurrence 1: Definition of my_func at line 5, col 4..11 (0-indexed)
    let mut occ_def = Occurrence::new();
    occ_def.range = vec![5, 4, 11];
    occ_def.symbol = "rust-project my_crate src/main.rs/my_func().".to_string();
    occ_def.symbol_roles = SymbolRole::Definition as i32;
    doc.occurrences.push(occ_def);

    // Occurrence 2: Reference to my_func at line 10, col 8..15
    let mut occ_ref = Occurrence::new();
    occ_ref.range = vec![10, 8, 15];
    occ_ref.symbol = "rust-project my_crate src/main.rs/my_func().".to_string();
    occ_ref.symbol_roles = 0;
    doc.occurrences.push(occ_ref);

    // Symbol info with doc
    let mut sym_info = SymbolInformation::new();
    sym_info.symbol = "rust-project my_crate src/main.rs/my_func().".to_string();
    sym_info.documentation = vec!["Documentation for my_func".to_string()];
    doc.symbols.push(sym_info);

    index.documents.push(doc);

    let bytes = index.write_to_bytes().unwrap();
    fs::write(&scip_file, bytes).unwrap();

    let scip = ScipIndex::load_from_file(&scip_file, &temp_dir).unwrap();
    assert_eq!(scip.workspace_root, temp_dir);

    // Test definition query at reference location (line 10, col 8) -> line 5, col 4
    let def_loc = scip.goto_definition("src/main.rs", 10, 8).expect("def found");
    assert_eq!(def_loc.file, "src/main.rs");
    assert_eq!(def_loc.line, 5);
    assert_eq!(def_loc.col, 4);

    // Test references query at definition location
    let refs = scip.find_references("src/main.rs", 5, 4);
    assert_eq!(refs.len(), 2); // includes def + 1 ref
    assert_eq!(refs[0].line, 5);
    assert_eq!(refs[1].line, 10);
    assert_eq!(refs[1].col, 8);

    // Test hover query
    let hover = scip.hover("src/main.rs", 5, 4).expect("hover doc found");
    assert_eq!(hover, "Documentation for my_func");

    // Test run_scip_session
    let scenarios = vec![
        SymmetricScenario {
            name: "test_def".to_string(),
            capability: "def".to_string(),
            target: "my_func".to_string(),
            location: Some(("src/main.rs".to_string(), 11, 9)), // 1-based (line 11, col 9 -> 0-based 10, 8)
            mutation: None,
        },
        SymmetricScenario {
            name: "test_hover".to_string(),
            capability: "hover".to_string(),
            target: "my_func".to_string(),
            location: Some(("src/main.rs".to_string(), 6, 5)), // 1-based (line 6, col 5 -> 0-based 5, 4)
            mutation: None,
        },
    ];

    let metrics = run_scip_session(&scip_file, &temp_dir, &scenarios).expect("scip session ran");
    assert_eq!(metrics.server_name, "rust-analyzer-scip");
    assert_eq!(metrics.query_results.len(), 2);
    assert!(metrics.query_results["test_def"].found);
    assert!(metrics.query_results["test_hover"].found);
    assert_eq!(metrics.query_results["test_hover"].span, "Documentation for my_func");

    let _ = fs::remove_dir_all(&temp_dir);
}

/// Reported by the EB-1A workspace against the SCIP schema since scip-code/scip PR 387: an occurrence with only the
/// typed range was skipped, and a four-element `range` was read with its end line as the end column.
#[test]
fn test_scip_loader_reads_every_range_encoding() {
    let temp_dir = std::env::temp_dir().join(format!("scip_ranges_{}", std::process::id()));
    fs::create_dir_all(&temp_dir).unwrap();
    let scip_file = temp_dir.join("index.scip");
    let occ = |symbol: &str, range: Vec<i32>, typed: Option<Typed_range>| {
        let mut o = Occurrence::new();
        o.symbol = symbol.to_string();
        o.symbol_roles = SymbolRole::Definition as i32;
        o.range = range;
        o.typed_range = typed;
        o
    };
    let single = |line, start_character, end_character| Typed_range::SingleLineRange(SingleLineRange { line, start_character, end_character, ..Default::default() });
    let multi = |start_line, start_character, end_line, end_character| {
        Typed_range::MultiLineRange(MultiLineRange { start_line, start_character, end_line, end_character, ..Default::default() })
    };
    let mut doc = Document::new();
    doc.relative_path = "src/lib.rs".to_string();
    doc.occurrences = vec![
        occ("old_single", vec![1, 4, 9], None),
        occ("old_multi", vec![3, 2, 5, 1], None),
        occ("typed_single", Vec::new(), Some(single(7, 4, 10))),
        occ("typed_multi", Vec::new(), Some(multi(9, 0, 12, 1))),
        // Both set: the typed range takes precedence.
        occ("both", vec![20, 0, 3], Some(single(14, 6, 8))),
    ];
    let mut index = Index::new();
    index.documents.push(doc);
    fs::write(&scip_file, index.write_to_bytes().unwrap()).unwrap();

    let scip = ScipIndex::load_from_file(&scip_file, &temp_dir).unwrap();
    let at = |symbol: &str| scip.definitions.get(symbol).map(|l| (l.line, l.col));
    assert_eq!(at("old_single"), Some((1, 4)));
    assert_eq!(at("old_multi"), Some((3, 2)));
    assert_eq!(at("typed_single"), Some((7, 4)), "a typed-only occurrence is kept");
    assert_eq!(at("typed_multi"), Some((9, 0)));
    assert_eq!(at("both"), Some((14, 6)));
    // A range going on past its first line holds the rest of that line, not up to its end line's number.
    assert_eq!(scip.find_symbol_at("src/lib.rs", 3, 40), Some("old_multi"));
    assert_eq!(scip.find_symbol_at("src/lib.rs", 9, 40), Some("typed_multi"));
    assert_eq!(scip.occurrences_by_line[&("src/lib.rs".to_string(), 1)][0].1, 9, "a single-line range ends at its end column");
    assert!(!scip.occurrences_by_line.contains_key(&("src/lib.rs".to_string(), 20)), "the deprecated range of an occurrence with a typed one is ignored");

    let _ = fs::remove_dir_all(&temp_dir);
}
