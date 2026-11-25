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
use ridx_format::{DefKind, DefKind::*, IndexWriter, RawDefinition, RawType, WorkspaceIndex};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static COMPOUND_COUNTER: AtomicU64 = AtomicU64::new(0);

fn next_temp_dir(prefix: &str) -> PathBuf {
    let id = COMPOUND_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_{}_{}_{}", prefix, std::process::id(), id));
    let _ = fs::create_dir_all(&dir);
    dir
}

type Span3 = (u32, u32, u32);
type Span4 = (u32, u32, u32, u32);
type RefTgt<'a> = (u32, &'a str, u32, u32);

fn save_ridx(w: &mut IndexWriter, path: PathBuf) {
    w.write_to(&mut fs::File::create(path).unwrap()).unwrap();
}

fn raw_def(id: u32, kind: DefKind, path: &str, file: &str, (l, sc, ec): Span3) -> RawDefinition {
    let name = path.rsplit("::").next().unwrap_or(path).to_string();
    RawDefinition {
        def_id: id,
        kind,
        visibility: 0,
        is_macro_expansion: false,
        name,
        path: path.to_string(),
        signature: None,
        file: file.to_string(),
        start_line: l,
        start_col: sc,
        end_line: l,
        end_col: ec,
    }
}

fn add_ty_span(w: &mut IndexWriter, f: &str, (sl, sc, el, ec): Span4, ty: &str, own: Option<&str>) {
    let raw = RawType { file: f.to_string(), start_line: sl, start_col: sc, end_line: el, end_col: ec, type_name: ty.to_string() };
    w.add_type_with_owner(raw, own.map(str::to_string));
}

fn add_ty(w: &mut IndexWriter, f: &str, (l, sc, ec): Span3, ty: &str, own: Option<&str>) {
    add_ty_span(w, f, (l, sc, l, ec), ty, own);
}

fn add_ref(w: &mut IndexWriter, f: &str, sp: Span4, t: RefTgt<'_>, k: DefKind, mac: bool) {
    w.add_reference(ridx_format::RawReference {
        file: f.to_string(),
        start_line: sp.0,
        start_col: sp.1,
        end_line: sp.2,
        end_col: sp.3,
        target_def_id: Some(t.0),
        target_path: t.1.to_string(),
        target_unit: None,
        target_file: Some(f.to_string()),
        target_line: t.2,
        target_col: t.3,
        kind: k,
        is_write: false,
        is_macro_expansion: mac,
    });
}

