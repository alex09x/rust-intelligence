/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Requests on a root whose index failed (prod-code, m=19837; review of b3d0d293).

use super::*;
use crate::lsp::reindex::Reindexer;
use crate::lsp::MultiRootWorkspace;
use std::collections::HashSet;

/// A Cargo package at `root` with `src/lib.rs`.
fn package(root: &Path) {
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"w\"\nversion = \"0.1.0\"\n").unwrap();
    std::fs::write(root.join("src/lib.rs"), "pub fn a() {}\n").unwrap();
}

/// A published index of `root` holding `src/lib.rs`, as `ridx index` leaves it.
fn publish_index(root: &Path) {
    let variant = root.join("target/ridx/variants/dev");
    std::fs::create_dir_all(variant.join(".deps")).unwrap();
    let mut w = ridx_format::IndexWriter::new("w", "0.1.0");
    w.get_or_insert_file("src/lib.rs");
    w.write_to(&mut std::fs::File::create(variant.join("w.ridx")).unwrap()).unwrap();
    let meta = json!({ "name": "w", "version": "0.1.0", "source_root": root.display().to_string(), "target_kind": "lib", "is_dependency": false });
    std::fs::write(variant.join(".deps/w.json"), meta.to_string()).unwrap();
    let manifest = json!({ "version": "2.0", "format_version": ridx_format::CURRENT_FORMAT_VERSION, "extractor_version": ridx_format::EXTRACTOR_VERSION,
        "active_variant_id": "dev", "variant_dir": "variants/dev", "workspace_root": root.display().to_string() });
    std::fs::write(root.join("target/ridx/ridx_manifest.json"), manifest.to_string()).unwrap();
}

fn references(root: &Path, id: u64, min_version: Option<i64>) -> Value {
    let mut params = json!({ "textDocument": { "uri": format!("file://{}", root.join("src/lib.rs").display()) }, "position": { "line": 0, "character": 7 }, "context": { "includeDeclaration": true } });
    if let Some(v) = min_version {
        params["minVersion"] = json!(v);
    }
    json!({ "jsonrpc": "2.0", "id": id, "method": "textDocument/references", "params": params })
}

fn answers(out: &[u8]) -> Vec<Value> {
    let text = String::from_utf8_lossy(out);
    text.split("Content-Length:").filter_map(|m| m.find('{').map(|i| serde_json::from_str(m[i..].trim()).unwrap())).collect()
}

/// A loop at the point a request arrives: initialized on the folder `root`.
struct Loop {
    gate: Gate,
    lifecycle: crate::lsp::LifecycleManager,
    mr: MultiRootWorkspace,
    ov: crate::overlay::OverlayManager,
    session: SessionIndexer,
    cancelled: Mutex<HashSet<Value>>,
}

impl Loop {
    fn new(root: &Path) -> Self {
        let mut lifecycle = crate::lsp::LifecycleManager::new();
        let init = json!({ "id": 1, "method": "initialize", "params": { "capabilities": {}, "workspaceFolders": [{ "uri": format!("file://{}", root.display()), "name": "w" }] } });
        lifecycle.handle_initialize(&init).unwrap();
        let mut gate = Gate::default();
        gate.initialized(&init, &lifecycle.reindex);
        Self { gate, lifecycle, mr: MultiRootWorkspace::new(), ov: crate::overlay::OverlayManager::new(), session: SessionIndexer::default(), cancelled: Mutex::default() }
    }

    fn intercept(&mut self, msg: Value, out: &mut Vec<u8>) -> Option<Value> {
        intercept(msg, &mut self.gate, out, &mut self.lifecycle, &mut self.mr, &mut self.ov, &self.session, &self.cancelled).unwrap()
    }
}

/// The index published by hand after the run failed answers: it is loaded before the failure is judged.
#[test]
fn test_an_index_published_after_the_failure_answers() {
    let dir = tempfile::tempdir().unwrap();
    let root = ridx_format::canonicalize_best_effort(dir.path());
    package(&root);
    let mut l = Loop::new(&root);
    l.lifecycle.reindex.fail_for_test(&root, "ridx index failed: no driver");
    let mut out = Vec::new();
    assert!(l.intercept(references(&root, 2, None), &mut out).is_none());
    assert_eq!(answers(&out)[0]["error"]["data"]["reason"], "indexFailed", "no index yet: refused");

    publish_index(&root);
    let mut out = Vec::new();
    let passed = l.intercept(references(&root, 3, Some(1)), &mut out);
    assert!(answers(&out).iter().all(|a| a["id"] != 3), "not refused: {:?}", answers(&out));
    assert!(passed.is_some(), "the file is not open: no version to wait for, answered from the index loaded");
    assert!(l.mr.roots().any(|r| r == root), "the published index was loaded");
}

