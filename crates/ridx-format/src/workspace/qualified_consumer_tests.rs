/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! References to an item whose path is qualified (`<Type as Trait>::name`) include the uses in a
//! crate that spells the path with the defining crate in front.

use super::sibling_tests::create_test_workspace;
use super::*;
use crate::format::DefKind;
use crate::writer::{IndexWriter, RawDefinition, RawReference};
use std::fs::File;
use std::path::{Path, PathBuf};

const DEF_PATH: &str = "<stats::Stats as core::clone::Clone>::clone";

/// Shard `name` of unit `unit_<name>`: the definition of `DEF_PATH` in crate `a`, or a reference
/// to `ref_path` in crate `b`.
fn write_shard(dir: &Path, name: &str, ref_path: Option<&str>) -> PathBuf {
    let path = dir.join(format!("{name}.ridx"));
    let mut w = IndexWriter::new(name, "0.1.0");
    w.set_unit_key(format!("unit_{name}"));
    match ref_path {
        None => {
            w.add_definition(RawDefinition {
                def_id: 1,
                kind: DefKind::AssocFn,
                visibility: 0,
                is_macro_expansion: false,
                name: "clone".to_string(),
                path: DEF_PATH.to_string(),
                signature: Some("fn(&self) -> Stats".to_string()),
                file: "src/stats.rs".to_string(),
                start_line: 12,
                start_col: 10,
                end_line: 12,
                end_col: 15,
            });
        }
        Some(target_path) => w.add_reference(RawReference {
            file: "src/search.rs".to_string(),
            start_line: 5,
            start_col: 9,
            end_line: 5,
            end_col: 14,
            target_def_id: None,
            target_path: target_path.to_string(),
            target_file: Some("src/stats.rs".to_string()),
            target_unit: Some("unit_a".to_string()),
            target_line: 12,
            target_col: 10,
            kind: DefKind::AssocFn,
            is_write: false,
            is_macro_expansion: false,
        }),
    }
    w.write_to(&mut File::create(&path).unwrap()).unwrap();
    path
}

/// Crate `a` defines the path; crate `b`, depending on it, uses it spelled `ref_path`.
fn workspace(ref_path: &str) -> (tempfile::TempDir, WorkspaceIndex) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let a = write_shard(dir.path(), "a", None);
    let b = write_shard(dir.path(), "b", Some(ref_path));
    let infos = [("a", &a), ("b", &b)].map(|(n, p)| (p.clone(), Some(root.join(n)), Some(format!("unit_{n}"))));
    let mut ws = create_test_workspace(infos.iter().map(|(p, r, u)| (p.as_path(), r.clone(), u.clone(), Some(TargetKind::Lib), false)).collect(), root.clone());
    ws.reader_dependencies[1] = vec!["unit_a".to_string()];
    (dir, ws)
}

fn files(ws: &WorkspaceIndex) -> Vec<String> {
    let refs = ws.find_references_for_unit(DEF_PATH, Some("unit_a"));
    refs.iter().map(|r| r.file.to_string()).collect()
}

#[test]
fn a_use_spelled_with_the_defining_crate_in_front_of_the_type_is_found() {
    let (_dir, ws) = workspace("<a::stats::Stats as core::clone::Clone>::clone");
    assert_eq!(files(&ws), vec!["src/search.rs"]);
}

#[test]
fn a_use_spelled_with_the_crate_in_front_of_the_trait_is_found() {
    let (_dir, ws) = workspace("<stats::Stats as a::core::clone::Clone>::clone");
    assert_eq!(files(&ws), vec!["src/search.rs"]);
}

#[test]
fn a_use_of_another_item_spelled_that_way_is_not_found() {
    let (_dir, ws) = workspace("<a::stats::Other as core::clone::Clone>::clone");
    assert!(files(&ws).is_empty());
}
