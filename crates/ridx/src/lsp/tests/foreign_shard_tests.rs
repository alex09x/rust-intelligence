/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! A workspace crate and a dependency whose stored paths end alike: `app` stores
//! `crates/app/src/lib.rs` under the root `<ws>/crates/app`, `dep` stores `src/lib.rs` and
//! `src/util.rs` under a root outside the workspace. The readers of a workspace file are the
//! ones that store it; the dependency's files must never answer for `crates/app/src/lib.rs` or
//! for `crates/app/src/util.rs`, which lies under the crate's root but in no shard.

use crate::lsp::hierarchy::scan::find_enclosing_function;
use crate::lsp::intelligence::scope_cache::collect_file_referenced_traits;
use crate::lsp::presentation::collect_raw_tokens;
use crate::lsp::protocol::PositionConverter;
use crate::overlay::OverlayManager;
use ridx_format::{DefKind, IndexWriter, RawDefinition, RawReference, WorkspaceIndex};
use std::fs;
use std::path::Path;

const APP_LIB: &str = "pub fn app_fn() {\n    1\n}\npub struct Local;\nuse other::LocalTrait;\n";
const APP_UTIL: &str = "fn local() {\n    2\n}\n";

struct Fixture {
    _dir: tempfile::TempDir,
    ws: WorkspaceIndex,
    lib: String,
    util: String,
}

fn def(
    id: u32,
    kind: DefKind,
    name: &str,
    path: &str,
    file: &str,
    (line, start, end): (u32, u32, u32),
) -> RawDefinition {
    RawDefinition {
        def_id: id,
        kind,
        visibility: 1,
        is_macro_expansion: false,
        name: name.to_string(),
        path: path.to_string(),
        signature: None,
        file: file.to_string(),
        start_line: line,
        start_col: start,
        end_line: line,
        end_col: end,
    }
}

fn trait_ref(file: &str, line: u32, path: &str) -> RawReference {
    RawReference {
        file: file.to_string(),
        start_line: line,
        start_col: 12,
        end_line: line,
        end_col: 22,
        target_def_id: None,
        target_path: path.to_string(),
        target_file: None,
        target_unit: None,
        target_line: 0,
        target_col: 0,
        kind: DefKind::Trait,
        is_write: false,
        is_macro_expansion: false,
    }
}

fn write_shard(
    index: &Path,
    name: &str,
    writer: &mut IndexWriter,
    source_root: &str,
    is_dependency: bool,
) {
    let mut f = fs::File::create(index.join(format!("{name}.ridx"))).unwrap();
    writer.write_to(&mut f).unwrap();
    let sidecar = format!(
        "{{\"name\": \"{name}\", \"version\": \"0.1.0\", \"unit_key\": \"\", \"source_root\": \"{source_root}\", \"is_dependency\": {is_dependency}}}"
    );
    fs::write(index.join(".deps").join(format!("{name}.json")), sidecar).unwrap();
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("crates/app/src")).unwrap();
    fs::write(root.join("crates/app/src/lib.rs"), APP_LIB).unwrap();
    fs::write(root.join("crates/app/src/util.rs"), APP_UTIL).unwrap();
    let index = root.join("target/ridx");
    fs::create_dir_all(index.join(".deps")).unwrap();

    let mut app = IndexWriter::new("app", "0.1.0");
    app.add_definition(def(
        1,
        DefKind::Fn,
        "app_fn",
        "app::app_fn",
        "crates/app/src/lib.rs",
        (1, 8, 14),
    ));
    app.add_definition(def(
        2,
        DefKind::Struct,
        "Local",
        "app::Local",
        "crates/app/src/lib.rs",
        (4, 12, 17),
    ));
    app.add_reference(trait_ref("crates/app/src/lib.rs", 5, "other::LocalTrait"));
    write_shard(
        &index,
        "app",
        &mut app,
        root.join("crates/app").to_str().unwrap(),
        false,
    );

    let mut dep = IndexWriter::new("dep", "1.0.0");
    dep.add_definition(def(
        1,
        DefKind::Fn,
        "dep_fn",
        "dep::dep_fn",
        "src/lib.rs",
        (9, 8, 14),
    ));
    dep.add_definition(def(
        2,
        DefKind::Fn,
        "dep_helper",
        "dep::dep_helper",
        "src/util.rs",
        (1, 4, 14),
    ));
    dep.add_reference(trait_ref("src/util.rs", 2, "dep::ForeignTrait"));
    write_shard(&index, "dep", &mut dep, "/deps/dep-1.0.0", true);

    let ws = WorkspaceIndex::open(root).expect("open the workspace");
    assert_eq!(ws.readers().len(), 2, "both shards are loaded");
    Fixture {
        lib: root
            .join("crates/app/src/lib.rs")
            .to_str()
            .unwrap()
            .to_string(),
        util: root
            .join("crates/app/src/util.rs")
            .to_str()
            .unwrap()
            .to_string(),
        _dir: dir,
        ws,
    }
}

#[test]
fn test_semantic_tokens_come_only_from_the_shards_that_store_the_file() {
    let fx = fixture();
    let overlays = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let tokens = collect_raw_tokens(&fx.lib, Some(&fx.ws), &overlays, &mut conv);
    let lines: Vec<u32> = tokens.iter().map(|t| t.line).collect();
    assert!(
        lines.iter().all(|&l| l < 5),
        "the file has five lines; a token at a later line came from the dependency's src/lib.rs: {lines:?}"
    );
    assert_eq!(
        lines,
        vec![0, 3, 4],
        "app_fn, Local and the trait reference, in file order"
    );
}

#[test]
fn test_enclosing_function_is_not_taken_from_a_dependency_file_with_the_same_tail() {
    let fx = fixture();
    let text = fs::read_to_string(&fx.util).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        find_enclosing_function(&fx.ws, &fx.util, 2, &lines).map(|(symbol, _)| symbol.path),
        None,
        "crates/app/src/util.rs is in no shard; dep::dep_helper lives in the dependency's src/util.rs"
    );
    let lib_text = fs::read_to_string(&fx.lib).unwrap();
    let lib_lines: Vec<&str> = lib_text.lines().collect();
    assert_eq!(
        find_enclosing_function(&fx.ws, &fx.lib, 2, &lib_lines).map(|(symbol, _)| symbol.path),
        Some("app::app_fn"),
        "the workspace file's own function still encloses line 2"
    );
}

#[test]
fn test_referenced_traits_are_not_collected_from_a_dependency_file_with_the_same_tail() {
    let fx = fixture();
    let util_traits = collect_file_referenced_traits(&fx.ws, &fx.util);
    assert!(
        !util_traits.contains("ForeignTrait"),
        "the dependency's src/util.rs references ForeignTrait, the workspace util.rs does not: {util_traits:?}"
    );
    assert!(
        util_traits.is_empty(),
        "no shard stores util.rs, so nothing is referenced from it: {util_traits:?}"
    );
    let lib_traits = collect_file_referenced_traits(&fx.ws, &fx.lib);
    assert!(
        lib_traits.contains("LocalTrait"),
        "the workspace file's own trait reference is kept: {lib_traits:?}"
    );
    assert!(!lib_traits.contains("ForeignTrait"));
}