/// A nested workspace whose own index failed is not hidden by the outer one loaded.
#[test]
fn test_a_loaded_outer_workspace_does_not_hide_a_failed_nested_one() {
    let dir = tempfile::tempdir().unwrap();
    let outer = ridx_format::canonicalize_best_effort(dir.path());
    package(&outer);
    publish_index(&outer);
    let nested = outer.join("nested");
    package(&nested);
    let mut mr = MultiRootWorkspace::new();
    assert!(mr.add_root(outer.clone()));
    let reindex = Reindexer::new();
    reindex.fail_for_test(&nested, "ridx index failed: no driver");
    let (root, _) = index_failed(&references(&nested, 1, None), &mr, &reindex).expect("the nested root's failure");
    assert_eq!(root, nested);
    assert!(index_failed(&references(&outer, 2, None), &mr, &reindex).is_none(), "the outer one answers");
}

/// A lone `.ridx` loaded for a root records no workspace root; it is still the root's index.
#[test]
fn test_a_lone_shard_loaded_counts_as_an_index() {
    let dir = tempfile::tempdir().unwrap();
    let root = ridx_format::canonicalize_best_effort(dir.path());
    package(&root);
    let mut w = ridx_format::IndexWriter::new("w", "0.1.0");
    w.get_or_insert_file("src/lib.rs");
    w.write_to(&mut std::fs::File::create(root.join("manual.ridx")).unwrap()).unwrap();
    let mut mr = MultiRootWorkspace::new();
    assert!(mr.add_root(root.join("manual.ridx")));
    assert!(mr.first_workspace().is_some_and(|ws| ws.workspace_root().is_none()));
    let reindex = Reindexer::new();
    reindex.fail_for_test(&root, "ridx index failed: no driver");
    assert!(index_failed(&references(&root, 1, None), &mr, &reindex).is_none(), "the loaded shard answers");
}

/// A request held for its version when the run failed gets the failure at the next turn, not a timeout.
#[test]
fn test_a_held_request_gets_the_failure() {
    let dir = tempfile::tempdir().unwrap();
    let root = ridx_format::canonicalize_best_effort(dir.path());
    package(&root);
    let mut l = Loop::new(&root);
    let file = root.join("src/lib.rs").display().to_string();
    l.ov.open_document(&format!("file://{file}"), &file, 1, "pub fn a() {}\n".to_string());
    let mut out = Vec::new();
    assert!(l.intercept(references(&root, 2, Some(1)), &mut out).is_none(), "held: no index covers version 1");
    assert!(!l.gate.waiting.is_empty());

    l.lifecycle.reindex.fail_for_test(&root, "ridx index failed: no driver");
    let mut out = Vec::new();
    let ready = release_waiting(&mut l.gate, &mut out, &mut l.lifecycle, &mut l.mr, &mut l.ov, &l.cancelled).unwrap();
    assert!(ready.is_empty());
    let a = answers(&out);
    let answer = a.iter().find(|a| a["id"] == 2).unwrap_or_else(|| panic!("answered: {a:?}"));
    assert_eq!(answer["error"]["data"]["reason"], "indexFailed");
    assert!(l.gate.waiting.is_empty(), "taken out of the waiting requests");
}

/// Review of 11a609ef: an index opened as its directory (`--index-path W/target/ridx`) is bound to that directory and
/// records `W` as its workspace root; it answers for `W` after a later run failed.
#[test]
fn test_an_index_opened_as_its_directory_answers_for_its_workspace() {
    let dir = tempfile::tempdir().unwrap();
    let root = ridx_format::canonicalize_best_effort(dir.path());
    package(&root);
    publish_index(&root);
    let mut mr = MultiRootWorkspace::new();
    assert!(mr.add_root(root.join("target/ridx")));
    assert_eq!(mr.first_workspace().and_then(|ws| ws.workspace_root()).map(ridx_format::canonicalize_best_effort), Some(root.clone()));
    let reindex = Reindexer::new();
    reindex.fail_for_test(&root, "ridx index failed: a newer build failed");
    assert!(index_failed(&references(&root, 1, None), &mr, &reindex).is_none(), "the index loaded answers");
}

