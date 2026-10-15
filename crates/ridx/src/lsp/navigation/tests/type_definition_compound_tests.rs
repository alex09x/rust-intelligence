/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Regression tests for `textDocument/typeDefinition` compound type rules,
//! `TyParam` scope resolution, and multi-crate `owner_path` disambiguation.

use crate::lsp::navigation::handle_type_definition;
use crate::lsp::protocol::PositionConverter;
use crate::overlay::OverlayManager;
use ridx_format::{DefKind, IndexWriter, RawDefinition, RawType, WorkspaceIndex};
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static COMPOUND_COUNTER: AtomicU64 = AtomicU64::new(0);

fn next_temp_dir(prefix: &str) -> PathBuf {
    let id = COMPOUND_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_{}_{}_{}", prefix, std::process::id(), id));
    let _ = fs::create_dir_all(&dir);
    dir
}

fn raw_def(
    def_id: u32,
    kind: DefKind,
    path: &str,
    file: &str,
    (line, sc, ec): (u32, u32, u32),
) -> RawDefinition {
    let name = path.rsplit("::").next().unwrap_or(path).to_string();
    RawDefinition {
        def_id,
        kind,
        visibility: 0,
        is_macro_expansion: false,
        name,
        path: path.to_string(),
        signature: None,
        file: file.to_string(),
        start_line: line,
        start_col: sc,
        end_line: line,
        end_col: ec,
    }
}

fn add_ty(
    w: &mut IndexWriter,
    file: &str,
    (line, sc, ec): (u32, u32, u32),
    ty: &str,
    owner: Option<&str>,
) {
    let raw = RawType {
        file: file.to_string(),
        start_line: line,
        start_col: sc,
        end_line: line,
        end_col: ec,
        type_name: ty.to_string(),
    };
    w.add_type_with_owner(raw, owner.map(str::to_string));
}

fn query(ws: &WorkspaceIndex, uri: &str, line: u32, character: u32) -> Value {
    let msg = json!({
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": character }
        }
    });
    handle_type_definition(
        &msg,
        Some(ws),
        &OverlayManager::new(),
        &mut PositionConverter::new(),
    )
}

