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
use crate::reader::IndexReader;
use crate::writer::IndexWriter;
use crate::{DefKind, RawDefinition};
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::path::PathBuf;

#[allow(clippy::type_complexity)]
pub(crate) fn build_mock_workspace(readers_info: Vec<(&PathBuf, Option<PathBuf>, Option<String>, HashMap<String, u64>)>, workspace_root: PathBuf) -> WorkspaceIndex {
    let mut readers = Vec::with_capacity(readers_info.len());
    let mut paths = Vec::with_capacity(readers_info.len());
    let mut mtimes = Vec::with_capacity(readers_info.len());
    let mut source_roots = Vec::with_capacity(readers_info.len());
    let mut unit_keys = Vec::with_capacity(readers_info.len());
    let mut reader_generations = Vec::with_capacity(readers_info.len());
    let mut reader_dependencies = Vec::with_capacity(readers_info.len());
    let mut reader_extern_units = Vec::with_capacity(readers_info.len());
    let mut reader_file_hashes = Vec::with_capacity(readers_info.len());
    let mut reader_is_dependency = Vec::with_capacity(readers_info.len());
    let mut meta_paths = Vec::with_capacity(readers_info.len());
    let mut meta_mtimes = Vec::with_capacity(readers_info.len());

    for (p, sroot, ukey, fhashes) in readers_info {
        readers.push(IndexReader::open(p).expect("failed to open index"));
        paths.push(p.clone());
        mtimes.push(None);
        source_roots.push(sroot);
        unit_keys.push(ukey);
        reader_generations.push(None);
        reader_dependencies.push(Vec::new());
        reader_extern_units.push(HashMap::new());
        reader_file_hashes.push(fhashes);
        reader_is_dependency.push(false);
        meta_paths.push(None);
        meta_mtimes.push(None);
    }

    let count = readers.len();
    let unit_match = UnitMatchIndex::build(&readers, &unit_keys, &reader_extern_units);
    WorkspaceIndex {
        readers,
        paths,
        mtimes,
        source_roots,
        unit_keys,
        unit_match,
        path_index: super::PathIndex::default(),
        target_kinds: vec![None; count],
        reader_generations,
        reader_dependencies,
        reader_extern_units,
        reader_file_hashes,
        own_sources: Default::default(),
        reader_is_dependency,
        meta_paths,
        meta_mtimes,
        last_reloaded_files: HashSet::new(),
        root_dir: None,
        workspace_root: Some(workspace_root),
        manifest_path: None,
        manifest_mtime: None,
        manifest_len: None,
        manifest_hash: None,
        manifest_generation: None,
        manifest_ino: None,
        active_variant_id: None,
        sysroot_cas_key: None,
        instance_id: 1,
        generation: 1,
    }
}

#[test]
fn test_longest_prefix_matching_order_independence() {
    let temp_dir = tempfile::tempdir().unwrap();

    let root_ridx = temp_dir.path().join("root_pkg.ridx");
    let mut root_writer = IndexWriter::new("root_pkg", "0.1.0");
    let mut f_root = File::create(&root_ridx).unwrap();
    root_writer.write_to(&mut f_root).unwrap();
    drop(f_root);

    let x_lib_ridx = temp_dir.path().join("x_lib.ridx");
    let mut x_lib_writer = IndexWriter::new("x", "0.1.0");
    let mut f_x_lib = File::create(&x_lib_ridx).unwrap();
    x_lib_writer.write_to(&mut f_x_lib).unwrap();
    drop(f_x_lib);

    let x_test_ridx = temp_dir.path().join("x_test.ridx");
    let mut x_test_writer = IndexWriter::new("x", "0.1.0");
    let mut f_x_test = File::create(&x_test_ridx).unwrap();
    x_test_writer.write_to(&mut f_x_test).unwrap();
    drop(f_x_test);

    let root_src = PathBuf::from("/repo");
    let x_src = PathBuf::from("/repo/crates/x");

    let ws_a = build_mock_workspace(
        vec![
            (&root_ridx, Some(root_src.clone()), Some("root_pkg 0.1.0 (lib)".into()), HashMap::new()),
            (&x_lib_ridx, Some(x_src.clone()), Some("x 0.1.0 (lib)".into()), HashMap::new()),
            (&x_test_ridx, Some(x_src.clone()), Some("x 0.1.0 (test)".into()), HashMap::new()),
        ],
        root_src.clone(),
    );

    let ws_b = build_mock_workspace(
        vec![
            (&x_lib_ridx, Some(x_src.clone()), Some("x 0.1.0 (lib)".into()), HashMap::new()),
            (&x_test_ridx, Some(x_src.clone()), Some("x 0.1.0 (test)".into()), HashMap::new()),
            (&root_ridx, Some(root_src.clone()), Some("root_pkg 0.1.0 (lib)".into()), HashMap::new()),
        ],
        root_src.clone(),
    );

    let file_in_x = "/repo/crates/x/src/lib.rs";
    let file_in_root = "/repo/src/lib.rs";

    assert_eq!(ws_a.reader_indices_for_file(file_in_x), vec![1, 2]);
    assert_eq!(ws_a.reader_index_for_file(file_in_x), Some(1));
    assert_eq!(ws_a.crate_name_for_file(file_in_x), Some("x"));

    assert_eq!(ws_b.reader_indices_for_file(file_in_x), vec![0, 1]);
    assert_eq!(ws_b.reader_index_for_file(file_in_x), Some(0));
    assert_eq!(ws_b.crate_name_for_file(file_in_x), Some("x"));

    assert_eq!(ws_a.reader_indices_for_file(file_in_root), vec![0]);
    assert_eq!(ws_a.reader_index_for_file(file_in_root), Some(0));
    assert_eq!(ws_a.crate_name_for_file(file_in_root), Some("root_pkg"));

    assert_eq!(ws_b.reader_indices_for_file(file_in_root), vec![2]);
    assert_eq!(ws_b.reader_index_for_file(file_in_root), Some(2));
    assert_eq!(ws_b.crate_name_for_file(file_in_root), Some("root_pkg"));
}

