/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! End-to-end compiler tests for structured `FnSigEntry` + `TyArena` extraction (1.2-G2)
//! across workspace crates, immutable dependencies, and sysroot (`core`).

use super::*;
use ridx_format::{DefKind as RDefKind, FnAbiKind, IndexReader, TyKindView, TyMutability};

fn find_def_idx<B: AsRef<[u8]>>(reader: &IndexReader<B>, path: &str) -> u32 {
    reader
        .definitions()
        .iter()
        .enumerate()
        .find_map(|(i, raw)| {
            let d = raw.from_le();
            (reader.get_str(d.path_offset) == path).then_some(i as u32)
        })
        .unwrap_or_else(|| panic!("definition `{path}` not found in `{}`", reader.crate_name()))
}

#[test]
fn test_driver_fn_sigs_unified_ws_dep_and_sysroot() {
    let _lock = DRIVER_TEST_LOCK.lock().unwrap();
    let temp_dir = std::env::temp_dir().join(format!("ridx_driver_fnsigs_{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp_dir);
    let cache_dir = temp_dir.join("cache");
    let reg_dir = temp_dir.join(".cargo/registry/src/dep_api");
    let ws_dir = temp_dir.join("ws_app");
    let out_dir = temp_dir.join("out");
    fs::create_dir_all(&reg_dir).unwrap();
    fs::create_dir_all(&ws_dir).unwrap();
    fs::create_dir_all(out_dir.join(".deps")).unwrap();

    let sysroot = std::process::Command::new("rustc").args(["--print", "sysroot"]).output().unwrap();
    let sysroot_str = String::from_utf8_lossy(&sysroot.stdout).trim().to_string();

    // 1. Compile immutable dependency `dep_api`
    let dep_src = reg_dir.join("lib.rs");
    fs::write(&dep_src, "pub struct Token(pub u64);\npub fn mint(seed: u64) -> Token { Token(seed) }\n").unwrap();
    let dep_rmeta = temp_dir.join("libdep_api.rmeta");
    let dep_args = vec![
        "rustc".to_string(),
        dep_src.to_str().unwrap().to_string(),
        "--crate-name".to_string(),
        "dep_api".to_string(),
        "--crate-type".to_string(),
        "lib".to_string(),
        "--edition=2021".to_string(),
        "--emit=metadata".to_string(),
        "-o".to_string(),
        dep_rmeta.to_str().unwrap().to_string(),
        "--sysroot".to_string(),
        sysroot_str.clone(),
    ];
    let mut dep_env = DriverEnv::for_test(&out_dir);
    dep_env.cache_dir = cache_dir.clone();
    dep_env.manifest_dir = Some(reg_dir.clone());
    assert!(run_driver_with_env(dep_args, dep_env).is_ok());

    let sidecar_path = fs::read_dir(out_dir.join(".deps"))
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.file_name().and_then(|s| s.to_str()).is_some_and(|s| s.starts_with("dep_api") && s.ends_with(".json")))
        .expect("dep_api sidecar");
    let sidecar: serde_json::Value = serde_json::from_slice(&fs::read(&sidecar_path).unwrap()).unwrap();
    let dep_unit_key = sidecar["unit_key"].as_str().unwrap().to_string();
    let dep_obj_path = sidecar["object_path"].as_str().unwrap().to_string();

    let dep_reader = IndexReader::open(&dep_obj_path).expect("open dep_api shard");
    let dep_arena = dep_reader.ty_arena();
    let mint_idx = find_def_idx(&dep_reader, "mint");
    let token_idx = find_def_idx(&dep_reader, "Token");
    let mint_sig = dep_reader.fn_sig_for_def(mint_idx).expect("dep_api::mint FnSig");
    assert_eq!(mint_sig.inputs().map(|t| dep_arena.display_ty(t)).collect::<Vec<_>>(), vec!["u64"]);
    match dep_arena.kind(mint_sig.output()) {
        TyKindView::Adt { def, .. } => {
            let dv = dep_arena.def(def).unwrap();
            assert_eq!(dv.path, "dep_api::Token");
            assert_eq!(dv.local_def_idx, Some(token_idx));
        }
        other => panic!("expected Adt(dep_api::Token), got {other:?}"),
    }

    // 2. Compile workspace crate `ws_app` against `--extern dep_api=...`
    let ws_src = ws_dir.join("lib.rs");
    let ws_code = r#"
pub trait Handler {
    type Item;
    fn handle(&self) -> Self::Item;
}

pub trait Streamer {
    fn rpitit_stream(&self) -> impl Iterator<Item = u8>;
}

pub struct Engine {
    pub count: usize,
}

impl Engine {
    pub fn push(&mut self, tok: dep_api::Token, buf: &[u8; 4]) -> Option<usize> {
        let _ = (tok, buf);
        Some(self.count)
    }

    pub async fn run_async(&self, h: &(dyn Handler<Item = u64> + Send)) -> dep_api::Token {
        dep_api::mint(h.handle())
    }
}

pub const unsafe extern "C" fn raw_op(ptr: *const u8) -> i32 {
    let _ = ptr;
    0
}

pub fn next_item<I: Iterator>(mut it: I) -> Option<I::Item> {
    it.next()
}
"#;
    fs::write(&ws_src, ws_code).unwrap();
    let ws_rmeta = temp_dir.join("libws_app.rmeta");
    let ws_args = vec![
        "rustc".to_string(),
        ws_src.to_str().unwrap().to_string(),
        "--crate-name".to_string(),
        "ws_app".to_string(),
        "--crate-type".to_string(),
        "lib".to_string(),
        "--edition=2021".to_string(),
        "--extern".to_string(),
        format!("dep_api={}", dep_rmeta.display()),
        "--emit=metadata".to_string(),
        "-o".to_string(),
        ws_rmeta.to_str().unwrap().to_string(),
        "--sysroot".to_string(),
        sysroot_str.clone(),
    ];
    let mut ws_env = DriverEnv::for_test(&out_dir);
    ws_env.cache_dir = cache_dir.clone();
    ws_env.manifest_dir = Some(ws_dir.clone());
    assert!(run_driver_with_env(ws_args, ws_env).is_ok());

    let ws_ridx = fs::read_dir(&out_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.file_name().and_then(|s| s.to_str()).is_some_and(|s| s.starts_with("ws_app") && s.ends_with(".ridx")))
        .expect("ws_app.ridx shard");
    let ws_reader = IndexReader::open(&ws_ridx).expect("open ws_app.ridx");
    // `dep_api` in `dep_api::Token` refers to the dependency's crate root.
    let crate_root_refs: Vec<(u32, String, ridx_format::DefKind)> = ws_reader
        .references()
        .iter()
        .filter(|r| r.span().start_line() == 16)
        .map(|r| (r.span().start_col(), ws_reader.get_str(r.target_path_offset()).to_string(), ridx_format::DefKind::from(r.kind)))
        .collect();
    assert!(crate_root_refs.iter().any(|(_, p, k)| p == "dep_api" && *k == ridx_format::DefKind::Mod), "crate root segment: {crate_root_refs:?}");
    let ws_arena = ws_reader.ty_arena();
    let engine_idx = find_def_idx(&ws_reader, "Engine");

    // Verify string sig retention alongside structured FnSigEntry
    let push_idx = find_def_idx(&ws_reader, "Engine::push");
    let push_def = ws_reader.definitions()[push_idx as usize].from_le();
    assert!(!ws_reader.get_str(push_def.sig_offset).is_empty(), "string DefEntry.sig must remain intact");

    let push_sig = ws_reader.fn_sig_for_def(push_idx).expect("Engine::push FnSig");
    assert!(push_sig.has_self() && !push_sig.is_async() && !push_sig.is_unsafe());
    let push_inputs: Vec<_> = push_sig.inputs().collect();
    assert_eq!(push_inputs.len(), 3);
    match ws_arena.kind(push_inputs[0]) {
        TyKindView::Ref { mutability: TyMutability::Mut, inner, .. } => match ws_arena.kind(inner) {
            TyKindView::Adt { def, .. } => {
                let dv = ws_arena.def(def).unwrap();
                assert_eq!(dv.path, "ws_app::Engine");
                assert_eq!(dv.local_def_idx, Some(engine_idx));
            }
            other => panic!("expected Engine Adt, got {other:?}"),
        },
        other => panic!("expected &mut Engine, got {other:?}"),
    }
    match ws_arena.kind(push_inputs[1]) {
        TyKindView::Adt { def, .. } => {
            let dv = ws_arena.def(def).unwrap();
            assert_eq!(dv.path, "dep_api::Token");
            assert_eq!(dv.extern_unit, Some(dep_unit_key.as_str()));
        }
        other => panic!("expected dep_api::Token, got {other:?}"),
    }
    assert_eq!(ws_arena.display_ty(push_inputs[2]), "&[u8; 4]");
    assert_eq!(ws_arena.display_ty(push_sig.output()), "Option<usize>");

    // Verify async fn -> impl Future<Output = dep_api::Token> + dyn Handler<Item = u64> + Send
    let async_idx = find_def_idx(&ws_reader, "Engine::run_async");
    let async_sig = ws_reader.fn_sig_for_def(async_idx).expect("run_async FnSig");
    assert!(async_sig.is_async() && async_sig.has_self());
    let out_str = ws_arena.display_ty(async_sig.output());
    assert!(out_str.contains("Future") && out_str.contains("Output = Token"), "async output: {out_str}");

    // Verify const unsafe extern "C" fn
    let raw_idx = find_def_idx(&ws_reader, "raw_op");
    let raw_sig = ws_reader.fn_sig_for_def(raw_idx).expect("raw_op FnSig");
    assert!(raw_sig.is_const() && raw_sig.is_unsafe() && !raw_sig.has_self());
    assert_eq!(raw_sig.abi(), FnAbiKind::C);
    assert_eq!(ws_arena.display_ty(raw_sig.output()), "i32");

    // Verify <I as Iterator>::Item projection in `next_item`
    let next_idx = find_def_idx(&ws_reader, "next_item");
    let next_sig = ws_reader.fn_sig_for_def(next_idx).expect("next_item FnSig");
    let next_out = ws_arena.display_ty(next_sig.output());
    assert!(next_out.contains("Iterator>::Item"), "projection output: {next_out}");

    // Verify RPITIT (`fn rpitit_stream(&self) -> impl Iterator<Item = u8>` — AnonAssocTy) lowers cleanly
    let rpitit_idx = find_def_idx(&ws_reader, "Streamer::rpitit_stream");
    let rpitit_sig = ws_reader.fn_sig_for_def(rpitit_idx).expect("rpitit_stream FnSig");
    let rpitit_out = ws_arena.display_ty(rpitit_sig.output());
    assert!(rpitit_out.contains("Iterator") && rpitit_out.contains("Item = u8"), "RPITIT output: {rpitit_out}");

    // 3. Verify sysroot (`core`) uses the exact same unified FnSig pipeline
    struct SysrootFnSigProbe(Option<(usize, usize)>);
    impl Callbacks for SysrootFnSigProbe {
        fn after_analysis<'tcx>(&mut self, _c: &rustc_interface::interface::Compiler, tcx: TyCtxt<'tcx>) -> Compilation {
            let (mut writer, _stats) = sysroot::extract_sysroot_crate(tcx, "core", None).expect("extract core");
            let mut bytes = Vec::new();
            writer.write_to(&mut std::io::Cursor::new(&mut bytes)).unwrap();
            let tmp = std::env::temp_dir().join(format!("ridx_core_fnsig_{}.ridx", std::process::id()));
            fs::write(&tmp, &bytes).unwrap();
            let reader = IndexReader::open(&tmp).unwrap();
            let _ = fs::remove_file(&tmp);
            let callable_defs = reader
                .definitions()
                .iter()
                .enumerate()
                .filter(|&(idx, d)| {
                    matches!(RDefKind::from(d.from_le().kind), RDefKind::Fn | RDefKind::AssocFn)
                        || reader.def_ty_for_def(idx as u32).is_some_and(|dt| dt.ctor_shape() == ridx_format::CtorShape::Tuple)
                })
                .count();
            let sig_rows = reader.fn_sigs().len();
            let str_len_idx = find_def_idx(&reader, "core::str::<impl str>::len");
            let str_len_sig = reader.fn_sig_for_def(str_len_idx).expect("str::len FnSig");
            assert!(str_len_sig.has_self() && str_len_sig.is_const());
            assert_eq!(reader.ty_arena().display_ty(str_len_sig.output()), "usize");
            self.0 = Some((callable_defs, sig_rows));
            Compilation::Stop
        }
    }
    let dummy_src = temp_dir.join("sys_probe.rs");
    fs::write(&dummy_src, "").unwrap();
    let probe_args = vec![
        "rustc".to_string(),
        dummy_src.to_str().unwrap().to_string(),
        "--crate-name".to_string(),
        "sys_probe".to_string(),
        "--crate-type".to_string(),
        "lib".to_string(),
        "--emit=metadata".to_string(),
        "-o".to_string(),
        temp_dir.join("libsys_probe.rmeta").to_str().unwrap().to_string(),
        "--sysroot".to_string(),
        sysroot_str,
    ];
    let mut probe = SysrootFnSigProbe(None);
    assert!(run_with_callbacks(&probe_args, &mut probe).is_ok());
    let (callable_defs, sig_rows) = probe.0.expect("probe ran");
    assert_eq!(callable_defs, sig_rows, "every Fn/AssocFn in core must have a FnSigEntry");

    let _ = fs::remove_dir_all(&temp_dir);
}