fn setup_compound_workspace() -> (PathBuf, WorkspaceIndex, String, String, String) {
    let root = next_temp_dir("typedef_compound");
    let app_lib = root.join("app/src/lib.rs");
    let core_lib = root.join("core/src/lib.rs");
    let alloc_lib = root.join("alloc/src/lib.rs");
    for f in [&app_lib, &core_lib, &alloc_lib] {
        let _ = fs::create_dir_all(f.parent().unwrap());
    }

    let app_code = concat!(
        "pub struct Foo<'a>(&'a str);\n",
        "pub struct Wrapper<T>(pub T);\n",
        "pub trait BoundTrait {}\n",
        "pub fn test_refs<'a>(r: &Foo<'a>, m: &mut Foo<'a>, lt: Foo<'a>) {}\n",
        "pub fn test_generics(w: Wrapper<i32>, o: Option<Foo<'static>>, b: Box<Foo<'static>>, p: u32) {}\n",
        "pub fn test_ty_param<T: BoundTrait>(x: T) {}\n",
    );
    fs::write(&app_lib, app_code).unwrap();
    let core_code =
        "pub enum Option<T> { None, Some(T) }\npub enum Result<T, E> { Ok(T), Err(E) }\n";
    fs::write(&core_lib, core_code).unwrap();
    fs::write(&alloc_lib, "pub struct Box<T>(T);\n").unwrap();

    let app_s = app_lib.to_str().unwrap();
    let core_s = core_lib.to_str().unwrap();
    let alloc_s = alloc_lib.to_str().unwrap();

    let mut cw = IndexWriter::new("core", "1.0.0");
    cw.add_definition(raw_def(
        1,
        DefKind::Enum,
        "core::option::Option",
        core_s,
        (1, 10, 16),
    ));
    cw.add_definition(raw_def(
        2,
        DefKind::Enum,
        "core::result::Result",
        core_s,
        (2, 10, 16),
    ));
    cw.write_to(&mut fs::File::create(root.join("00_core.ridx")).unwrap())
        .unwrap();

    let mut aw = IndexWriter::new("alloc", "1.0.0");
    aw.add_definition(raw_def(
        1,
        DefKind::Struct,
        "alloc::boxed::Box",
        alloc_s,
        (1, 12, 15),
    ));
    aw.write_to(&mut fs::File::create(root.join("01_alloc.ridx")).unwrap())
        .unwrap();

    let mut w = IndexWriter::new("app", "0.1.0");
    w.add_definition(raw_def(10, DefKind::Struct, "app::Foo", app_s, (1, 12, 15)));
    w.add_definition(raw_def(
        11,
        DefKind::Struct,
        "app::Wrapper",
        app_s,
        (2, 12, 19),
    ));
    w.add_definition(raw_def(
        12,
        DefKind::Trait,
        "app::BoundTrait",
        app_s,
        (3, 11, 21),
    ));
    w.add_definition_with_owners(
        raw_def(
            13,
            DefKind::TyParam,
            "app::test_ty_param::T",
            app_s,
            (6, 22, 23),
        ),
        vec!["scope:6:1:6:46".to_string()],
    );

    add_ty(&mut w, app_s, (4, 22, 23), "&Foo", Some("app::Foo"));
    add_ty(&mut w, app_s, (4, 35, 36), "&mut Foo", Some("app::Foo"));
    add_ty(&mut w, app_s, (4, 50, 52), "Foo<'a>", Some("app::Foo"));
    add_ty(
        &mut w,
        app_s,
        (5, 22, 23),
        "Wrapper<i32>",
        Some("app::Wrapper"),
    );
    add_ty(
        &mut w,
        app_s,
        (5, 39, 40),
        "Option<Foo>",
        Some("core::option::Option"),
    );
    add_ty(
        &mut w,
        app_s,
        (5, 64, 65),
        "Box<Foo>",
        Some("alloc::boxed::Box"),
    );
    add_ty(&mut w, app_s, (5, 86, 87), "u32", Some("u32"));
    add_ty(&mut w, app_s, (6, 37, 38), "T", Some("app::BoundTrait"));
    w.write_to(&mut fs::File::create(root.join("02_app.ridx")).unwrap())
        .unwrap();

    let ws = WorkspaceIndex::open(&root).unwrap();
    (
        root,
        ws,
        format!("file://{app_s}"),
        format!("file://{core_s}"),
        format!("file://{alloc_s}"),
    )
}