#[test]
fn test_facade_subcrate_and_benchmark_routing() {
    let temp_dir = tempfile::tempdir().unwrap();

    // 1. Facade crate: `bevy` at workspace root `/repo`
    let bevy_ridx = temp_dir.path().join("bevy.ridx");
    let mut bevy_writer = IndexWriter::new("bevy", "0.15.0");
    bevy_writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Struct,
        visibility: 0,
        is_macro_expansion: false,
        name: "App".to_string(),
        path: "bevy::app::App".to_string(),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 5,
        end_col: 2,
    });
    let mut f_bevy = File::create(&bevy_ridx).unwrap();
    bevy_writer.write_to(&mut f_bevy).unwrap();
    drop(f_bevy);

    // 2. Subcrate: `bevy_ecs` at `/repo/crates/bevy_ecs`
    let ecs_ridx = temp_dir.path().join("bevy_ecs.ridx");
    let mut ecs_writer = IndexWriter::new("bevy_ecs", "0.15.0");
    ecs_writer.add_definition(RawDefinition {
        def_id: 10,
        kind: DefKind::Struct,
        visibility: 0,
        is_macro_expansion: false,
        name: "EntityIndexSet".to_string(),
        path: "bevy_ecs::entity::index_set::EntityIndexSet".to_string(),
        signature: None,
        file: "src/entity/index_set.rs".to_string(),
        start_line: 10,
        start_col: 1,
        end_line: 20,
        end_col: 2,
    });
    let mut f_ecs = File::create(&ecs_ridx).unwrap();
    ecs_writer.write_to(&mut f_ecs).unwrap();
    drop(f_ecs);

    // 3. Benchmark crate: `bevy_ecs_bench` at `/repo/benches/benches/bevy_ecs`
    let bench_ridx = temp_dir.path().join("bevy_ecs_bench.ridx");
    let mut bench_writer = IndexWriter::new("bevy_ecs_bench", "0.15.0");
    bench_writer.add_definition(RawDefinition {
        def_id: 20,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "bench_entity_set".to_string(),
        path: "bevy_ecs_bench::bench_entity_set".to_string(),
        signature: None,
        file: "src/main.rs".to_string(),
        start_line: 5,
        start_col: 1,
        end_line: 15,
        end_col: 2,
    });
    let mut f_bench = File::create(&bench_ridx).unwrap();
    bench_writer.write_to(&mut f_bench).unwrap();
    drop(f_bench);

    let ws_root = PathBuf::from("/repo");
    let root_src = PathBuf::from("/repo");
    let ecs_src = PathBuf::from("/repo/crates/bevy_ecs");
    let bench_src = PathBuf::from("/repo/benches/benches/bevy_ecs");

    let ecs_file = "/repo/crates/bevy_ecs/src/entity/index_set.rs";
    let facade_file = "/repo/src/lib.rs";
    let bench_file = "/repo/benches/benches/bevy_ecs/src/main.rs";

    // Scenario A: Order [bevy_ecs_bench, bevy, bevy_ecs]
    // Benchmark crate is placed FIRST in reader array to expose ordering bias.
    let ws_a = build_mock_workspace(
        vec![
            (&bench_ridx, Some(bench_src.clone()), Some("bench_ukey".into()), HashMap::new()),
            (&bevy_ridx, Some(root_src.clone()), Some("bevy_ukey".into()), HashMap::new()),
            (&ecs_ridx, Some(ecs_src.clone()), Some("ecs_ukey".into()), HashMap::new()),
        ],
        ws_root.clone(),
    );

    // Scenario B: Order [bevy, bevy_ecs, bevy_ecs_bench]
    let ws_b = build_mock_workspace(
        vec![
            (&bevy_ridx, Some(root_src.clone()), Some("bevy_ukey".into()), HashMap::new()),
            (&ecs_ridx, Some(ecs_src.clone()), Some("ecs_ukey".into()), HashMap::new()),
            (&bench_ridx, Some(bench_src.clone()), Some("bench_ukey".into()), HashMap::new()),
        ],
        ws_root.clone(),
    );

    // 1. Verify that `crates/bevy_ecs/src/entity/index_set.rs` routes to `bevy_ecs`
    // Longest prefix match on `source_roots` MUST select `bevy_ecs` (3 components),
    // NOT the broader facade `bevy` (1 component), and NOT `bevy_ecs_bench`.
    assert_eq!(ws_a.crate_name_for_file(ecs_file), Some("bevy_ecs"));
    assert_eq!(ws_b.crate_name_for_file(ecs_file), Some("bevy_ecs"));
    assert_eq!(ws_a.reader_index_for_file(ecs_file), Some(2));
    assert_eq!(ws_b.reader_index_for_file(ecs_file), Some(1));

    // 2. Verify that `benches/benches/bevy_ecs/src/main.rs` routes to `bevy_ecs_bench`
    // Substring `/bevy_ecs/` in the path must NOT cause this file to route to `bevy_ecs`.
    assert_eq!(ws_a.crate_name_for_file(bench_file), Some("bevy_ecs_bench"));
    assert_eq!(ws_b.crate_name_for_file(bench_file), Some("bevy_ecs_bench"));
    assert_eq!(ws_a.reader_index_for_file(bench_file), Some(0));
    assert_eq!(ws_b.reader_index_for_file(bench_file), Some(2));

    // 3. Verify that facade `/repo/src/lib.rs` routes to `bevy`
    assert_eq!(ws_a.crate_name_for_file(facade_file), Some("bevy"));
    assert_eq!(ws_b.crate_name_for_file(facade_file), Some("bevy"));
    assert_eq!(ws_a.reader_index_for_file(facade_file), Some(1));
    assert_eq!(ws_b.reader_index_for_file(facade_file), Some(0));

    // 4. Verify relative path lookup behavior
    let rel_ecs_file = "crates/bevy_ecs/src/entity/index_set.rs";
    assert_eq!(ws_a.crate_name_for_file(rel_ecs_file), Some("bevy_ecs"));
    assert_eq!(ws_b.crate_name_for_file(rel_ecs_file), Some("bevy_ecs"));
}

