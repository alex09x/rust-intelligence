/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Unit tests for authoritative target classification and workspace target-kind routing.

use super::target_kind::{classify_target, TargetKind};
use super::WorkspaceIndex;
use crate::writer::IndexWriter;
use std::collections::HashMap;
use std::fs::File;
use std::path::PathBuf;

#[test]
fn test_classification_lib_in_examples_dir() {
    // A library crate living inside an examples directory (e.g. workspace member `examples/foo_lib`)
    // Must be classified as Lib, NOT Example.
    let compiler_args = vec![
        "rustc".to_string(),
        "examples/foo_lib/src/lib.rs".to_string(),
        "--crate-type".to_string(),
        "lib".to_string(),
    ];
    let kind = classify_target(Some("lib"), &compiler_args);
    assert_eq!(kind, TargetKind::Lib);
}

#[test]
fn test_classification_checkout_in_tests_dir() {
    // A checkout path containing `/tests/` in its directory prefix (e.g. `/tmp/x/tests/my_crate`)
    // Must be classified as Lib, NOT Test.
    let compiler_args = vec![
        "rustc".to_string(),
        "/tmp/x/tests/my_crate/src/lib.rs".to_string(),
        "--crate-type".to_string(),
        "lib".to_string(),
    ];
    let kind = classify_target(Some("lib"), &compiler_args);
    assert_eq!(kind, TargetKind::Lib);
}

#[test]
fn test_classification_test_flag_on_lib() {
    // Compiling a library unit with `--test` for unit testing.
    // Must be classified as Test.
    let compiler_args = vec![
        "rustc".to_string(),
        "src/lib.rs".to_string(),
        "--test".to_string(),
        "--crate-type".to_string(),
        "lib".to_string(),
    ];
    let kind = classify_target(Some("lib"), &compiler_args);
    assert_eq!(kind, TargetKind::Test);
}

#[test]
fn test_classification_bench_with_harness_false() {
    // A benchmark target configured with `harness = false`.
    // Cargo compiles it as `--crate-type bin` WITHOUT `--test`.
    // Authoritative cargo metadata `kind: bench` must classify it as Bench, NOT Bin.
    let compiler_args = vec![
        "rustc".to_string(),
        "benches/criterion_bench.rs".to_string(),
        "--crate-type".to_string(),
        "bin".to_string(),
    ];
    let kind = classify_target(Some("bench"), &compiler_args);
    assert_eq!(kind, TargetKind::Bench);
}

#[test]
fn test_classification_bin_target_with_examples_path() {
    // A binary target configured as `[[bin]] path = "examples/tool.rs"`.
    // Cargo metadata provides `kind: bin`. It must be classified as Bin, NOT Example.
    let compiler_args = vec![
        "rustc".to_string(),
        "examples/tool.rs".to_string(),
        "--crate-type".to_string(),
        "bin".to_string(),
    ];
    let kind = classify_target(Some("bin"), &compiler_args);
    assert_eq!(kind, TargetKind::Bin);
}

#[allow(clippy::type_complexity)]
fn build_mock_workspace_with_kinds(
    readers_info: Vec<(&std::path::Path, Option<PathBuf>, Option<String>, Option<TargetKind>)>,
    ws_root: PathBuf,
) -> WorkspaceIndex {
    let mut readers = Vec::new();
    let mut paths = Vec::new();
    let mut mtimes = Vec::new();
    let mut source_roots = Vec::new();
    let mut unit_keys = Vec::new();
    let mut target_kinds = Vec::new();
    let mut reader_generations = Vec::new();
    let mut reader_dependencies = Vec::new();
    let mut reader_extern_units = Vec::new();
    let mut reader_file_hashes = Vec::new();
    let mut reader_is_dependency = Vec::new();
    let mut meta_paths = Vec::new();
    let mut meta_mtimes = Vec::new();

    for (p, sroot, ukey, tkind) in readers_info {
        let r = crate::reader::IndexReader::open(p).unwrap();
        readers.push(r);
        paths.push(p.to_path_buf());
        mtimes.push(None);
        source_roots.push(sroot);
        unit_keys.push(ukey);
        target_kinds.push(tkind);
        reader_generations.push(None);
        reader_dependencies.push(Vec::new());
        reader_extern_units.push(HashMap::new());
        reader_file_hashes.push(HashMap::new());
        reader_is_dependency.push(false);
        meta_paths.push(None);
        meta_mtimes.push(None);
    }

    WorkspaceIndex {
        readers,
        paths,
        mtimes,
        source_roots,
        unit_keys,
        target_kinds,
        reader_generations,
        reader_dependencies,
        reader_extern_units,
        reader_file_hashes,
        reader_is_dependency,
        meta_paths,
        meta_mtimes,
        last_reloaded_files: std::collections::HashSet::new(),
        root_dir: None,
        workspace_root: Some(ws_root),
        manifest_path: None,
        manifest_mtime: None,
        manifest_len: None,
        manifest_hash: None,
        manifest_generation: None,
        manifest_ino: None,
        active_variant_id: None,
        sysroot_cas_key: None,
    }
}

