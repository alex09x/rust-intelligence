/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use super::rename_general_tests::{fresh_temp_dir, open_manifest_ws, raw_def};
use crate::lsp::editing::handle_rename;
use crate::lsp::protocol::{path_to_uri, PositionConverter};
use crate::overlay::OverlayManager;
use ridx_format::{DefKind, IndexWriter};
use serde_json::json;
use std::fs;

#[test]
fn test_general_rule_u4_pub_crate_visibility_and_generic_inherent_owner() {
    let dir = fresh_temp_dir("gen_u4");
    for sub in ["src", "examples", ".ridx"] {
        fs::create_dir_all(dir.join(sub)).unwrap();
    }
    fs::write(dir.join("Cargo.toml"), "[package]\nname = \"vis_pkg\"\nversion = \"0.1.0\"\nedition = \"2021\"\n").unwrap();
    let lib_rs = dir.join("src/lib.rs");
    let lib_str = lib_rs.to_str().unwrap();
    fs::write(
        &lib_rs,
        "pub struct SecretBox;\nimpl SecretBox { pub(crate) fn fallback_val(&self) {} }\npub struct PublicHolder<'a>(&'a str);\nimpl<'a> PublicHolder<'a> { pub fn fallback_val(self) {} }\n",
    )
    .unwrap();
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

#[test]
fn test_pm_enum_semantic_reexport_prefix_without_error_literal() {
    let dir = fresh_temp_dir("pm_enum_prefix");
    for sub in ["alpha_core/src/diagnostics", "alpha_facade/src", "alpha_macros/src", ".ridx"] {
        fs::create_dir_all(dir.join(sub)).unwrap();
    }
    fs::write(dir.join("Cargo.toml"), "[workspace]\nmembers = [\"alpha_core\", \"alpha_facade\", \"alpha_macros\"]\nresolver = \"2\"\n").unwrap();
    fs::write(dir.join("alpha_core/Cargo.toml"), "[package]\nname = \"alpha_core\"\nversion = \"0.1.0\"\nedition = \"2021\"\n").unwrap();
    fs::write(
        dir.join("alpha_facade/Cargo.toml"),
        "[package]\nname = \"alpha_facade\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[dependencies]\nalpha_core = { path = \"../alpha_core\" }\nalpha_macros = { path = \"../alpha_macros\" }\n",
    )
    .unwrap();
    fs::write(dir.join("alpha_macros/Cargo.toml"), "[package]\nname = \"alpha_macros\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[lib]\nproc-macro = true\n").unwrap();

    let codes_rs = dir.join("alpha_core/src/diagnostics/codes.rs");
    fs::write(&codes_rs, "pub enum ProblemCode {\n    Fatal,\n}\n").unwrap();
    let diag_mod = dir.join("alpha_core/src/diagnostics/mod.rs");
    fs::write(&diag_mod, "mod codes;\npub use self::codes::ProblemCode;\n").unwrap();
    let core_lib = dir.join("alpha_core/src/lib.rs");
    fs::write(&core_lib, "pub mod diagnostics;\n").unwrap();
    let facade_lib = dir.join("alpha_facade/src/lib.rs");
    fs::write(&facade_lib, "pub use alpha_core::*;\n").unwrap();

    let pm_lib = dir.join("alpha_macros/src/lib.rs");
    fs::write(
        &pm_lib,
        concat!(
            "macro_rules! quote { ($($t:tt)*) => { () }; }\n",
            "pub fn expand() {\n",
            "    let _ = quote!(alpha_facade::diagnostics::ProblemCode::Fatal);\n",
            "    let _ = quote!(#krate::diagnostics::ProblemCode::Fatal);\n",
            "    let _ = quote!(unrelated_ns::ProblemCode::Fatal);\n",
            "}\n",
        ),
    )
    .unwrap();

    let ws_canon = dir.canonicalize().unwrap();
    let codes_abs = codes_rs.canonicalize().unwrap();
    let pm_abs = pm_lib.canonicalize().unwrap();

    let idx_dir = dir.join(".ridx");
    let mut wc = IndexWriter::new("alpha_core", "0.1.0");
    wc.set_unit_key("alpha_core_lib");
    wc.add_definition(raw_def(1, DefKind::Mod, 0, "diagnostics", "alpha_core::diagnostics", None, "alpha_core/src/lib.rs", 1, 9, 1, 20));
    wc.add_definition(raw_def(2, DefKind::Mod, 1, "codes", "alpha_core::diagnostics::codes", None, "alpha_core/src/diagnostics/mod.rs", 1, 5, 1, 10));
    wc.add_definition(raw_def(3, DefKind::Enum, 0, "ProblemCode", "alpha_core::diagnostics::codes::ProblemCode", None, "alpha_core/src/diagnostics/codes.rs", 1, 10, 1, 21));
    wc.write_to(&mut fs::File::create(idx_dir.join("alpha_core.ridx")).unwrap()).unwrap();

    let mut wf = IndexWriter::new("alpha_facade", "0.1.0");
    wf.set_unit_key("alpha_facade_lib");
    wf.add_definition(raw_def(1, DefKind::Mod, 0, "alpha_facade", "alpha_facade", None, "alpha_facade/src/lib.rs", 1, 1, 1, 2));
    wf.write_to(&mut fs::File::create(idx_dir.join("alpha_facade.ridx")).unwrap()).unwrap();

    let mut wm = IndexWriter::new("alpha_macros", "0.1.0");
    wm.set_unit_key("alpha_macros_lib");
    wm.add_definition(raw_def(1, DefKind::Fn, 0, "expand", "alpha_macros::expand", Some("pub fn expand()"), "alpha_macros/src/lib.rs", 2, 8, 2, 14));
    wm.write_to(&mut fs::File::create(idx_dir.join("alpha_macros.ridx")).unwrap()).unwrap();

    let ws = open_manifest_ws(&idx_dir, &ws_canon);
    let (codes_uri, pm_uri) = (path_to_uri(codes_abs.to_str().unwrap()), path_to_uri(pm_abs.to_str().unwrap()));
    let res = handle_rename(
        &json!({ "params": { "textDocument": { "uri": &codes_uri }, "position": { "line": 0, "character": 12 }, "newName": "IssueSeverity" } }),
        Some(&ws),
        &OverlayManager::new(),
        &mut PositionConverter::new(),
    )
    .unwrap();

    let pm_edits = res["changes"].get(&pm_uri).and_then(|v| v.as_array()).expect("proc-macro file should receive semantic enum edits");
    let edited_lines: Vec<u64> = pm_edits.iter().filter_map(|e| e["range"]["start"]["line"].as_u64()).collect();
    assert_eq!(edited_lines, vec![2, 3], "only resolved re-export prefixes (lines 2 and 3) must be updated, not unrelated_ns on line 4: {pm_edits:?}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_unindexed_owner_resolved_via_arbitrary_factory_signatures() {
    let dir = fresh_temp_dir("factory_sig");
    for sub in ["src", "examples", ".ridx"] {
        fs::create_dir_all(dir.join(sub)).unwrap();
    }
    fs::write(dir.join("Cargo.toml"), "[package]\nname = \"net_pipe\"\nversion = \"0.1.0\"\nedition = \"2021\"\n").unwrap();
    let lib_rs = dir.join("src/lib.rs");
    fs::write(
        &lib_rs,
        concat!(
            "pub struct SessionChannel;\n",
            "impl SessionChannel { pub fn renegotiate(&self) {} }\n",
            "pub struct GatewayMaker;\n",
            "impl GatewayMaker { pub fn assemble(&self) -> Result<SessionChannel, ()> { Ok(SessionChannel) } }\n",
            "pub struct PacketPipe;\n",
            "impl PacketPipe { pub fn flush_frames(&self) {} }\n",
            "pub fn open_endpoint() -> PacketPipe { PacketPipe }\n",
            "pub struct IdleTimer;\n",
            "impl IdleTimer { pub fn finish(&self) {} }\n",
            "impl GatewayMaker { pub fn unused_timer(&self) -> IdleTimer { IdleTimer } }\n",
            "pub struct LocalApp;\n",
            "impl LocalApp { pub fn finish(&self) {} }\n",
        ),
    )
    .unwrap();
    let ex_rs = dir.join("examples/client.rs");
    fs::write(
        &ex_rs,
        concat!(
            "use net_pipe::{GatewayMaker, LocalApp, open_endpoint};\n",
            "fn main() {\n",
            "    let ch = GatewayMaker.assemble().unwrap();\n",
            "    ch.renegotiate();\n",
            "    let p = open_endpoint();\n",
            "    p.flush_frames();\n",
            "    let unused_timer = 0;\n",
            "    let _ = unused_timer;\n",
            "    LocalApp.finish();\n",
            "}\n",
        ),
    )
    .unwrap();

    let lib_str = lib_rs.canonicalize().unwrap().to_str().unwrap().to_string();
    let ex_str = ex_rs.canonicalize().unwrap().to_str().unwrap().to_string();
    let idx_dir = dir.join(".ridx");
    let mut w = IndexWriter::new("net_pipe", "0.1.0");
    w.set_unit_key("net_pipe_lib");
    w.add_definition(raw_def(1, DefKind::Struct, 0, "SessionChannel", "net_pipe::SessionChannel", None, &lib_str, 1, 12, 1, 26));
    w.add_definition(raw_def(2, DefKind::AssocFn, 0, "renegotiate", "net_pipe::SessionChannel::renegotiate", Some("pub fn renegotiate(&self)"), &lib_str, 2, 30, 2, 41));
    w.add_definition(raw_def(3, DefKind::Struct, 0, "GatewayMaker", "net_pipe::GatewayMaker", None, &lib_str, 3, 12, 3, 24));
    w.add_definition(raw_def(
        4,
        DefKind::AssocFn,
        0,
        "assemble",
        "net_pipe::GatewayMaker::assemble",
        Some("pub fn assemble(&self) -> Result<SessionChannel, ()>"),
        &lib_str,
        4,
        28,
        4,
        36,
    ));
    w.add_definition(raw_def(5, DefKind::Struct, 0, "PacketPipe", "net_pipe::PacketPipe", None, &lib_str, 5, 12, 5, 22));
    w.add_definition(raw_def(6, DefKind::AssocFn, 0, "flush_frames", "net_pipe::PacketPipe::flush_frames", Some("pub fn flush_frames(&self)"), &lib_str, 6, 26, 6, 38));
    w.add_definition(raw_def(7, DefKind::Fn, 0, "open_endpoint", "net_pipe::open_endpoint", Some("pub fn open_endpoint() -> PacketPipe"), &lib_str, 7, 8, 7, 21));
    w.add_definition(raw_def(8, DefKind::Struct, 0, "IdleTimer", "net_pipe::IdleTimer", None, &lib_str, 8, 12, 8, 21));
    w.add_definition(raw_def(9, DefKind::AssocFn, 0, "finish", "net_pipe::IdleTimer::finish", Some("pub fn finish(&self)"), &lib_str, 9, 25, 9, 31));
    w.add_definition(raw_def(
        10,
        DefKind::AssocFn,
        0,
        "unused_timer",
        "net_pipe::GatewayMaker::unused_timer",
        Some("pub fn unused_timer(&self) -> IdleTimer"),
        &lib_str,
        10,
        28,
        10,
        40,
    ));
    w.add_definition(raw_def(11, DefKind::Struct, 0, "LocalApp", "net_pipe::LocalApp", None, &lib_str, 11, 12, 11, 20));
    w.add_definition(raw_def(12, DefKind::AssocFn, 0, "finish", "net_pipe::LocalApp::finish", Some("pub fn finish(&self)"), &lib_str, 12, 24, 12, 30));
    w.write_to(&mut fs::File::create(idx_dir.join("net_pipe.ridx")).unwrap()).unwrap();

    let ws = open_manifest_ws(&idx_dir, &dir.canonicalize().unwrap());
    let (lib_uri, ex_uri, ov, mut conv) = (path_to_uri(&lib_str), path_to_uri(&ex_str), OverlayManager::new(), PositionConverter::new());

    let res1 = handle_rename(
        &json!({ "params": { "textDocument": { "uri": &lib_uri }, "position": { "line": 1, "character": 31 }, "newName": "rekey_link" } }),
        Some(&ws),
        &ov,
        &mut conv,
    )
    .unwrap();
    assert!(res1["changes"].get(&ex_uri).is_some(), "GatewayMaker::assemble -> SessionChannel must activate example dot-call: {res1}");

    let res2 = handle_rename(
        &json!({ "params": { "textDocument": { "uri": &lib_uri }, "position": { "line": 5, "character": 27 }, "newName": "drain_frames" } }),
        Some(&ws),
        &ov,
        &mut conv,
    )
    .unwrap();
    assert!(res2["changes"].get(&ex_uri).is_some(), "open_endpoint() -> PacketPipe must activate example dot-call: {res2}");

    let res3 = handle_rename(
        &json!({ "params": { "textDocument": { "uri": &lib_uri }, "position": { "line": 8, "character": 26 }, "newName": "complete_timer" } }),
        Some(&ws),
        &ov,
        &mut conv,
    )
    .unwrap();
    assert!(res3["changes"].get(&ex_uri).is_none(), "uninvoked factory method must not falsely activate IdleTimer or trigger ambiguity on LocalApp.finish(): {res3}");
    let _ = fs::remove_dir_all(&dir);
}