fn query(ws: &WorkspaceIndex, uri: &str, line: u32, character: u32) -> Value {
    let msg = json!({
        "params": { "textDocument": { "uri": uri }, "position": { "line": line, "character": character } }
    });
    let mut conv = PositionConverter::new();
    handle_type_definition(&msg, Some(ws), &OverlayManager::new(), &mut conv)
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
    let core_code = "pub enum Option<T> { None, Some(T) }\npub enum Result<T, E> { Ok(T), Err(E) }\n";
    fs::write(&core_lib, core_code).unwrap();
    fs::write(&alloc_lib, "pub struct Box<T>(T);\n").unwrap();

    let app_s = app_lib.to_str().unwrap();
    let core_s = core_lib.to_str().unwrap();
    let alloc_s = alloc_lib.to_str().unwrap();

    let mut cw = IndexWriter::new("core", "1.0.0");
    cw.add_definition(raw_def(1, Enum, "core::option::Option", core_s, (1, 10, 16)));
    cw.add_definition(raw_def(2, Enum, "core::result::Result", core_s, (2, 10, 16)));
    save_ridx(&mut cw, root.join("00_core.ridx"));

    let mut aw = IndexWriter::new("alloc", "1.0.0");
    aw.add_definition(raw_def(1, Struct, "alloc::boxed::Box", alloc_s, (1, 12, 15)));
    save_ridx(&mut aw, root.join("01_alloc.ridx"));

    let mut w = IndexWriter::new("app", "0.1.0");
    for (id, k, p, sp) in [(10, Struct, "app::Foo", (1, 12, 15)), (11, Struct, "app::Wrapper", (2, 12, 19)), (12, Trait, "app::BoundTrait", (3, 11, 21))] {
        w.add_definition(raw_def(id, k, p, app_s, sp));
    }
    let tp_def = raw_def(13, TyParam, "app::test_ty_param::T", app_s, (6, 22, 23));
    w.add_definition_with_owners(tp_def, vec!["scope:6:1:6:46".to_string()]);
    for (sp, ty, own) in [
        ((4, 22, 23), "&Foo", Some("app::Foo")),
        ((4, 35, 36), "&mut Foo", Some("app::Foo")),
        ((4, 50, 52), "Foo<'a>", Some("app::Foo")),
        ((5, 22, 23), "Wrapper<i32>", Some("app::Wrapper")),
        ((5, 39, 40), "Option<Foo>", Some("core::option::Option")),
        ((5, 64, 65), "Box<Foo>", Some("alloc::boxed::Box")),
        ((5, 86, 87), "u32", Some("u32")),
        ((6, 37, 38), "T", Some("app::BoundTrait")),
    ] {
        add_ty(&mut w, app_s, sp, ty, own);
    }
    save_ridx(&mut w, root.join("02_app.ridx"));

    let ws = WorkspaceIndex::open(&root).unwrap();
    let uris = (format!("file://{app_s}"), format!("file://{core_s}"), format!("file://{alloc_s}"));
    (root, ws, uris.0, uris.1, uris.2)
}

fn assert_loc(res: &Value, expected_uri: &str, line: u64, col: u64) {
    // The file each names: on Windows the answer is `file:///C:/...`, the fixture's `file://C:\...`.
    let file = |uri: &str| crate::lsp::protocol::uri_to_path(uri).replace('\\', "/");
    assert_eq!(file(res["uri"].as_str().unwrap()), file(expected_uri));
    assert_eq!(res["range"]["start"]["line"].as_u64().unwrap(), line);
    assert_eq!(res["range"]["start"]["character"].as_u64().unwrap(), col);
}