#[test]
fn test_target_kind_routing_prefers_lib_over_benches_and_examples() {
    let temp_dir = tempfile::tempdir().unwrap();

    let bench_ridx = temp_dir.path().join("bevy_ecs_bench.ridx");
    let mut bench_writer = IndexWriter::new("bevy_ecs", "0.15.0");
    let mut f_bench = File::create(&bench_ridx).unwrap();
    bench_writer.write_to(&mut f_bench).unwrap();
    drop(f_bench);

    let lib_ridx = temp_dir.path().join("bevy_ecs_lib.ridx");
    let mut lib_writer = IndexWriter::new("bevy_ecs", "0.15.0");
    let mut f_lib = File::create(&lib_ridx).unwrap();
    lib_writer.write_to(&mut f_lib).unwrap();
    drop(f_lib);

    let ecs_root = PathBuf::from("/repo/crates/bevy_ecs");
    let ws_root = PathBuf::from("/repo");

    // Bench comes first in reader array; lib comes second
    let ws = build_mock_workspace_with_kinds(
        vec![
            (&bench_ridx, Some(ecs_root.clone()), Some("bevy_ecs 0.15.0 (bench)".into()), Some(TargetKind::Bench)),
            (&lib_ridx, Some(ecs_root.clone()), Some("bevy_ecs 0.15.0 (lib)".into()), Some(TargetKind::Lib)),
        ],
        ws_root,
    );

    let query_file = "/repo/crates/bevy_ecs/src/entity/mod.rs";
    let matched_indices = ws.reader_indices_for_file(query_file);
    assert_eq!(matched_indices, vec![0, 1]);

    // `reader_index_for_file` must choose Lib (index 1), NOT Bench (index 0)
    let best_idx = ws.reader_index_for_file(query_file);
    assert_eq!(best_idx, Some(1));
    assert_eq!(ws.target_kind_at(best_idx.unwrap()), Some(TargetKind::Lib));
}

#[test]
fn test_target_kind_serialization_roundtrip() {
    let kinds = [
        (TargetKind::Lib, "\"lib\""),
        (TargetKind::Bin, "\"bin\""),
        (TargetKind::Test, "\"test\""),
        (TargetKind::Bench, "\"bench\""),
        (TargetKind::Example, "\"example\""),
        (TargetKind::CustomBuild, "\"custom-build\""),
    ];

    for (k, expected_json) in kinds {
        let serialized = serde_json::to_string(&k).unwrap();
        assert_eq!(serialized, expected_json);
        let deserialized: TargetKind = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized, k);
    }
}

#[test]
fn test_loaded_file_metadata_extracts_manifest_dir_and_target_kind() {
    let temp_dir = tempfile::tempdir().unwrap();
    let deps_dir = temp_dir.path().join(".deps");
    std::fs::create_dir_all(&deps_dir).unwrap();

    let sidecar_json = serde_json::json!({
        "name": "my_crate",
        "unit_key": "uk_my_crate",
        "is_dependency": false,
        "env": {
            "CARGO_MANIFEST_DIR": "/repo/crates/my_crate",
            "CARGO_CRATE_NAME": "my_crate",
            "CARGO_PKG_NAME": "my_crate"
        },
        "compiler_args": ["rustc", "crates/my_crate/src/lib.rs", "--crate-type", "lib"]
    });
    std::fs::write(
        deps_dir.join("my_crate.json"),
        serde_json::to_vec(&sidecar_json).unwrap(),
    )
    .unwrap();

    let ridx_file = temp_dir.path().join("my_crate.ridx");
    let mut writer = IndexWriter::new("my_crate", "0.1.0");
    let mut f = File::create(&ridx_file).unwrap();
    writer.write_to(&mut f).unwrap();
    drop(f);

    let empty_hashes = HashMap::new();
    let meta = WorkspaceIndex::load_file_metadata(
        &ridx_file,
        temp_dir.path(),
        "my_crate",
        "uk_my_crate",
        "",
        &empty_hashes,
    );

    assert_eq!(
        meta.source_root,
        Some(PathBuf::from("/repo/crates/my_crate"))
    );
    assert_eq!(meta.target_kind, Some(TargetKind::Lib));
    assert!(!meta.is_dependency);
}

