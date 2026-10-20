/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! `textDocument/codeLens` on closed files must come from the readers that own the file, not from
//! every shard whose file table has an entry with the same path suffix (`src/lib.rs`).

use super::handle_code_lens;
use crate::lsp::protocol::{path_to_uri, PositionConverter};
use crate::overlay::OverlayManager;
use ridx_format::{DefKind, IndexWriter, RawDefinition, WorkspaceIndex};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static LENS_TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Two workspace crates and one dependency, each with a `src/lib.rs`, indexed into one variant
/// directory with per-shard sidecars that carry the source roots, like `ridx index` writes them.
struct Fixture {
    root: PathBuf,
    registry: PathBuf,
    ws: WorkspaceIndex,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
        let _ = fs::remove_dir_all(&self.registry);
    }
}

fn def(
    def_id: u32,
    name: &str,
    krate: &str,
    line: u32,
    start_col: u32,
    end_col: u32,
) -> RawDefinition {
    RawDefinition {
        def_id,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: name.to_string(),
        path: format!("{krate}::{name}"),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: line,
        start_col,
        end_line: line,
        end_col,
    }
}

fn write_shard(
    var_dir: &Path,
    stem: &str,
    krate: &str,
    source_root: &Path,
    is_dependency: bool,
    defs: Vec<RawDefinition>,
) {
    let mut writer = IndexWriter::new(krate, "0.1.0");
    writer.set_unit_key(&format!("{krate} 0.1.0 (lib)"));
    for d in defs {
        writer.add_definition(d);
    }
    writer
        .write_to(&mut fs::File::create(var_dir.join(format!("{stem}.ridx"))).unwrap())
        .unwrap();
    let sidecar = json!({
        "name": krate,
        "version": "0.1.0",
        "unit_key": format!("{krate} 0.1.0 (lib)"),
        "source_root": source_root.display().to_string(),
        "target_kind": "lib",
        "is_dependency": is_dependency,
    });
    fs::write(
        var_dir.join(".deps").join(format!("{stem}.json")),
        serde_json::to_string(&sidecar).unwrap(),
    )
    .unwrap();
}

fn fixture() -> Fixture {
    let id = LENS_TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
    let tag = format!("{}_{}", std::process::id(), id);
    let root = std::env::temp_dir().join(format!("ridx_lens_ws_{tag}"));
    let registry = std::env::temp_dir().join(format!("ridx_lens_registry_{tag}"));
    let var_dir = root.join("variants/var1");
    fs::create_dir_all(var_dir.join(".deps")).unwrap();

    let crate_a = root.join("crates/crate_a");
    let crate_b = root.join("crates/crate_b");
    let dep = registry.join("dep_crate-1.0.0");
    for (dir, text) in [
        (&crate_a, "pub fn alpha() {}\n\npub fn beta() {}\n"),
        (&crate_b, "pub fn gamma() {}\n"),
        (&dep, "pub fn dep_item() {}\n"),
    ] {
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(dir.join("src/lib.rs"), text).unwrap();
    }
    fs::create_dir_all(crate_a.join("examples")).unwrap();
    fs::write(crate_a.join("examples/demo.rs"), "fn main() {}\n").unwrap();

    write_shard(
        &var_dir,
        "crate_a",
        "crate_a",
        &crate_a,
        false,
        vec![
            def(1, "alpha", "crate_a", 1, 8, 13),
            def(2, "beta", "crate_a", 3, 8, 12),
        ],
    );
    write_shard(
        &var_dir,
        "crate_b",
        "crate_b",
        &crate_b,
        false,
        vec![def(1, "gamma", "crate_b", 1, 8, 13)],
    );
    write_shard(
        &var_dir,
        "dep_crate",
        "dep_crate",
        &dep,
        true,
        vec![def(1, "dep_item", "dep_crate", 1, 8, 16)],
    );

    let manifest = json!({
        "version": "2.0",
        "format_version": ridx_format::CURRENT_FORMAT_VERSION,
        "extractor_version": ridx_format::EXTRACTOR_VERSION,
        "active_variant_id": "var1",
        "variant_dir": "variants/var1",
        "workspace_root": root.display().to_string(),
        "manifest_path": root.join("Cargo.toml").display().to_string(),
        "profile": "dev"
    });
    fs::write(
        root.join("ridx_manifest.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let ws = WorkspaceIndex::open(&root).expect("open the fixture workspace");
    assert_eq!(ws.readers().len(), 3, "three shards must be mapped");
    Fixture { root, registry, ws }
}

fn lens(uri: &str, line: u32, start_col: u32, end_col: u32, name: &str) -> Value {
    json!({
        "range": {
            "start": { "line": line - 1, "character": start_col - 1 },
            "end": { "line": line - 1, "character": end_col - 1 }
        },
        "data": { "uri": uri, "line": line, "character": start_col, "name": name }
    })
}

fn code_lens(fx: &Fixture, file: &Path) -> (String, Value) {
    let uri = path_to_uri(file.to_str().unwrap());
    let msg = json!({ "params": { "textDocument": { "uri": &uri } } });
    let mut conv = PositionConverter::new();
    let result = handle_code_lens(&msg, Some(&fx.ws), &OverlayManager::new(), &mut conv);
    (uri, result)
}

#[test]
fn test_code_lens_closed_file_lists_only_the_owning_crates_definitions() {
    let fx = fixture();
    let (uri, result) = code_lens(&fx, &fx.root.join("crates/crate_a/src/lib.rs"));
    // `crate_b` and `dep_crate` also index a `src/lib.rs`; none of their definitions may appear.
    assert_eq!(
        result,
        json!([lens(&uri, 1, 8, 13, "alpha"), lens(&uri, 3, 8, 12, "beta")])
    );
}

#[test]
fn test_code_lens_dependency_file_outside_the_workspace_keeps_its_lenses() {
    let fx = fixture();
    let (uri, result) = code_lens(&fx, &fx.registry.join("dep_crate-1.0.0/src/lib.rs"));
    assert_eq!(result, json!([lens(&uri, 1, 8, 16, "dep_item")]));
}

#[test]
fn test_code_lens_file_no_reader_indexes_is_empty() {
    let fx = fixture();
    // Under crate_a's source root but absent from its file table: the owning reader has no
    // definitions for it, so the answer is empty rather than borrowed from another file.
    let (_, inside) = code_lens(&fx, &fx.root.join("crates/crate_a/examples/demo.rs"));
    assert_eq!(inside, json!([]));
    // Under the workspace root but under no crate's source root.
    let (_, outside) = code_lens(&fx, &fx.root.join("tools/src/lib.rs"));
    assert_eq!(outside, json!([]));
}