#[test]
fn test_typedef_shared_ref_foo() {
    let (root, ws, app_uri, _, _) = setup_compound_workspace();
    assert_loc(&query(&ws, &app_uri, 3, 21), &app_uri, 0, 11);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_typedef_mut_ref_foo() {
    let (root, ws, app_uri, _, _) = setup_compound_workspace();
    assert_loc(&query(&ws, &app_uri, 3, 34), &app_uri, 0, 11);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_typedef_lifetime_generic_foo() {
    let (root, ws, app_uri, _, _) = setup_compound_workspace();
    assert_loc(&query(&ws, &app_uri, 3, 49), &app_uri, 0, 11);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_typedef_wrapper_i32_resolves_outer_wrapper() {
    let (root, ws, app_uri, _, _) = setup_compound_workspace();
    assert_loc(&query(&ws, &app_uri, 4, 21), &app_uri, 1, 11);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_typedef_option_foo_resolves_outer_option() {
    let (root, ws, app_uri, core_uri, _) = setup_compound_workspace();
    assert_loc(&query(&ws, &app_uri, 4, 38), &core_uri, 0, 9);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_typedef_box_foo_resolves_outer_box() {
    let (root, ws, app_uri, _, alloc_uri) = setup_compound_workspace();
    assert_loc(&query(&ws, &app_uri, 4, 63), &alloc_uri, 0, 11);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_typedef_primitive_u32_returns_null() {
    let (root, ws, app_uri, _, _) = setup_compound_workspace();
    assert_eq!(query(&ws, &app_uri, 4, 85), Value::Null);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_typedef_ty_param_t_resolves_to_declaration() {
    let (root, ws, app_uri, _, _) = setup_compound_workspace();
    assert_loc(&query(&ws, &app_uri, 5, 36), &app_uri, 5, 21);
    let _ = fs::remove_dir_all(&root);
}

fn write_src(root: &Path, rel: &str, content: &str) -> String {
    let p = root.join(rel);
    let _ = fs::create_dir_all(p.parent().unwrap());
    fs::write(&p, content).unwrap();
    p.to_str().unwrap().to_string()
}

#[test]
fn test_typedef_multi_crate_same_name_config_uses_owner_path() {
    let root = next_temp_dir("typedef_multi_config");
    let a_s = write_src(&root, "crate_a/src/lib.rs", "pub struct Config;\n");
    let b_s = write_src(&root, "crate_b/src/lib.rs", "// crate_b\npub struct Config;\n");
    let c_src = "pub fn init(cfg: crate_b::Config, cfg2: crate_b::Config) {}\n";
    let c_s = write_src(&root, "crate_c/src/lib.rs", c_src);

    for (idx_f, cr, p, f_s, l) in [("00_crate_a.ridx", "crate_a", "crate_a::Config", &a_s, 1), ("01_crate_b.ridx", "crate_b", "crate_b::Config", &b_s, 2)] {
        let mut w = IndexWriter::new(cr, "0.1.0");
        w.add_definition(raw_def(1, Struct, p, f_s, (l, 12, 18)));
        save_ridx(&mut w, root.join(idx_f));
    }

    let mut wc = IndexWriter::new("crate_c", "0.1.0");
    add_ty(&mut wc, &c_s, (1, 13, 16), "Config", Some("crate_b::Config"));
    add_ty(&mut wc, &c_s, (1, 35, 39), "crate_b::Config", None);
    save_ridx(&mut wc, root.join("02_crate_c.ridx"));

    let ws = WorkspaceIndex::open(&root).unwrap();
    let (c_uri, b_uri) = (format!("file://{c_s}"), format!("file://{b_s}"));
    for col in [12, 34] {
        assert_loc(&query(&ws, &c_uri, 0, col), &b_uri, 1, 11);
    }
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_typedef_enclosing_macro_and_raw_string_return_null() {
    let root = next_temp_dir("typedef_enclosing_macro");
    let code = "pub struct TestDescAndFn;\npub struct Lint { pub desc: &'static str }\n#[tokio::test]\npub fn make_lint() -> Lint {\n    Lint {\n        desc: r##\"clippy::should_panic_without_expect\"##,\n    }\n}\n";
    let f_s = write_src(&root, "src/lib.rs", code);

    let mut w = IndexWriter::new("macro_test", "0.1.0");
    let td = "macro_test::TestDescAndFn";
    w.add_definition(raw_def(1, Struct, td, &f_s, (1, 12, 25)));
    w.add_definition(raw_def(2, Struct, "macro_test::Lint", &f_s, (2, 12, 16)));
    add_ref(&mut w, &f_s, (3, 1, 3, 15), (1, td, 1, 12), Struct, true);
    let ty_path = "macro_test::Lint";
    add_ty_span(&mut w, &f_s, (5, 5, 7, 6), ty_path, Some(ty_path));
    save_ridx(&mut w, root.join("00_macro_test.ridx"));

    let ws = WorkspaceIndex::open(&root).unwrap();
    let uri = format!("file://{f_s}");
    assert_eq!(query(&ws, &uri, 2, 9), Value::Null);
    assert_eq!(query(&ws, &uri, 5, 28), Value::Null);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_typedef_qualified_path_and_self_override_enclosing_result() {
    let root = next_temp_dir("typedef_qual_path");
    let code = "pub enum Result<T, E> { Ok(T), Err(E) }\npub struct ColorModel(pub u8);\npub enum MyError { Missing }\npub struct A;\npub(in crate) use ColorModel as Cm;\nimpl MyError {\n    pub fn check(&self, a: A) -> Result<(), MyError> {\n        let _ = ColorModel::BC6H;\n        Err(Self::Missing)\n    }\n}\n";
    let f_s = write_src(&root, "src/lib.rs", code);

    let mut w = IndexWriter::new("stream_test", "0.1.0");
    for (id, k, p, sp) in [
        (1, Enum, "stream_test::Result", (1, 10, 16)),
        (2, Struct, "stream_test::ColorModel", (2, 12, 22)),
        (3, Enum, "stream_test::MyError", (3, 10, 17)),
        (4, Struct, "stream_test::A", (4, 12, 13)),
        (5, AssocFn, "stream_test::MyError::check", (7, 12, 17)),
    ] {
        w.add_definition(raw_def(id, k, p, &f_s, sp));
    }
    let cm_tgt = (2, "stream_test::ColorModel", 2, 12);
    add_ref(&mut w, &f_s, (5, 19, 5, 29), cm_tgt, Struct, false);
    add_ref(&mut w, &f_s, (8, 17, 8, 27), cm_tgt, Struct, false);
    add_ty(&mut w, &f_s, (7, 25, 26), "A", None);
    let (res_ty, res_own) = ("stream_test::Result<(), stream_test::MyError>", Some("stream_test::Result"));
    add_ty_span(&mut w, &f_s, (7, 54, 10, 6), res_ty, res_own);
    save_ridx(&mut w, root.join("00_stream_test.ridx"));

    let ws = WorkspaceIndex::open(&root).unwrap();
    let uri = format!("file://{f_s}");
    assert_eq!(query(&ws, &uri, 4, 20), Value::Null);
    assert_loc(&query(&ws, &uri, 6, 24), &uri, 3, 11);
    assert_loc(&query(&ws, &uri, 7, 28), &uri, 1, 11);
    assert_loc(&query(&ws, &uri, 8, 18), &uri, 2, 9);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_typedef_homonymous_crate_prefers_reachable_extern_unit() {
    let root = next_temp_dir("typedef_dep_unit");
    let lib_s = write_src(&root, "lib_arena/src/lib.rs", "pub struct Arena;\n");
    let reg_s = write_src(&root, "reg_arena/src/lib.rs", "// registry\npub struct Arena;\n");
    let app_s = write_src(&root, "app/src/lib.rs", "pub fn use_arena(a: la_arena::Arena) {}\n");
    let deps_dir = root.join(".deps");
    let _ = fs::create_dir_all(&deps_dir);

    const M_LIB: &str = r#"{"name":"la_arena","unit_key":"uk_lib_arena","is_dependency":false}"#;
    const M_REG: &str = r#"{"name":"la_arena","unit_key":"uk_reg_arena","is_dependency":true}"#;
    const M_APP: &str = r#"{"name":"app","unit_key":"uk_app","is_dependency":false,"extern_units":{"la_arena":"uk_reg_arena"}}"#;
    for (stem, ver, uk, f_s, l, m_json) in [("00_la_arena_lib", "0.3.0", "uk_lib_arena", &lib_s, 1, M_LIB), ("01_la_arena_reg", "0.3.1", "uk_reg_arena", &reg_s, 2, M_REG)] {
        let mut w = IndexWriter::new("la_arena", ver);
        w.set_unit_key(uk);
        w.add_definition(raw_def(1, Struct, "la_arena::Arena", f_s, (l, 12, 17)));
        save_ridx(&mut w, root.join(format!("{stem}.ridx")));
        fs::write(deps_dir.join(format!("{stem}.json")), m_json).unwrap();
    }
    let mut w_app = IndexWriter::new("app", "0.1.0");
    w_app.set_unit_key("uk_app");
    let ar = "la_arena::Arena";
    add_ty(&mut w_app, &app_s, (1, 18, 19), ar, Some(ar));
    save_ridx(&mut w_app, root.join("02_app.ridx"));
    fs::write(deps_dir.join("02_app.json"), M_APP).unwrap();

    let ws = WorkspaceIndex::open(&root).unwrap();
    let (app_uri, reg_uri) = (format!("file://{app_s}"), format!("file://{reg_s}"));
    assert_loc(&query(&ws, &app_uri, 0, 17), &reg_uri, 1, 11);
    let _ = fs::remove_dir_all(&root);
}
