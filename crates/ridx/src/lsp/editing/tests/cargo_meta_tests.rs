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

fn package(dir: &Path, name: &str) {
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("Cargo.toml"), format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n")).unwrap();
    std::fs::write(dir.join("src/lib.rs"), "").unwrap();
}

/// Review of 8c5b025d: a TOML literal string is read as a basic one; a pattern's every wildcard level is listed.
#[test]
fn test_member_patterns_in_either_quote_are_listed_to_their_depth() {
    assert_eq!(quoted("members = ['a/*', \"b\\\"c\"]"), vec!["a/*".to_string(), "b\"c".to_string()]);
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::write(root.join("Cargo.toml"), "[workspace]\nmembers = ['groups/*/members/*']\nresolver = \"2\"\n").unwrap();
    package(&root.join("groups/g1/members/old"), "old");
    let before = fingerprint(&root.join("Cargo.toml"));
    package(&root.join("groups/g1/members/new"), "new");
    assert_ne!(fingerprint(&root.join("Cargo.toml")), before, "a member under a branch already there");
}

/// Review of 8c5b025d: the layout is Cargo's answer now, never a cached one a new member is missing from.
#[test]
fn test_the_layout_sees_a_member_added_since_the_last_lookup() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::write(root.join("Cargo.toml"), "[workspace]\nmembers = ['groups/*/members/*']\nresolver = \"2\"\n").unwrap();
    package(&root.join("groups/g1/members/old"), "old");
    let file = root.join("groups/g1/members/old/src/lib.rs");
    let first = layout(&file).unwrap().unwrap();
    assert_eq!(first.packages.len(), 1);
    package(&root.join("groups/g1/members/new"), "new");
    let second = layout(&file).unwrap().unwrap();
    assert_eq!(second.packages.len(), 2, "{:?}", second.packages);
}