/// Review of 3e51faf9: a nested workspace's index opened as its directory, then the enclosing one: the guard let a
/// request on the nested workspace through, and the enclosing index answered it. Both now go by the root the directory
/// is bound to, the nested workspace.
#[test]
fn test_the_index_admitted_is_the_index_answering() {
    let dir = tempfile::tempdir().unwrap();
    let outer = ridx_format::canonicalize_best_effort(dir.path());
    let nested = outer.join("nested");
    for root in [&outer, &nested] {
        package(root);
        publish_index(root);
    }
    let mut mr = MultiRootWorkspace::new();
    assert!(mr.add_root(nested.join("target/ridx")));
    assert!(mr.add_root(outer.clone()));
    let reindex = Reindexer::new();
    reindex.fail_for_test(&nested, "ridx index failed: a newer build failed");
    let request = references(&nested, 1, None);
    assert!(index_failed(&request, &mr, &reindex).is_none(), "the nested index loaded answers");
    let answering = mr.doc_ws(&request).and_then(|ws| ws.workspace_root()).map(ridx_format::canonicalize_best_effort);
    assert_eq!(answering, Some(nested.clone()), "and it is the one answering");
    assert!(!mr.add_root(nested.join("target/ridx")), "the same index is not loaded twice");
    assert!(mr.remove_root(&nested.join("target/ridx")), "removed by the directory it was opened as");
    assert!(mr.roots().all(|r| r != nested), "{:?}", mr.roots().collect::<Vec<_>>());
    assert!(mr.roots().any(|r| r == outer), "the enclosing one stays");
}

/// A shard of `root`'s package and its sidecar in `dir`.
fn shard_in(dir: &Path, root: &Path) {
    std::fs::create_dir_all(dir.join(".deps")).unwrap();
    let mut w = ridx_format::IndexWriter::new("w", "0.1.0");
    w.get_or_insert_file("src/lib.rs");
    w.write_to(&mut std::fs::File::create(dir.join("w.ridx")).unwrap()).unwrap();
    let meta = json!({ "name": "w", "version": "0.1.0", "source_root": root.display().to_string(), "target_kind": "lib", "is_dependency": false });
    std::fs::write(dir.join(".deps/w.json"), meta.to_string()).unwrap();
}

fn manifest(root: &Path) -> Value {
    json!({ "version": "2.0", "format_version": ridx_format::CURRENT_FORMAT_VERSION, "extractor_version": ridx_format::EXTRACTOR_VERSION,
        "active_variant_id": "dev", "variant_dir": "variants/dev", "workspace_root": root.display().to_string() })
}

/// Review of 6b8ee66c: every place an index is opened by (a flat `target/ridx`, a variant directory, an index kept
/// outside the workspace) answers for the workspace it records, after a later run of it failed, and is removed by
/// that place.
#[test]
fn test_every_form_of_an_index_directory_answers_for_its_workspace() {
    let elsewhere = tempfile::tempdir().unwrap();
    for form in ["flat", "variant", "elsewhere"] {
        let dir = tempfile::tempdir().unwrap();
        let root = ridx_format::canonicalize_best_effort(dir.path());
        package(&root);
        let opened = match form {
            "flat" => {
                shard_in(&root.join("target/ridx"), &root);
                root.join("target/ridx")
            }
            "variant" => {
                let variant = root.join("target/ridx/variants/dev");
                shard_in(&variant, &root);
                std::fs::write(variant.join("variant_manifest.json"), manifest(&root).to_string()).unwrap();
                variant
            }
            _ => {
                let kept = ridx_format::canonicalize_best_effort(elsewhere.path()).join(format!("kept-{}", std::process::id()));
                shard_in(&kept.join("variants/dev"), &root);
                std::fs::write(kept.join("ridx_manifest.json"), manifest(&root).to_string()).unwrap();
                kept
            }
        };
        let mut mr = MultiRootWorkspace::new();
        assert!(mr.add_root(opened.clone()), "{form}");
        assert_eq!(mr.first_workspace().and_then(|ws| ws.workspace_root()).map(ridx_format::canonicalize_best_effort), Some(root.clone()), "{form}");
        assert!(mr.roots().any(|r| r == root), "{form}: bound to its workspace");
        let reindex = Reindexer::new();
        reindex.fail_for_test(&root, "ridx index failed: a newer build failed");
        assert!(index_failed(&references(&root, 1, None), &mr, &reindex).is_none(), "{form}: the index loaded answers");
        assert!(mr.remove_root(&opened), "{form}: removed by the place it was opened by");
        assert!(mr.is_empty(), "{form}");
    }
}

/// Review of 6b8ee66c: removing a workspace kept inside another one's directories (`A/target`, a package of its own)
/// removed `A` too, whose shards lie under `A/target`.
#[test]
fn test_removing_one_root_keeps_another_whose_shards_lie_under_it() {
    let dir = tempfile::tempdir().unwrap();
    let outer = ridx_format::canonicalize_best_effort(dir.path());
    let inner = outer.join("target");
    package(&outer);
    publish_index(&outer);
    package(&inner);
    publish_index(&inner);
    let mut mr = MultiRootWorkspace::new();
    assert!(mr.add_root(outer.clone()));
    assert!(mr.add_root(inner.clone()));
    assert_eq!(mr.len(), 2);
    assert!(mr.remove_root(&inner));
    assert_eq!(mr.roots().collect::<Vec<_>>(), vec![outer.as_path()], "only the root removed goes");
}
