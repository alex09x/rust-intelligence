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
use scip::types::{Document, Index, Occurrence, SymbolInformation, SymbolRole};
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