#[test]
fn test_typedef_shared_ref_foo() {
    let (root, ws, app_uri, _, _) = setup_compound_workspace();
    let res = query(&ws, &app_uri, 3, 21);
    assert_eq!(res["uri"], app_uri);
    assert_eq!(res["range"]["start"]["line"], 0);
    assert_eq!(res["range"]["start"]["character"], 11);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_typedef_mut_ref_foo() {
    let (root, ws, app_uri, _, _) = setup_compound_workspace();
    let res = query(&ws, &app_uri, 3, 34);
    assert_eq!(res["uri"], app_uri);
    assert_eq!(res["range"]["start"]["line"], 0);
    assert_eq!(res["range"]["start"]["character"], 11);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_typedef_lifetime_generic_foo() {
    let (root, ws, app_uri, _, _) = setup_compound_workspace();
    let res = query(&ws, &app_uri, 3, 49);
    assert_eq!(res["uri"], app_uri);
    assert_eq!(res["range"]["start"]["line"], 0);
    assert_eq!(res["range"]["start"]["character"], 11);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_typedef_wrapper_i32_resolves_outer_wrapper() {
    let (root, ws, app_uri, _, _) = setup_compound_workspace();
    let res = query(&ws, &app_uri, 4, 21);
    assert_eq!(res["uri"], app_uri);
    assert_eq!(res["range"]["start"]["line"], 1);
    assert_eq!(res["range"]["start"]["character"], 11);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_typedef_option_foo_resolves_outer_option() {
    let (root, ws, app_uri, core_uri, _) = setup_compound_workspace();
    let res = query(&ws, &app_uri, 4, 38);
    assert_eq!(res["uri"], core_uri);
    assert_eq!(res["range"]["start"]["line"], 0);
    assert_eq!(res["range"]["start"]["character"], 9);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_typedef_box_foo_resolves_outer_box() {
    let (root, ws, app_uri, _, alloc_uri) = setup_compound_workspace();
    let res = query(&ws, &app_uri, 4, 63);
    assert_eq!(res["uri"], alloc_uri);
    assert_eq!(res["range"]["start"]["line"], 0);
    assert_eq!(res["range"]["start"]["character"], 11);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_typedef_primitive_u32_returns_null() {
    let (root, ws, app_uri, _, _) = setup_compound_workspace();
    let res = query(&ws, &app_uri, 4, 85);
    assert_eq!(res, Value::Null);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_typedef_ty_param_t_resolves_to_declaration() {
    let (root, ws, app_uri, _, _) = setup_compound_workspace();
    let res = query(&ws, &app_uri, 5, 36);
    assert_eq!(res["uri"], app_uri);
    assert_eq!(res["range"]["start"]["line"], 5);
    assert_eq!(res["range"]["start"]["character"], 21);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_typedef_multi_crate_same_name_config_uses_owner_path() {
    let root = next_temp_dir("typedef_multi_config");
    let a_file = root.join("crate_a/src/lib.rs");
    let b_file = root.join("crate_b/src/lib.rs");
    let c_file = root.join("crate_c/src/lib.rs");
    for f in [&a_file, &b_file, &c_file] {
        let _ = fs::create_dir_all(f.parent().unwrap());
    }
    fs::write(&a_file, "pub struct Config;\n").unwrap();
    fs::write(&b_file, "// crate_b\npub struct Config;\n").unwrap();
    fs::write(
        &c_file,
        "pub fn init(cfg: crate_b::Config, cfg2: crate_b::Config) {}\n",
    )
    .unwrap();

    let a_s = a_file.to_str().unwrap();
    let b_s = b_file.to_str().unwrap();
    let c_s = c_file.to_str().unwrap();

    let mut wa = IndexWriter::new("crate_a", "0.1.0");
    wa.add_definition(raw_def(
        1,
        DefKind::Struct,
        "crate_a::Config",
        a_s,
        (1, 12, 18),
    ));
    wa.write_to(&mut fs::File::create(root.join("00_crate_a.ridx")).unwrap())
        .unwrap();

    let mut wb = IndexWriter::new("crate_b", "0.1.0");
    wb.add_definition(raw_def(
        1,
        DefKind::Struct,
        "crate_b::Config",
        b_s,
        (2, 12, 18),
    ));
    wb.write_to(&mut fs::File::create(root.join("01_crate_b.ridx")).unwrap())
        .unwrap();

    let mut wc = IndexWriter::new("crate_c", "0.1.0");
    add_ty(&mut wc, c_s, (1, 13, 16), "Config", Some("crate_b::Config"));
    add_ty(&mut wc, c_s, (1, 35, 39), "crate_b::Config", None);
    wc.write_to(&mut fs::File::create(root.join("02_crate_c.ridx")).unwrap())
        .unwrap();

    let ws = WorkspaceIndex::open(&root).unwrap();
    let c_uri = format!("file://{c_s}");
    let b_uri = format!("file://{b_s}");

    for col in [12, 34] {
        let res = query(&ws, &c_uri, 0, col);
        assert_eq!(res["uri"], b_uri);
        assert_eq!(res["range"]["start"]["line"], 1);
        assert_eq!(res["range"]["start"]["character"], 11);
    }

    let _ = fs::remove_dir_all(&root);
}