#[test]
fn test_candidate_reader_indices_prioritizes_sysroot_before_external_deps() {
    let temp_dir = tempfile::tempdir().unwrap();

    let my_crate_ridx = temp_dir.path().join("my_crate.ridx");
    let mut w_my = IndexWriter::new("my_crate", "0.1.0");
    let mut f = File::create(&my_crate_ridx).unwrap();
    w_my.write_to(&mut f).unwrap();
    drop(f);

    let dep_ridx = temp_dir.path().join("dep_crate.ridx");
    let mut w_dep = IndexWriter::new("dep_crate", "1.0.0");
    let mut f = File::create(&dep_ridx).unwrap();
    w_dep.write_to(&mut f).unwrap();
    drop(f);

    let alloc_ridx = temp_dir.path().join("alloc.ridx");
    let mut w_alloc = IndexWriter::new("alloc", "1.98.0");
    let mut f = File::create(&alloc_ridx).unwrap();
    w_alloc.write_to(&mut f).unwrap();
    drop(f);

    let my_src = PathBuf::from("/repo/src");
    let ws_root = PathBuf::from("/repo");

    // Index 0: my_crate (workspace crate containing queried file)
    // Index 1: dep_crate (external dependency, reader_is_dependency = true)
    // Index 2: alloc (sysroot crate, reader_is_dependency = true, crate_name = "alloc")
    let mut ws = build_mock_workspace(
        vec![
            (&my_crate_ridx, Some(my_src), Some("my_crate_ukey".into()), HashMap::new()),
            (&dep_ridx, None, Some("dep_crate_ukey".into()), HashMap::new()),
            (&alloc_ridx, None, Some("alloc_ukey".into()), HashMap::new()),
        ],
        ws_root,
    );
    ws.reader_is_dependency[1] = true;
    ws.reader_is_dependency[2] = true;

    let query_file = "/repo/src/lib.rs";
    let candidates = ws.candidate_reader_indices_for_file(query_file);

    // Order MUST be:
    // [0]: my_crate (target file crate)
    // [2]: alloc (sysroot shard)
    // [1]: dep_crate (external dependency shard)
    assert_eq!(candidates, vec![0, 2, 1]);
}
