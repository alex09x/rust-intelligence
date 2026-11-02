/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use crate::lsp::editing::{handle_prepare_rename, handle_rename};
use crate::lsp::protocol::{path_to_uri, PositionConverter};
use crate::overlay::OverlayManager;
use ridx_format::{DefKind, IndexWriter, RawDefinition, RawReference, WorkspaceIndex};
use serde_json::json;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static GEN_TEST_SEQ: AtomicU64 = AtomicU64::new(0);

fn fresh_temp_dir(tag: &str) -> PathBuf {
    let n = GEN_TEST_SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_gen_rename_{tag}_{}_{n}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[allow(clippy::too_many_arguments)]
fn raw_def(id: u32, kind: DefKind, vis: u8, name: &str, path: &str, sig: Option<&str>, file: &str, sl: u32, sc: u32, el: u32, ec: u32) -> RawDefinition {
    RawDefinition {
        def_id: id,
        kind,
        visibility: vis,
        is_macro_expansion: false,
        name: name.into(),
        path: path.into(),
        signature: sig.map(String::from),
        file: file.into(),
        start_line: sl,
        start_col: sc,
        end_line: el,
        end_col: ec,
    }
}

#[allow(clippy::too_many_arguments)]
fn raw_mac_ref(file: &str, sl: u32, sc: u32, el: u32, ec: u32, def_id: u32, path: &str, unit: &str, tl: u32, tc: u32, kind: DefKind) -> RawReference {
    RawReference {
        file: file.into(),
        start_line: sl,
        start_col: sc,
        end_line: el,
        end_col: ec,
        target_def_id: Some(def_id),
        target_path: path.into(),
        target_unit: Some(unit.into()),
        target_file: Some(file.into()),
        target_line: tl,
        target_col: tc,
        kind,
        is_write: false,
        is_macro_expansion: true,
    }
}

#[test]
fn test_general_rule_m1_custom_autoref_macro_without_ampersands() {
    let dir = fresh_temp_dir("m1");
    fs::create_dir_all(dir.join("src")).unwrap();
    let lib_rs = dir.join("src/lib.rs");
    let lib_str = lib_rs.to_str().unwrap();
    let code = concat!(
        "pub trait Converter { fn convert(&self) -> i32; }\n",
        "pub struct Direct;\n",
        "impl Direct { pub fn convert(&self) -> i32 { 7 } }\n",
        "macro_rules! choose_converter {\n",
        "    ($val:expr) => { $val.convert() };\n",
        "}\n",
        "pub fn run(d: Direct) -> i32 { choose_converter!(d) }\n",
    );
    fs::write(&lib_rs, code).unwrap();

    let mut w = IndexWriter::new("pkg", "0.1.0");
    w.set_unit_key("pkg_lib");
    w.add_definition(raw_def(1, DefKind::Trait, 0, "Converter", "pkg::Converter", None, lib_str, 1, 11, 1, 20));
    w.add_definition(raw_def(2, DefKind::AssocFn, 0, "convert", "pkg::Converter::convert", Some("fn convert(&self) -> i32"), lib_str, 1, 26, 1, 33));
    w.add_definition(raw_def(3, DefKind::Struct, 0, "Direct", "pkg::Direct", None, lib_str, 2, 12, 2, 18));
    w.add_definition(raw_def(4, DefKind::AssocFn, 0, "convert", "pkg::Direct::convert", Some("pub fn convert(&self) -> i32"), lib_str, 3, 22, 3, 29));
    w.add_definition(raw_def(5, DefKind::Macro, 0, "choose_converter", "pkg::choose_converter", None, lib_str, 4, 14, 6, 2));
    w.add_reference(raw_mac_ref(lib_str, 7, 32, 7, 52, 4, "pkg::Direct::convert", "pkg_lib", 3, 22, DefKind::AssocFn));

    let ridx_path = dir.join("pkg.ridx");
    w.write_to(&mut fs::File::create(&ridx_path).unwrap()).unwrap();
    let mut ws = WorkspaceIndex::open(&ridx_path).unwrap();
    ws.set_workspace_root(Some(dir.clone()));

    let uri = path_to_uri(lib_str);
    let ov = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let prep = json!({ "params": { "textDocument": { "uri": &uri }, "position": { "line": 2, "character": 22 } } });
    let ren = json!({ "params": { "textDocument": { "uri": &uri }, "position": { "line": 2, "character": 22 }, "newName": "transform" } });

    assert!(handle_prepare_rename(&prep, Some(&ws), &ov, &mut conv).unwrap_err().message.contains("choose_converter"));
    assert!(handle_rename(&ren, Some(&ws), &ov, &mut conv).unwrap_err().message.contains("choose_converter"));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_general_rule_m2_custom_dual_binding_macro() {
    let dir = fresh_temp_dir("m2");
    fs::create_dir_all(dir.join("src")).unwrap();
    let lib_rs = dir.join("src/lib.rs");
    let lib_str = lib_rs.to_str().unwrap();
    let code = concat!(
        "pub struct Alpha;\n",
        "pub enum Kind { Alpha(Alpha) }\n",
        "macro_rules! bind_variant_from {\n",
        "    ($v:ident) => { impl From<$v> for Kind { fn from(x: $v) -> Self { Self::$v(x) } } };\n",
        "}\n",
        "bind_variant_from!(Alpha);\n",
    );
    fs::write(&lib_rs, code).unwrap();

    let mut w = IndexWriter::new("pkg", "0.1.0");
    w.set_unit_key("pkg_lib");
    w.add_definition(raw_def(1, DefKind::Struct, 0, "Alpha", "pkg::Alpha", None, lib_str, 1, 12, 1, 17));
    w.add_definition(raw_def(2, DefKind::Variant, 0, "Alpha", "pkg::Kind::Alpha", None, lib_str, 2, 17, 2, 22));
    w.add_reference(raw_mac_ref(lib_str, 6, 20, 6, 25, 1, "pkg::Alpha", "pkg_lib", 1, 12, DefKind::Struct));
    w.add_reference(raw_mac_ref(lib_str, 6, 20, 6, 25, 2, "pkg::Kind::Alpha", "pkg_lib", 2, 17, DefKind::Variant));

    let ridx_path = dir.join("pkg.ridx");
    w.write_to(&mut fs::File::create(&ridx_path).unwrap()).unwrap();
    let mut ws = WorkspaceIndex::open(&ridx_path).unwrap();
    ws.set_workspace_root(Some(dir.clone()));

    let uri = path_to_uri(lib_str);
    let ov = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let prep = json!({ "params": { "textDocument": { "uri": &uri }, "position": { "line": 0, "character": 12 } } });
    let ren = json!({ "params": { "textDocument": { "uri": &uri }, "position": { "line": 0, "character": 12 }, "newName": "Omega" } });

    assert!(handle_prepare_rename(&prep, Some(&ws), &ov, &mut conv).unwrap_err().message.contains("binds to multiple definitions"));
    assert!(handle_rename(&ren, Some(&ws), &ov, &mut conv).unwrap_err().message.contains("binds to multiple definitions"));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_cargo_metadata_main_entrypoint_vs_custom_snapshot_dir() {
    let dir = fresh_temp_dir("cargo_main");
    fs::create_dir_all(dir.join("src/bin")).unwrap();
    fs::create_dir_all(dir.join("examples")).unwrap();
    fs::create_dir_all(dir.join("custom_compile_snapshots")).unwrap();

    fs::write(
        dir.join("Cargo.toml"),
        concat!(
            "[package]\nname = \"pkg\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n",
            "[[bin]]\nname = \"real_cli\"\npath = \"src/bin/real_cli.rs\"\n\n",
            "[[example]]\nname = \"real_demo\"\npath = \"examples/real_demo.rs\"\n",
        ),
    )
    .unwrap();
    fs::write(dir.join("src/lib.rs"), "pub fn helper() {}\n").unwrap();
    let bin_rs = dir.join("src/bin/real_cli.rs");
    let ex_rs = dir.join("examples/real_demo.rs");
    let snap_rs = dir.join("custom_compile_snapshots/case_ok.rs");
    fs::write(&bin_rs, "fn main() {}\n").unwrap();
    fs::write(&ex_rs, "fn main() {}\n").unwrap();
    fs::write(&snap_rs, "fn main() {\n    main();\n}\n").unwrap();

    let mut w = IndexWriter::new("real_cli", "0.1.0");
    w.set_unit_key("real_cli_bin");
    let bin_str = bin_rs.to_str().unwrap();
    w.add_definition(raw_def(1, DefKind::Fn, 0, "main", "real_cli::main", Some("fn main()"), bin_str, 1, 4, 1, 8));
    let ridx_path = dir.join("real_cli.ridx");
    w.write_to(&mut fs::File::create(&ridx_path).unwrap()).unwrap();
    let mut ws = WorkspaceIndex::open(&ridx_path).unwrap();
    ws.set_workspace_root(Some(dir.clone()));

    let ov = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let bin_uri = path_to_uri(bin_str);
    let bin_req = json!({ "params": { "textDocument": { "uri": &bin_uri }, "position": { "line": 0, "character": 4 }, "newName": "entry" } });
    assert!(handle_prepare_rename(&bin_req, Some(&ws), &ov, &mut conv).unwrap_err().message.contains("real_cli"));
    assert!(handle_rename(&bin_req, Some(&ws), &ov, &mut conv).unwrap_err().message.contains("real_cli"));

    let ex_uri = path_to_uri(ex_rs.to_str().unwrap());
    let ex_req = json!({ "params": { "textDocument": { "uri": &ex_uri }, "position": { "line": 0, "character": 4 }, "newName": "entry" } });
    assert!(handle_prepare_rename(&ex_req, Some(&ws), &ov, &mut conv).unwrap_err().message.contains("real_demo"));
    assert!(handle_rename(&ex_req, Some(&ws), &ov, &mut conv).unwrap_err().message.contains("real_demo"));

    let snap_uri = path_to_uri(snap_rs.to_str().unwrap());
    let snap_req = json!({ "params": { "textDocument": { "uri": &snap_uri }, "position": { "line": 0, "character": 4 }, "newName": "entry" } });
    assert!(handle_prepare_rename(&snap_req, Some(&ws), &ov, &mut conv).is_ok());
    let snap_res = handle_rename(&snap_req, Some(&ws), &ov, &mut conv).expect("snapshot fn main is not a Cargo entrypoint");
    assert_eq!(snap_res["changes"][&snap_uri].as_array().unwrap().len(), 2);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_unindexed_cargo_targets_semantic_resolution_shadowing_and_ambiguity() {
    let dir = fresh_temp_dir("unindexed_sem");
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::create_dir_all(dir.join("examples")).unwrap();

    fs::write(
        dir.join("Cargo.toml"),
        concat!(
            "[package]\nname = \"pkg\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n",
            "[[example]]\nname = \"short_zero\"\npath = \"examples/short_zero.rs\"\n\n",
            "[[example]]\nname = \"imported_cube\"\npath = \"examples/imported_cube.rs\"\n\n",
            "[[example]]\nname = \"shadowed_cube\"\npath = \"examples/shadowed_cube.rs\"\n\n",
            "[[example]]\nname = \"ambig_step\"\npath = \"examples/ambig_step.rs\"\n",
        ),
    )
    .unwrap();

    let lib_rs = dir.join("src/lib.rs");
    let lib_str = lib_rs.to_str().unwrap();
    let lib_code = concat!(
        "pub struct Runner;\n",
        "impl Runner { pub fn run(&self) {} }\n",
        "pub struct Cube(pub f32);\n",
        "pub mod prelude { pub use crate::Cube; }\n",
        "pub struct Alpha;\n",
        "impl Alpha { pub fn step(&self) {} }\n",
        "pub struct Beta;\n",
        "impl Beta { pub fn step(&self) {} }\n",
    );
    fs::write(&lib_rs, lib_code).unwrap();

    let ex_short = dir.join("examples/short_zero.rs");
    let ex_imp_cube = dir.join("examples/imported_cube.rs");
    let ex_shd_cube = dir.join("examples/shadowed_cube.rs");
    let ex_ambig = dir.join("examples/ambig_step.rs");
    fs::write(&ex_short, "use pkg::Runner;\nfn main() { let r = Runner; r.run(); }\n").unwrap();
    fs::write(&ex_imp_cube, "use pkg::prelude::*;\nfn main() { let _ = Cube(1.0); }\n").unwrap();
    fs::write(&ex_shd_cube, "use pkg::prelude::*;\nstruct Cube(f32);\nfn main() { let _ = Cube(1.0); }\n").unwrap();
    fs::write(&ex_ambig, "use pkg::{Alpha, Beta};\nfn main() { let a = Alpha; let b = Beta; a.step(); b.step(); }\n").unwrap();

    let mut w = IndexWriter::new("pkg", "0.1.0");
    w.set_unit_key("pkg_lib");
    w.add_definition(raw_def(1, DefKind::Struct, 0, "Runner", "pkg::Runner", None, lib_str, 1, 12, 1, 18));
    w.add_definition(raw_def(2, DefKind::AssocFn, 0, "run", "pkg::Runner::run", Some("pub fn run(&self)"), lib_str, 2, 22, 2, 25));
    w.add_definition(raw_def(3, DefKind::Struct, 0, "Cube", "pkg::Cube", None, lib_str, 3, 12, 3, 16));
    w.add_definition(raw_def(4, DefKind::Struct, 0x02, "Cube", "pkg::prelude::Cube", None, lib_str, 4, 34, 4, 38));
    w.add_definition(raw_def(5, DefKind::Struct, 0, "Alpha", "pkg::Alpha", None, lib_str, 5, 12, 5, 17));
    w.add_definition(raw_def(6, DefKind::AssocFn, 0, "step", "pkg::Alpha::step", Some("pub fn step(&self)"), lib_str, 6, 21, 6, 25));
    w.add_definition(raw_def(7, DefKind::Struct, 0, "Beta", "pkg::Beta", None, lib_str, 7, 12, 7, 16));
    w.add_definition(raw_def(8, DefKind::AssocFn, 0, "step", "pkg::Beta::step", Some("pub fn step(&self)"), lib_str, 8, 20, 8, 24));

    let ridx_path = dir.join("pkg.ridx");
    w.write_to(&mut fs::File::create(&ridx_path).unwrap()).unwrap();
    let mut ws = WorkspaceIndex::open(&ridx_path).unwrap();
    ws.set_workspace_root(Some(dir.clone()));

    let lib_uri = path_to_uri(lib_str);
    let ov = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let run_req = json!({ "params": { "textDocument": { "uri": &lib_uri }, "position": { "line": 1, "character": 22 }, "newName": "execute" } });
    let run_res = handle_rename(&run_req, Some(&ws), &ov, &mut conv).expect("3-char method with 0 indexed refs renames cleanly");
    let short_uri = path_to_uri(&ex_short.canonicalize().unwrap().to_string_lossy());
    assert!(run_res["changes"].get(&short_uri).is_some(), "expected edit in short_zero.rs: {run_res}");

    let cube_req = json!({ "params": { "textDocument": { "uri": &lib_uri }, "position": { "line": 2, "character": 12 }, "newName": "Voxel" } });
    let cube_res = handle_rename(&cube_req, Some(&ws), &ov, &mut conv).expect("Cube renames cleanly");
    let imp_uri = path_to_uri(&ex_imp_cube.canonicalize().unwrap().to_string_lossy());
    let shd_uri = path_to_uri(&ex_shd_cube.canonicalize().unwrap().to_string_lossy());
    assert!(cube_res["changes"].get(&imp_uri).is_some(), "expected edit in imported_cube.rs");
    assert!(cube_res["changes"].get(&shd_uri).is_none(), "shadowed_cube.rs must not be edited");

    let step_req = json!({ "params": { "textDocument": { "uri": &lib_uri }, "position": { "line": 5, "character": 21 }, "newName": "advance" } });
    assert!(handle_prepare_rename(&step_req, Some(&ws), &ov, &mut conv).unwrap_err().message.contains("ambig_step.rs"));
    assert!(handle_rename(&step_req, Some(&ws), &ov, &mut conv).unwrap_err().message.contains("ambig_step.rs"));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_general_rule_p1_custom_proc_macro_string_coupled() {
    let dir = fresh_temp_dir("p1");
    fs::create_dir_all(dir.join("codegen/src")).unwrap();
    fs::create_dir_all(dir.join("app/src")).unwrap();

    fs::write(dir.join("Cargo.toml"), "[workspace]\nmembers = [\"codegen\", \"app\"]\nresolver = \"2\"\n").unwrap();
    fs::write(dir.join("codegen/Cargo.toml"), "[package]\nname = \"my_codegen_helper\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[lib]\nproc-macro = true\n").unwrap();
    fs::write(dir.join("app/Cargo.toml"), "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n").unwrap();
    fs::write(
        dir.join("codegen/src/lib.rs"),
        "macro_rules! fake_quote { ($($t:tt)*) => {}; }\npub fn gen_call() -> &'static str {\n    let m = \"dispatch_event\";\n    fake_quote!(val.#m());\n    m\n}\n",
    )
    .unwrap();
    let app_lib = dir.join("app/src/lib.rs");
    let app_str = app_lib.to_str().unwrap();
    fs::write(&app_lib, "pub struct Handler;\nimpl Handler { pub fn dispatch_event(&self) {} }\npub fn invoke(h: &Handler) { h.dispatch_event(); }\n").unwrap();

    let mut w = IndexWriter::new("app", "0.1.0");
    w.set_unit_key("app_lib");
    w.add_definition(raw_def(1, DefKind::Struct, 0, "Handler", "app::Handler", None, app_str, 1, 12, 1, 19));
    w.add_definition(raw_def(2, DefKind::AssocFn, 0, "dispatch_event", "app::Handler::dispatch_event", Some("pub fn dispatch_event(&self)"), app_str, 2, 23, 2, 37));
    w.add_reference(raw_mac_ref(app_str, 3, 1, 3, 50, 2, "app::Handler::dispatch_event", "app_lib", 2, 23, DefKind::AssocFn));
    let ridx_path = dir.join("app.ridx");
    w.write_to(&mut fs::File::create(&ridx_path).unwrap()).unwrap();
    let mut ws = WorkspaceIndex::open(&ridx_path).unwrap();
    ws.set_workspace_root(Some(dir.clone()));

    let app_uri = path_to_uri(app_str);
    let ov = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let req = json!({ "params": { "textDocument": { "uri": &app_uri }, "position": { "line": 1, "character": 23 }, "newName": "emit_event" } });

    assert!(handle_prepare_rename(&req, Some(&ws), &ov, &mut conv).unwrap_err().message.contains("my_codegen_helper"));
    assert!(handle_rename(&req, Some(&ws), &ov, &mut conv).unwrap_err().message.contains("my_codegen_helper"));

    let _ = fs::remove_dir_all(&dir);
}

fn open_manifest_ws(idx_dir: &std::path::Path, ws_root: &std::path::Path) -> WorkspaceIndex {
    let manifest = json!({
        "version": "2.0",
        "format_version": ridx_format::CURRENT_FORMAT_VERSION,
        "extractor_version": ridx_format::EXTRACTOR_VERSION,
        "generation": 1,
        "workspace_root": ws_root.to_str().unwrap()
    });
    fs::write(idx_dir.join("variant_manifest.json"), manifest.to_string()).unwrap();
    WorkspaceIndex::open(idx_dir).unwrap()
}

#[test]
fn test_general_rule_u1_manifest_json_and_umbrella_prelude_example() {
    let dir = fresh_temp_dir("u1_manifest_prelude");
    for sub in ["sub_mod/src", "src", "examples"] {
        fs::create_dir_all(dir.join(sub)).unwrap();
    }
    fs::write(dir.join("Cargo.toml"), "[workspace]\nmembers = [\".\", \"sub_mod\"]\n[package]\nname = \"umbrella_pkg\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[dependencies]\nsub_mod = { path = \"sub_mod\" }\n").unwrap();
    fs::write(dir.join("sub_mod/Cargo.toml"), "[package]\nname = \"sub_mod\"\nversion = \"0.1.0\"\nedition = \"2021\"\n").unwrap();
    let sub_lib = dir.join("sub_mod/src/lib.rs");
    let sub_str = sub_lib.to_str().unwrap();
    fs::write(&sub_lib, "pub mod items { pub struct Gizmo; impl Gizmo { pub fn spin_fast(&self) {} } }\npub mod prelude { pub use crate::items::*; }\n").unwrap();
    fs::write(dir.join("src/lib.rs"), "pub mod prelude { pub use sub_mod::prelude::*; }\n").unwrap();
    let ex_file = dir.join("examples/demo.rs");
    fs::write(&ex_file, "use umbrella_pkg::prelude::*;\nfn main() { let g = Gizmo; g.spin_fast(); }\n").unwrap();

    let idx_dir = dir.join(".ridx");
    fs::create_dir_all(&idx_dir).unwrap();
    let mut w = IndexWriter::new("sub_mod", "0.1.0");
    w.set_unit_key("sub_lib");
    w.add_definition(raw_def(1, DefKind::Struct, 0, "Gizmo", "sub_mod::items::Gizmo", None, sub_str, 1, 28, 1, 33));
    w.add_definition(raw_def(2, DefKind::AssocFn, 0, "spin_fast", "sub_mod::items::Gizmo::spin_fast", Some("pub fn spin_fast(&self)"), sub_str, 1, 55, 1, 64));
    w.add_definition(raw_def(3, DefKind::Mod, 0, "prelude", "sub_mod::prelude", None, sub_str, 2, 9, 2, 16));
    w.write_to(&mut fs::File::create(idx_dir.join("sub_mod.ridx")).unwrap()).unwrap();

    let ws = open_manifest_ws(&idx_dir, &dir);
    let (sub_uri, ex_uri) = (path_to_uri(sub_str), path_to_uri(ex_file.canonicalize().unwrap().to_str().unwrap()));
    let res = handle_rename(
        &json!({ "params": { "textDocument": { "uri": &sub_uri }, "position": { "line": 0, "character": 56 }, "newName": "rotate_fast" } }),
        Some(&ws),
        &OverlayManager::new(),
        &mut PositionConverter::new(),
    )
    .unwrap();
    assert!(res["changes"].get(&ex_uri).is_some(), "umbrella prelude example must receive rename edit: {res}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_general_rule_u4_pub_crate_visibility_and_generic_inherent_owner() {
    let dir = fresh_temp_dir("gen_u4");
    for sub in ["src", "examples", ".ridx"] {
        fs::create_dir_all(dir.join(sub)).unwrap();
    }
    fs::write(dir.join("Cargo.toml"), "[package]\nname = \"vis_pkg\"\nversion = \"0.1.0\"\nedition = \"2021\"\n").unwrap();
    let lib_rs = dir.join("src/lib.rs");
    let lib_str = lib_rs.to_str().unwrap();
    fs::write(&lib_rs, "pub struct SecretBox;\nimpl SecretBox { pub(crate) fn fallback_val(&self) {} }\npub struct PublicHolder<'a>(&'a str);\nimpl<'a> PublicHolder<'a> { pub fn fallback_val(self) {} }\n").unwrap();
    let ex_file = dir.join("examples/use_holder.rs");
    fs::write(&ex_file, "use vis_pkg::{PublicHolder, SecretBox};\nfn main() { let _ = SecretBox; let h: PublicHolder = PublicHolder(\"x\"); h.fallback_val(); }\n").unwrap();

    let idx_dir = dir.join(".ridx");
    let mut w = IndexWriter::new("vis_pkg", "0.1.0");
    w.set_unit_key("vis_lib");
    w.add_definition(raw_def(1, DefKind::Struct, 0, "SecretBox", "SecretBox", None, lib_str, 1, 12, 1, 21));
    w.add_definition(raw_def(2, DefKind::AssocFn, 0, "fallback_val", "SecretBox::fallback_val", Some("pub(crate) fn fallback_val(&self)"), lib_str, 2, 32, 2, 44));
    w.add_definition(raw_def(3, DefKind::Struct, 0, "PublicHolder", "PublicHolder", None, lib_str, 3, 12, 3, 24));
    w.add_definition(raw_def(4, DefKind::AssocFn, 0, "fallback_val", "PublicHolder::<'a>::fallback_val", Some("pub fn fallback_val(self)"), lib_str, 4, 36, 4, 48));
    w.write_to(&mut fs::File::create(idx_dir.join("vis_pkg.ridx")).unwrap()).unwrap();

    let ws = open_manifest_ws(&idx_dir, &dir);
    let (lib_uri, ex_uri, ov, mut conv) = (path_to_uri(lib_str), path_to_uri(ex_file.canonicalize().unwrap().to_str().unwrap()), OverlayManager::new(), PositionConverter::new());
    let secret_res =
        handle_rename(&json!({ "params": { "textDocument": { "uri": &lib_uri }, "position": { "line": 1, "character": 33 }, "newName": "priv_val" } }), Some(&ws), &ov, &mut conv)
            .unwrap();
    assert!(secret_res["changes"].get(&ex_uri).is_none(), "pub(crate) method must not touch or refuse external example: {secret_res}");
    let pub_res =
        handle_rename(&json!({ "params": { "textDocument": { "uri": &lib_uri }, "position": { "line": 3, "character": 37 }, "newName": "into_val" } }), Some(&ws), &ov, &mut conv)
            .unwrap();
    assert!(pub_res["changes"].get(&ex_uri).is_some(), "<PublicHolder<'a>>::fallback_val must update example: {pub_res}");
    let _ = fs::remove_dir_all(&dir);
}
