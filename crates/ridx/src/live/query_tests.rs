/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Unit tests for [`super::query::LiveDb`] across clean disk files and dirty [`OverlayManager`] buffers.

use std::fs;

use ridx_format::{DefKind, IndexWriter, RawDefinition, WorkspaceIndex};
use tempfile::TempDir;

use crate::overlay::OverlayManager;

use super::query::LiveDb;

fn setup_tiny_workspace() -> (TempDir, std::path::PathBuf, WorkspaceIndex) {
    let dir = TempDir::new().unwrap();
    let src_dir = dir.path().join("src");
    fs::create_dir_all(&src_dir).unwrap();
    let lib_rs = src_dir.join("lib.rs");
    let clean_src = concat!(
        "pub struct Counter {\n",
        "    pub count: usize,\n",
        "}\n",
        "impl Counter {\n",
        "    pub fn new(v: usize) -> Self { Self { count: v } }\n",
        "    pub fn inc(&mut self) -> usize { self.count += 1; self.count }\n",
        "}\n",
        "pub enum Mode { Fast, Safe }\n",
        "pub mod sub {\n",
        "    pub fn helper() -> u32 { 7 }\n",
        "}\n",
        "pub fn demo() -> usize {\n",
        "    let mut c = Counter::new(10);\n",
        "    c.inc()\n",
        "}\n",
    );
    fs::write(&lib_rs, clean_src).unwrap();

    let ridx_dir = dir.path().join("target/ridx");
    fs::create_dir_all(&ridx_dir).unwrap();
    let mut writer = IndexWriter::new("tiny_demo", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Struct,
        visibility: 0,
        is_macro_expansion: false,
        name: "Counter".to_string(),
        path: "Counter".to_string(),
        signature: Some("pub struct Counter".to_string()),
        file: "src/lib.rs".to_string(),
        start_line: 1,
        start_col: 12,
        end_line: 1,
        end_col: 19,
    });
    let ridx_file = ridx_dir.join("tiny_demo.ridx");
    writer.write_to(&mut fs::File::create(&ridx_file).unwrap()).unwrap();
    let mut ws = WorkspaceIndex::open(&ridx_dir).unwrap();
    ws.set_workspace_root(Some(dir.path().to_path_buf()));
    (dir, lib_rs, ws)
}

fn assert_owned_send<T: Send + 'static>() {}

