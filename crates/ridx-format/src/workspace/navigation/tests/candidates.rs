/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use super::super::*;
use super::*;

#[test]
fn test_cross_crate_navigation_candidates_and_type_at() {
    let test_dir = new_test_dir("ridx_cand_nav");
    let manifest_file = test_dir.0.join("ridx_manifest.json");

    let dep_root = test_dir.0.join("dep_src");
    fs::create_dir_all(dep_root.join("src")).unwrap();
    fs::write(dep_root.join("src/lib.rs"), "pub fn my_fn() {}\n").unwrap();

    let mut w_caller = IndexWriter::new("caller", "0.1.0");
    w_caller.set_unit_key("caller_key");
    // Reference to unresolvable cross-crate symbol (hits fallback lines 181-192 in find_definition_at)
    w_caller.add_reference(RawReference {
        file: "src/main.rs".to_string(),
        start_line: 15,
        start_col: 5,
        end_line: 15,
        end_col: 15,
        target_def_id: None,
        target_path: "missing_dep::missing_fn".to_string(),
        target_unit: None,
        target_file: Some("src/lib.rs".to_string()),
        target_line: 10,
        target_col: 1,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });

    // Reference to ambiguous candidates (hits lines 293-298)
    w_caller.add_reference(RawReference {
        file: "src/main.rs".to_string(),
        start_line: 30,
        start_col: 5,
        end_line: 30,
        end_col: 15,
        target_def_id: None,
        target_path: "ambig::func".to_string(),
        target_unit: None,
        target_file: Some("other/file.rs".to_string()),
        target_line: 50,
        target_col: 1,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });

    // Reference matching line 50 and file src/a.rs (hits lines 289-291)
    w_caller.add_reference(RawReference {
        file: "src/main.rs".to_string(),
        start_line: 35,
        start_col: 5,
        end_line: 35,
        end_col: 15,
        target_def_id: None,
        target_path: "ambig::func".to_string(),
        target_unit: None,
        target_file: Some("src/a.rs".to_string()),
        target_line: 50,
        target_col: 1,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });

    // Reference where target_line does not match (hits line 511 continue)
    w_caller.add_reference(RawReference {
        file: "src/main.rs".to_string(),
        start_line: 40,
        start_col: 5,
        end_line: 40,
        end_col: 15,
        target_def_id: None,
        target_path: "ambig::func".to_string(),
        target_unit: None,
        target_file: Some("src/a.rs".to_string()),
        target_line: 99,
        target_col: 1,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });

    // Reference with existing target_unit but missing symbol (hits lines 300-302)
    w_caller.add_reference(RawReference {
        file: "src/main.rs".to_string(),
        start_line: 45,
        start_col: 5,
        end_line: 45,
        end_col: 15,
        target_def_id: None,
        target_path: "ambig::nonexistent_symbol".to_string(),
        target_unit: Some("ambig_key1".to_string()),
        target_file: None,
        target_line: 0,
        target_col: 0,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });

    // Add a type in caller
    w_caller.add_type(RawType { file: "src/main.rs".to_string(), start_line: 15, start_col: 5, end_line: 15, end_col: 15, type_name: "MyType".to_string() });

    let var_dir = test_dir.0.join("variants/var_nav");
    fs::create_dir_all(&var_dir).unwrap();

    let mut f_caller = File::create(var_dir.join("caller.ridx")).unwrap();
    w_caller.write_to(&mut f_caller).unwrap();
    drop(f_caller);

    // Ambig shard 1
    let mut w_a1 = IndexWriter::new("ambig", "1.0.0");
    w_a1.set_unit_key("ambig_key1");
    w_a1.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "func".to_string(),
        path: "ambig::func".to_string(),
        signature: Some("fn func() -> i32".to_string()),
        file: "src/a.rs".to_string(),
        start_line: 50,
        start_col: 1,
        end_line: 52,
        end_col: 2,
    });
    let mut f_a1 = File::create(var_dir.join("ambig1.ridx")).unwrap();
    w_a1.write_to(&mut f_a1).unwrap();
    drop(f_a1);

    // Ambig shard 2
    let mut w_a2 = IndexWriter::new("ambig", "2.0.0");
    w_a2.set_unit_key("ambig_key2");
    w_a2.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "func".to_string(),
        path: "ambig::func".to_string(),
        signature: Some("fn func() -> i64".to_string()),
        file: "src/b.rs".to_string(),
        start_line: 50,
        start_col: 1,
        end_line: 52,
        end_col: 2,
    });
    w_a2.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::Struct,
        visibility: 0,
        is_macro_expansion: false,
        name: "NoSig".to_string(),
        path: "ambig::NoSig".to_string(),
        signature: None,
        file: "src/b.rs".to_string(),
        start_line: 60,
        start_col: 1,
        end_line: 62,
        end_col: 2,
    });
    let mut f_a2 = File::create(var_dir.join("ambig2.ridx")).unwrap();
    w_a2.write_to(&mut f_a2).unwrap();
    drop(f_a2);

    // Dep shard with source_root
    let mut w_dep = IndexWriter::new("dep_crate", "0.1.0");
    w_dep.set_unit_key("dep_key");
    w_dep.add_type(RawType { file: "src/lib.rs".to_string(), start_line: 1, start_col: 1, end_line: 1, end_col: 10, type_name: "DepType".to_string() });
    let dep_file = test_dir.0.join("dep.ridx");
    let mut f_dep = File::create(&dep_file).unwrap();
    w_dep.write_to(&mut f_dep).unwrap();
    drop(f_dep);

    // Write variant manifest linking dep_crate with source_root
    let manifest_data = serde_json::json!({
        "version": "2.0",
        "format_version": crate::CURRENT_FORMAT_VERSION,
        "extractor_version": crate::EXTRACTOR_VERSION,
        "active_variant_id": "var_nav",
        "variant_dir": "variants/var_nav",
        "workspace_root": test_dir.0.display().to_string(),
        "shards": [
            { "name": "caller", "version": "0.1.0", "file": "caller.ridx", "is_dependency": false },
            { "name": "ambig", "version": "1.0.0", "file": "ambig1.ridx", "is_dependency": false },
            { "name": "ambig", "version": "2.0.0", "file": "ambig2.ridx", "is_dependency": false }
        ],
        "dependency_shards": [
            {
                "name": "dep_crate",
                "version": "0.1.0",
                "unit_key": "dep_key",
                "object_path": dep_file.display().to_string(),
                "source_root": dep_root.display().to_string(),
                "is_dependency": true,
                "dependencies": [],
                "extern_units": {},
                "file_hashes": {}
            }
        ]
    });
    fs::write(&manifest_file, serde_json::to_string(&manifest_data).unwrap()).unwrap();

    let ws = WorkspaceIndex::open(&test_dir.0).unwrap();

    // 1. Fallback unresolvable cross-crate definition at cursor (lines 181-192)
    let unres_def = ws.find_definition_at("src/main.rs", 15, 8);
    assert!(unres_def.is_some());
    let ud = unres_def.unwrap();
    assert_eq!(ud.path, "missing_dep::missing_fn");
    assert_eq!(ud.unit_key, Some("caller_key"));

    // 2. Ambiguous candidate resolution matching line 50 (lines 293-298)
    let ambig_def = ws.find_definition_at("src/main.rs", 30, 8);
    assert!(ambig_def.is_some());

    // 2b. Line and file match (lines 289-291)
    let exact_def = ws.find_definition_at("src/main.rs", 35, 8);
    assert!(exact_def.is_some());
    assert_eq!(exact_def.unwrap().file, "src/a.rs");

    // 2c. Ambiguous candidate without line match (line 298)
    let no_line_match_def = ws.find_definition_at("src/main.rs", 40, 8);
    assert!(no_line_match_def.is_some());

    // 2d. Target unit mismatch returns None from find_cross_definition, falling back to raw ref (lines 300-302)
    let unit_mismatch_def = ws.find_definition_at("src/main.rs", 45, 8);
    assert!(unit_mismatch_def.is_some());
    assert_eq!(unit_mismatch_def.unwrap().path, "ambig::nonexistent_symbol");

    // 2e. Definition with signature None (line 65)
    let nosig = ws.find_definition_by_path("ambig::NoSig");
    assert!(nosig.is_some());
    assert_eq!(nosig.unwrap().signature, None);

    // 3. type_at with absolute path under dep source root (lines 625-627)
    let abs_dep_path = dep_root.join("src/lib.rs");
    let ty_dep = ws.type_at(abs_dep_path.to_str().unwrap(), 1, 5);
    assert_eq!(ty_dep, Some("DepType"));

    // 4. type_at with absolute path outside dep root (line 629, 638, 644)
    let abs_other = test_dir.0.join("other/src/lib.rs");
    let ty_other = ws.type_at(abs_other.to_str().unwrap(), 1, 5);
    assert_eq!(ty_other, None);

    // 5. type_at with relative path (line 632)
    let ty_caller = ws.type_at("src/main.rs", 15, 8);
    assert_eq!(ty_caller, Some("MyType"));

    // 6. find_references_for_unit with coordinate matching (lines 504-528)
    let refs1 = ws.find_references_for_unit("ambig::func", Some("ambig_key1"));
    assert_eq!(refs1.len(), 1);
    assert_eq!(refs1[0].span.start_line(), 35);

    // 7. resolve_path_for_crate with root_dir and None workspace_root (lines 111-115)
    let mut ws_no_ws_root = ws;
    ws_no_ws_root.workspace_root = None;
    ws_no_ws_root.root_dir = Some(test_dir.0.clone());
    let resolved = ws_no_ws_root.resolve_path_for_crate("unknown", "src/lib.rs");
    assert_eq!(resolved, test_dir.0.join("src/lib.rs"));

    ws_no_ws_root.root_dir = None;
    let resolved_plain = ws_no_ws_root.resolve_path_for_crate("unknown", "src/lib.rs");
    assert_eq!(resolved_plain, PathBuf::from("src/lib.rs"));
}
