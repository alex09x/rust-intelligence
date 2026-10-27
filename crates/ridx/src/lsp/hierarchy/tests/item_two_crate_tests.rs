/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! incomingCalls of a function of crate `a` finds the callers in crate `b` that depends on it, in
//! both orders of the shards, also when `a` is built twice (two units of one crate and version)
//! and `b` depends on the other build: the references are scoped to the unit of the item, and a
//! consumer of a sibling build is a consumer of the item.

use super::item_position_tests::incoming_callers;
use ridx_format::{DefKind, IndexWriter, RawDefinition, RawReference, WorkspaceIndex};
use std::fs;
use std::path::{Path, PathBuf};

static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn def(id: u32, name: &str, path: &str, line: u32, col: u32, file: &str) -> RawDefinition {
    RawDefinition {
        def_id: id,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        file: file.to_string(),
        name: name.to_string(),
        start_line: line,
        start_col: col,
        end_line: line,
        end_col: col + name.len() as u32,
        path: path.to_string(),
        signature: Some("fn()".to_string()),
    }
}

/// Writes the shard `<stem>.ridx` of crate `krate` with unit `unit` and its dependency sidecar.
fn shard(dir: &Path, stem: &str, krate: &str, unit: &str, deps: &[&str], w: IndexWriter) {
    let mut w = w;
    w.set_unit_key(unit);
    w.write_to(&mut fs::File::create(dir.join(format!("{stem}.ridx"))).unwrap())
        .unwrap();
    fs::create_dir_all(dir.join(".deps")).unwrap();
    let info = serde_json::json!({ "name": krate, "version": "0.1.0", "unit_key": unit,
        "dependencies": deps });
    fs::write(dir.join(format!(".deps/{stem}.json")), info.to_string()).unwrap();
}

/// Crate `a` (`pub fn f` on line 1 of `a.rs`), built as unit `a-lib` and, when `second_build`,
/// also as `a-lib2`; crate `b` (unit `b-lib`) depends on `b_dep` and holds `fn g`, which calls
/// `a::f` on line 2 of `b.rs`. `b_first` makes the shard of `b` sort before those of `a`.
fn workspace(b_first: bool, second_build: bool, b_dep: &str) -> (PathBuf, WorkspaceIndex) {
    let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_two_crate_{}_{}", std::process::id(), id));
    fs::create_dir_all(&dir).unwrap();
    let (a_file, b_file) = (dir.join("a.rs"), dir.join("b.rs"));
    fs::write(&a_file, "pub fn f() {}\n").unwrap();
    fs::write(&b_file, "fn g() {\n    a::f();\n}\n").unwrap();
    let (a, b) = (a_file.to_str().unwrap(), b_file.to_str().unwrap());
    let (b_stem, a_stem) = if b_first {
        ("a_b", "b_a")
    } else {
        ("b_b", "a_a")
    };

    let build = |unit: &str| {
        let mut w = IndexWriter::new("a", "0.1.0");
        w.add_definition(def(1, "f", "a::f", 1, 8, a));
        (unit.to_string(), w)
    };
    let (unit, w) = build("a-lib");
    shard(&dir, a_stem, "a", &unit, &[], w);
    if second_build {
        let (unit, w) = build("a-lib2");
        shard(&dir, &format!("{a_stem}2"), "a", &unit, &[], w);
    }

    let mut w = IndexWriter::new("b", "0.1.0");
    w.add_definition(def(1, "g", "b::g", 1, 4, b));
    w.add_reference(RawReference {
        file: b.to_string(),
        start_line: 2,
        start_col: 8,
        end_line: 2,
        end_col: 9,
        target_def_id: None,
        target_path: "a::f".to_string(),
        target_unit: Some("a-lib".to_string()),
        target_file: Some(a.to_string()),
        target_line: 1,
        target_col: 8,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    shard(&dir, b_stem, "b", "b-lib", &[b_dep], w);
    let ws = WorkspaceIndex::open(&dir).unwrap();
    (dir, ws)
}

fn callers_of_f(b_first: bool, second_build: bool, b_dep: &str) -> Vec<(String, Vec<u64>)> {
    let (dir, ws) = workspace(b_first, second_build, b_dep);
    // The item as prepareCallHierarchy answers it: the name of `f` at line 0, character 7.
    let callers = incoming_callers(&ws, &dir.join("a.rs"), 0, 7);
    let _ = fs::remove_dir_all(dir);
    callers
}

fn g_calls_f() -> Vec<(String, Vec<u64>)> {
    vec![("g".to_string(), vec![1])]
}

#[test]
fn a_dependent_crate_is_a_caller_with_its_shard_after_the_item_crate() {
    assert_eq!(callers_of_f(false, false, "a-lib"), g_calls_f());
}

#[test]
fn a_dependent_crate_is_a_caller_with_its_shard_before_the_item_crate() {
    assert_eq!(callers_of_f(true, false, "a-lib"), g_calls_f());
}

#[test]
fn a_crate_depending_on_another_build_is_a_caller_with_its_shard_after() {
    assert_eq!(callers_of_f(false, true, "a-lib2"), g_calls_f());
}

#[test]
fn a_crate_depending_on_another_build_is_a_caller_with_its_shard_before() {
    assert_eq!(callers_of_f(true, true, "a-lib2"), g_calls_f());
}

#[test]
fn a_crate_depending_on_nothing_of_the_item_crate_is_no_caller_shard_after() {
    assert_eq!(callers_of_f(false, true, "other-lib"), vec![]);
}

#[test]
fn a_crate_depending_on_nothing_of_the_item_crate_is_no_caller_shard_before() {
    assert_eq!(callers_of_f(true, true, "other-lib"), vec![]);
}