#[test]
fn live_db_clean_and_dirty_overlay_queries() {
    assert_owned_send::<LiveDb>();

    let (_dir, lib_rs, mut ws) = setup_tiny_workspace();
    let mut live = LiveDb::new();
    assert_eq!(live.cached_fn_count(), 0, "LiveDb::new must be lazy");

    let clean_text = fs::read_to_string(&lib_rs).unwrap();
    let c_decl_off = clean_text.find("mut c =").unwrap() as u32 + 4;
    let ty = live.type_at(&ws, &lib_rs, c_decl_off, None).expect("expected Counter type at `c`");
    assert_eq!(ty.display, "Counter");
    assert_eq!(ty.defs.len(), 1);
    assert_eq!(ty.defs[0].line, 1);
    assert_eq!(live.cached_fn_count(), 1);

    let dot_off = clean_text.find("c.inc()").unwrap() as u32 + 2;
    let dot_cands = live.dot_candidates(&ws, &lib_rs, dot_off, None);
    let dot_labels: Vec<&str> = dot_cands.iter().map(|c| c.label.as_str()).collect();
    assert!(dot_labels.contains(&"count"), "expected field `count`, got {dot_labels:?}");
    assert!(dot_labels.contains(&"inc"), "expected inherent method `inc`, got {dot_labels:?}");
    assert!(!dot_labels.contains(&"new"), "assoc fn without self must not appear in dot_candidates");

    let colon_type_off = clean_text.find("Counter::new").unwrap() as u32 + "Counter::".len() as u32;
    let type_cands = live.path_candidates(&ws, &lib_rs, colon_type_off, None);
    let type_labels: Vec<&str> = type_cands.iter().map(|c| c.label.as_str()).collect();
    assert!(type_labels.contains(&"new") && type_labels.contains(&"inc"), "got {type_labels:?}");

    // Mutate `ws` in place while `live` stays alive (`LiveDb` holds zero borrow of `ws`).
    ws.bump_generation();
    // Generation bump must auto-invalidate stale caches on next query and repopulate cleanly.
    let ty_after_bump = live.type_at(&ws, &lib_rs, c_decl_off, None).expect("expected Counter after ws reload");
    assert_eq!(ty_after_bump.display, "Counter");
    assert_eq!(live.cached_fn_count(), 1);

    // Explicit `invalidate()` drops derived caches immediately.
    live.invalidate();
    assert_eq!(live.cached_fn_count(), 0);

    // Dirty buffer via OverlayManager adding a brand-new struct + method + incomplete dot
    let mut ov = OverlayManager::new();
    let dirty_text = format!(
        "{clean_text}\npub struct Added {{ pub val: u64 }}\nimpl Added {{ pub fn double(&self) -> u64 {{ self.val * 2 }} }}\npub fn edit_fn(a: Added) {{\n    let m = Mode::Fast;\n    let h = sub::helper();\n    a.\n}}\n"
    );
    let path_str = lib_rs.to_str().unwrap();
    ov.open_with_provenance(&format!("file://{path_str}"), path_str, 1, clean_text, true);
    ov.update(path_str, 2, dirty_text.clone());

    let dirty_dot = dirty_text.rfind("a.").unwrap() as u32 + 2;
    let d_cands = live.dot_candidates(&ws, &lib_rs, dirty_dot, Some(&ov));
    let d_labels: Vec<&str> = d_cands.iter().map(|c| c.label.as_str()).collect();
    assert!(d_labels.contains(&"val") && d_labels.contains(&"double"), "dirty dot cands: {d_labels:?}");

    let mode_colon = dirty_text.find("Mode::Fast").unwrap() as u32 + "Mode::".len() as u32;
    let mode_cands = live.path_candidates(&ws, &lib_rs, mode_colon, Some(&ov));
    let mode_labels: Vec<&str> = mode_cands.iter().map(|c| c.label.as_str()).collect();
    assert_eq!(mode_labels, vec!["Fast", "Safe"]);

    let sub_colon = dirty_text.find("sub::helper").unwrap() as u32 + "sub::".len() as u32;
    let sub_cands = live.path_candidates(&ws, &lib_rs, sub_colon, Some(&ov));
    assert!(sub_cands.iter().any(|c| c.label == "helper" && c.kind == DefKind::Fn));
}

#[test]
fn live_ty_defs_exclude_impls_assocs_and_non_typeable_tokens() {
    let (_dir, lib_rs, ws) = setup_tiny_workspace();
    let mut live = LiveDb::new();
    let mut ov = OverlayManager::new();
    let src = concat!(
        "pub trait SlabItem {\n",
        "    type Key;\n",
        "}\n",
        "pub enum RegErr<T> {\n",
        "    Missing(T),\n",
        "    Empty,\n",
        "}\n",
        "impl<T> RegErr<T> {\n",
        "    pub fn check(&self) -> bool {\n",
        "        match self {\n",
        "            Self::Empty => true,\n",
        "            _ => false,\n",
        "        }\n",
        "    }\n",
        "}\n",
        "pub fn take_key<I: SlabItem>(key: &I::Key) {\n",
        "    let x = 42;\n",
        "    let _ = (key, x);\n",
        "}\n",
    );
    let path_str = lib_rs.to_str().unwrap();
    ov.open_with_provenance(&format!("file://{path_str}"), path_str, 1, String::new(), true);
    ov.update(path_str, 2, src.to_string());

    // Keyword `let`, integer literal `42`, and punctuation `;` must return None.
    let let_off = src.find("let x = 42;").unwrap() as u32;
    assert!(live.type_at(&ws, &lib_rs, let_off, Some(&ov)).is_none(), "keyword `let` must return None");
    let lit_off = src.find("42;").unwrap() as u32;
    assert!(live.type_at(&ws, &lib_rs, lit_off, Some(&ov)).is_none(), "literal `42` must return None");
    let semi_off = src.find("42;").unwrap() as u32 + 2;
    assert!(live.type_at(&ws, &lib_rs, semi_off, Some(&ov)).is_none(), "punctuation `;` must return None");

    // Unnormalized projection `&I::Key` must resolve only to trait `SlabItem` (line 1), not `type Key` (line 2) or `fn take_key`.
    let key_ty_off = src.find("I::Key").unwrap() as u32 + 3;
    let key_ty = live.type_at(&ws, &lib_rs, key_ty_off, Some(&ov)).expect("expected type at `I::Key`");
    let key_lines: Vec<u32> = key_ty.defs.iter().map(|d| d.line).collect();
    assert_eq!(key_lines, vec![1], "projection defs should contain only trait SlabItem: {key_lines:?}");

    // `Self::Empty` inside `impl<T> RegErr<T>` must resolve only to enum `RegErr` (line 4), not the `impl` block (line 8).
    let self_pat_off = src.find("Self::Empty").unwrap() as u32 + 6;
    let self_ty = live.type_at(&ws, &lib_rs, self_pat_off, Some(&ov)).expect("expected type at `Self::Empty`");
    let self_lines: Vec<u32> = self_ty.defs.iter().map(|d| d.line).collect();
    assert_eq!(self_lines, vec![4], "Self::Empty defs should contain only enum RegErr: {self_lines:?}");
}