#[test]
fn test_candidate_reader_indices_filters_inaccessible_non_lib_targets() {
    let temp_dir = tempfile::tempdir().unwrap();

    let ecs_lib_ridx = temp_dir.path().join("ecs_lib.ridx");
    let mut ecs_lib_w = IndexWriter::new("bevy_ecs", "0.15.0");
    let mut f = File::create(&ecs_lib_ridx).unwrap();
    ecs_lib_w.write_to(&mut f).unwrap();
    drop(f);

    let ecs_bench_ridx = temp_dir.path().join("ecs_bench.ridx");
    let mut ecs_bench_w = IndexWriter::new("bevy_ecs_bench", "0.15.0");
    let mut f = File::create(&ecs_bench_ridx).unwrap();
    ecs_bench_w.write_to(&mut f).unwrap();
    drop(f);

    let app_lib_ridx = temp_dir.path().join("app_lib.ridx");
    let mut app_lib_w = IndexWriter::new("bevy_app", "0.15.0");
    let mut f = File::create(&app_lib_ridx).unwrap();
    app_lib_w.write_to(&mut f).unwrap();
    drop(f);

    let render_bench_ridx = temp_dir.path().join("render_bench.ridx");
    let mut render_bench_w = IndexWriter::new("render_bench", "0.15.0");
    let mut f = File::create(&render_bench_ridx).unwrap();
    render_bench_w.write_to(&mut f).unwrap();
    drop(f);

    let ws = build_mock_workspace_with_kinds(
        vec![
            (&ecs_lib_ridx, Some(PathBuf::from("/repo/crates/bevy_ecs")), None, Some(TargetKind::Lib)),
            (&ecs_bench_ridx, Some(PathBuf::from("/repo/crates/bevy_ecs")), None, Some(TargetKind::Bench)),
            (&app_lib_ridx, Some(PathBuf::from("/repo/crates/bevy_app")), None, Some(TargetKind::Lib)),
            (&render_bench_ridx, Some(PathBuf::from("/repo/crates/bevy_render")), None, Some(TargetKind::Bench)),
        ],
        PathBuf::from("/repo"),
    );

    // From app_lib (file in /repo/crates/bevy_app/src/lib.rs):
    let candidates = ws.candidate_reader_indices_for_file("/repo/crates/bevy_app/src/lib.rs");
    // Candidates must include app_lib (idx 2) and ecs_lib (idx 0), but NOT benches (idx 1, 3)!
    assert!(candidates.contains(&2), "Must contain bevy_app (own lib)");
    assert!(candidates.contains(&0), "Must contain bevy_ecs (other lib)");
    assert!(!candidates.contains(&1), "Must NOT contain bevy_ecs bench");
    assert!(!candidates.contains(&3), "Must NOT contain bevy_render bench");
}

#[test]
fn test_find_definitions_by_owner_for_file_blocks_benchmark_leakage() {
    let temp_dir = tempfile::tempdir().unwrap();

    let ecs_lib_ridx = temp_dir.path().join("ecs_lib.ridx");
    let mut ecs_lib_w = IndexWriter::new("bevy_ecs", "0.15.0");
    ecs_lib_w.add_definition_with_owners(
        crate::writer::RawDefinition {
            def_id: 1,
            kind: crate::format::DefKind::Struct,
            visibility: 0,
            is_macro_expansion: false,
            name: "World".to_string(),
            path: "bevy_ecs::World".to_string(),
            signature: None,
            file: "src/world.rs".to_string(),
            start_line: 1,
            start_col: 1,
            end_line: 1,
            end_col: 10,
        },
        vec!["bevy_ecs".to_string()],
    );
    let mut f = File::create(&ecs_lib_ridx).unwrap();
    ecs_lib_w.write_to(&mut f).unwrap();
    drop(f);

    let ecs_bench_ridx = temp_dir.path().join("ecs_bench.ridx");
    let mut ecs_bench_w = IndexWriter::new("change_detection", "0.15.0");
    ecs_bench_w.add_definition_with_owners(
        crate::writer::RawDefinition {
            def_id: 2,
            kind: crate::format::DefKind::Struct,
            visibility: 0,
            is_macro_expansion: false,
            name: "ChaCha8Rng".to_string(),
            path: "change_detection::ChaCha8Rng".to_string(),
            signature: None,
            file: "benches/change_detection.rs".to_string(),
            start_line: 1,
            start_col: 1,
            end_line: 1,
            end_col: 10,
        },
        vec!["change_detection".to_string()],
    );
    let mut f = File::create(&ecs_bench_ridx).unwrap();
    ecs_bench_w.write_to(&mut f).unwrap();
    drop(f);

    let ws = build_mock_workspace_with_kinds(
        vec![
            (&ecs_lib_ridx, Some(PathBuf::from("/repo/crates/bevy_ecs")), None, Some(TargetKind::Lib)),
            (&ecs_bench_ridx, Some(PathBuf::from("/repo/crates/bevy_ecs")), None, Some(TargetKind::Bench)),
        ],
        PathBuf::from("/repo"),
    );

    // Query from lib file (/repo/crates/bevy_ecs/src/world.rs):
    let defs = ws.find_definitions_by_owner_for_file(
        "/repo/crates/bevy_ecs/src/world.rs",
        "change_detection",
    );
    assert!(defs.is_empty(), "Benchmark symbols must NOT leak to library file queries");

    // Query from inside the benchmark file (/repo/crates/bevy_ecs/benches/change_detection.rs):
    let bench_defs = ws.find_definitions_by_owner_for_file(
        "/repo/crates/bevy_ecs/benches/change_detection.rs",
        "change_detection",
    );
    assert_eq!(bench_defs.len(), 1, "Benchmark file must see its own definitions");
    assert_eq!(bench_defs[0].name, "ChaCha8Rng");
}