#[test]
fn live_type_at_evaluates_indexed_cfg_features_not_any_and_cfg_attr() {
    let dir = TempDir::new().unwrap();
    let src_dir = dir.path().join("src");
    fs::create_dir_all(&src_dir).unwrap();
    let lib_rs = src_dir.join("lib.rs");
    let src = concat!(
        "pub struct ActiveVal;\n",
        "#[cfg(feature = \"alpha\")]\n",
        "pub fn enabled_by_feature(v_ok: ActiveVal) -> ActiveVal { v_ok }\n",
        "#[cfg(feature = \"beta\")]\n",
        "pub fn disabled_by_feature(v_dead: ActiveVal) -> ActiveVal { v_dead }\n",
        "#[cfg(not(feature = \"beta\"))]\n",
        "pub fn enabled_by_not(v_not: ActiveVal) -> ActiveVal { v_not }\n",
        "#[cfg(any(feature = \"beta\", custom_flag))]\n",
        "pub fn enabled_by_any(v_any: ActiveVal) -> ActiveVal { v_any }\n",
        "#[cfg_attr(custom_flag, cfg(feature = \"beta\"))]\n",
        "pub fn disabled_by_cfg_attr(v_attr: ActiveVal) -> ActiveVal { v_attr }\n",
        "pub fn block_tail_cfg(v_tail: ActiveVal) {\n",
        "    #[cfg(feature = \"beta\")]\n",
        "    { let _ = v_tail; }\n",
        "}\n",
    );
    fs::write(&lib_rs, src).unwrap();

    let ridx_dir = dir.path().join("target/ridx");
    fs::create_dir_all(&ridx_dir).unwrap();
    let mut writer = IndexWriter::new("cfg_demo", "0.1.0");
    writer.add_cfg_flag("custom_flag");
    writer.add_cfg_key_value("feature", "alpha");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Struct,
        visibility: 0,
        is_macro_expansion: false,
        name: "ActiveVal".to_string(),
        path: "ActiveVal".to_string(),
        signature: Some("pub struct ActiveVal".to_string()),
        file: "src/lib.rs".to_string(),
        start_line: 1,
        start_col: 12,
        end_line: 1,
        end_col: 21,
    });
    writer.write_to(&mut fs::File::create(ridx_dir.join("cfg_demo.ridx")).unwrap()).unwrap();
    let mut ws = WorkspaceIndex::open(&ridx_dir).unwrap();
    ws.set_workspace_root(Some(dir.path().to_path_buf()));

    let mut live = LiveDb::new();
    for needle in ["v_ok:", "v_not:", "v_any:"] {
        let off = src.find(needle).unwrap() as u32 + 1;
        assert!(live.type_at(&ws, &lib_rs, off, None).is_some(), "expected active cfg at {needle}");
    }
    for needle in ["v_dead:", "v_attr:", "let _ = v_tail"] {
        let off = src.find(needle).unwrap() as u32 + if needle.starts_with("let") { 8 } else { 1 };
        assert!(live.type_at(&ws, &lib_rs, off, None).is_none(), "expected inactive cfg at {needle}");
    }
}
