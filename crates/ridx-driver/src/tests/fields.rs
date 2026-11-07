/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! End-to-end compiler tests for `FieldEntry`, `DefTyEntry`, and tuple-ctor `FnSigEntry` (1.2-G3)
//! across workspace crates, external dependencies, and sysroot (`core` + `std`).

use super::*;
use ridx_format::{CtorShape, DefKind as RDefKind, IndexReader, ItemVis};

fn find_def_idx<M: AsRef<[u8]>>(reader: &IndexReader<M>, kind: RDefKind, path_suffix: &str) -> u32 {
    reader
        .definitions()
        .iter()
        .enumerate()
        .find_map(|(idx, d)| {
            let dk = RDefKind::from(d.kind);
            let p = reader.get_str(d.path_offset());
            (dk == kind && (p == path_suffix || p.ends_with(path_suffix))).then_some(idx as u32)
        })
        .unwrap_or_else(|| panic!("missing {kind:?} ending with `{path_suffix}`"))
}

#[test]
fn test_driver_fields_ctors_ty_alias_and_consts_ws_dep_and_sysroot() {
    let _lock = DRIVER_TEST_LOCK.lock().unwrap();
    let temp_dir = std::env::temp_dir().join(format!("ridx_driver_fields_{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp_dir);
    let cache_dir = temp_dir.join("cache");
    let reg_dir = temp_dir.join(".cargo/registry/src/dep_g3");
    let ws_dir = temp_dir.join("ws_g3");
    let out_dir = temp_dir.join("out");
    fs::create_dir_all(&reg_dir).unwrap();
    fs::create_dir_all(&ws_dir).unwrap();
    fs::create_dir_all(out_dir.join(".deps")).unwrap();

    let sysroot = std::process::Command::new("rustc").args(["--print", "sysroot"]).output().unwrap();
    let sysroot_str = String::from_utf8_lossy(&sysroot.stdout).trim().to_string();

    // 1. External dependency `dep_g3`: non-exhaustive struct, tuple struct + ctor, TyAlias, Const
    let dep_src = reg_dir.join("lib.rs");
    let dep_code = r#"
#![allow(dead_code)]
#[non_exhaustive]
pub struct DepConfig<T> {
    pub open: T,
    secret: u64,
}

pub struct DepPair<A, B>(pub A, B);

pub type DepResult<T> = Result<T, &'static str>;

pub const DEP_LIMIT: usize = 1024;
"#;
    fs::write(&dep_src, dep_code).unwrap();
    let dep_rmeta = temp_dir.join("libdep_g3.rmeta");
    let dep_args = vec![
        "rustc".to_string(),
        dep_src.to_str().unwrap().to_string(),
        "--crate-name".to_string(),
        "dep_g3".to_string(),
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
    dep_env.manifest_dir = Some(reg_dir);
    assert!(run_driver_with_env(dep_args, dep_env).is_ok());

    let sidecar_path = fs::read_dir(out_dir.join(".deps"))
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.file_name().and_then(|s| s.to_str()).is_some_and(|s| s.starts_with("dep_g3") && s.ends_with(".json")))
        .expect("dep_g3 sidecar");
    let sidecar: serde_json::Value = serde_json::from_slice(&fs::read(&sidecar_path).unwrap()).unwrap();
    let dep_reader = IndexReader::open(sidecar["object_path"].as_str().unwrap()).unwrap();
    let dep_arena = dep_reader.ty_arena();

    let cfg_idx = find_def_idx(&dep_reader, RDefKind::Struct, "DepConfig");
    let cfg_dt = dep_reader.def_ty_for_def(cfg_idx).expect("DepConfig DefTy");
    assert_eq!(cfg_dt.ctor_shape(), CtorShape::Record);
    assert!(cfg_dt.is_non_exhaustive());
    assert_eq!(dep_reader.find_field(cfg_idx, "open").unwrap().vis, ItemVis::Public);
    assert_eq!(dep_reader.find_field(cfg_idx, "secret").unwrap().vis, ItemVis::Private);

    let pair_idx = find_def_idx(&dep_reader, RDefKind::Struct, "DepPair");
    let pair_dt = dep_reader.def_ty_for_def(pair_idx).expect("DepPair DefTy");
    assert_eq!(pair_dt.ctor_shape(), CtorShape::Tuple);
    let p0 = dep_reader.find_field(pair_idx, "0").expect("DepPair.0");
    let p1 = dep_reader.find_field(pair_idx, "1").expect("DepPair.1");
    assert!(p0.is_tuple_field() && p0.vis == ItemVis::Public);
    assert!(p1.is_tuple_field() && p1.vis == ItemVis::Private);
    let pair_sig = dep_reader.fn_sig_for_def(pair_idx).expect("DepPair tuple ctor FnSig");
    assert!(pair_sig.is_const());
    assert_eq!(pair_sig.inputs().len(), 2);
    assert_eq!(dep_arena.display_ty(pair_sig.output()), "DepPair<A, B>");

    let alias_idx = find_def_idx(&dep_reader, RDefKind::TyAlias, "DepResult");
    let alias_ty = dep_reader.def_ty_for_def(alias_idx).expect("DepResult DefTy").ty();
    assert_eq!(dep_arena.display_ty(alias_ty), "Result<T, &'static str>");

    let limit_idx = find_def_idx(&dep_reader, RDefKind::Const, "DEP_LIMIT");
    assert_eq!(dep_arena.display_ty(dep_reader.def_ty_for_def(limit_idx).unwrap().ty()), "usize");

    // 2. Workspace crate `ws_g3`: all 4 field visibilities, unit struct, union, enum variants, statics, assoc consts
    let ws_src = ws_dir.join("lib.rs");
    let ws_code = r#"
#![allow(dead_code)]
pub struct RootRecord {
    pub(crate) crate_vis: u16,
    priv_vis: u8,
}

pub mod outer {
    pub mod inner {
        pub struct ScopedFields {
            pub pub_f: u32,
            pub(crate) crate_f: i64,
            pub(super) super_f: bool,
            priv_f: dep_g3::DepPair<u8, u16>,
        }
    }
}

pub struct MarkerUnit;

pub union RawBits {
    pub u: u32,
    pub f: f32,
}

pub enum Event<T> {
    Click { x: i32, y: i32 },
    Key(T, bool),
    Quit,
}

pub type LocalMap<V> = dep_g3::DepResult<Event<V>>;

pub const MAX_FPS: u32 = 120;
pub static GLOBAL_NAME: &str = "ridx";
pub static mut COUNTER: u64 = 0;

pub trait HasId {
    const ID: u16;
}

impl HasId for MarkerUnit {
    const ID: u16 = 7;
}
"#;
    fs::write(&ws_src, ws_code).unwrap();
    let ws_rmeta = temp_dir.join("libws_g3.rmeta");
    let ws_args = vec![
        "rustc".to_string(),
        ws_src.to_str().unwrap().to_string(),
        "--crate-name".to_string(),
        "ws_g3".to_string(),
        "--crate-type".to_string(),
        "lib".to_string(),
        "--edition=2021".to_string(),
        "--emit=metadata".to_string(),
        "--extern".to_string(),
        format!("dep_g3={}", dep_rmeta.display()),
        "-o".to_string(),
        ws_rmeta.to_str().unwrap().to_string(),
        "--sysroot".to_string(),
        sysroot_str.clone(),
    ];
    let mut ws_env = DriverEnv::for_test(&out_dir);
    ws_env.cache_dir = cache_dir;
    ws_env.manifest_dir = Some(ws_dir);
    assert!(run_driver_with_env(ws_args, ws_env).is_ok());

    let ws_idx_path = fs::read_dir(&out_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.file_name().and_then(|s| s.to_str()).is_some_and(|s| s.starts_with("ws_g3") && s.ends_with(".ridx")))
        .expect("ws_g3.ridx");
    let ws_reader = IndexReader::open(&ws_idx_path).unwrap();
    let ws_arena = ws_reader.ty_arena();

    // Crate-root `pub(crate)` vs private field distinction
    let root_rec = find_def_idx(&ws_reader, RDefKind::Struct, "RootRecord");
    assert_eq!(ws_reader.find_field(root_rec, "crate_vis").unwrap().vis, ItemVis::Crate);
    assert_eq!(ws_reader.find_field(root_rec, "priv_vis").unwrap().vis, ItemVis::Private);

    // Submodule 4-level field visibilities
    let scoped = find_def_idx(&ws_reader, RDefKind::Struct, "outer::inner::ScopedFields");
    assert_eq!(ws_reader.find_field(scoped, "pub_f").unwrap().vis, ItemVis::Public);
    assert_eq!(ws_reader.find_field(scoped, "crate_f").unwrap().vis, ItemVis::Crate);
    assert_eq!(ws_reader.find_field(scoped, "super_f").unwrap().vis, ItemVis::Restricted);
    let priv_f = ws_reader.find_field(scoped, "priv_f").unwrap();
    assert_eq!(priv_f.vis, ItemVis::Private);
    assert_eq!(ws_arena.display_ty(priv_f.ty), "DepPair<u8, u16>");

    // Unit struct & Union
    let unit_idx = find_def_idx(&ws_reader, RDefKind::Struct, "MarkerUnit");
    assert_eq!(ws_reader.def_ty_for_def(unit_idx).unwrap().ctor_shape(), CtorShape::Unit);
    assert_eq!(ws_reader.fields_for_owner(unit_idx).len(), 0);

    let union_idx = find_def_idx(&ws_reader, RDefKind::Union, "RawBits");
    assert_eq!(ws_reader.fields_for_owner(union_idx).len(), 2);
    assert_eq!(ws_arena.display_ty(ws_reader.find_field(union_idx, "f").unwrap().ty), "f32");

    // Enum + Record / Tuple / Unit variants
    let ev_idx = find_def_idx(&ws_reader, RDefKind::Enum, "Event");
    assert_eq!(ws_reader.def_ty_for_def(ev_idx).unwrap().ctor_shape(), CtorShape::None);

    let click_idx = find_def_idx(&ws_reader, RDefKind::Variant, "Event::Click");
    let click_dt = ws_reader.def_ty_for_def(click_idx).unwrap();
    assert_eq!(click_dt.ctor_shape(), CtorShape::Record);
    assert_eq!(click_dt.parent_def_index(), Some(ev_idx));
    assert_eq!(ws_reader.fields_for_owner(click_idx).len(), 2);

    let key_idx = find_def_idx(&ws_reader, RDefKind::Variant, "Event::Key");
    let key_dt = ws_reader.def_ty_for_def(key_idx).unwrap();
    assert_eq!(key_dt.ctor_shape(), CtorShape::Tuple);
    assert_eq!(key_dt.variant_idx(), Some(1));
    assert_eq!(ws_arena.display_ty(ws_reader.find_field(key_idx, "0").unwrap().ty), "T");
    let key_sig = ws_reader.fn_sig_for_def(key_idx).expect("Event::Key tuple ctor sig");
    assert_eq!(key_sig.inputs().len(), 2);
    assert_eq!(ws_arena.display_ty(key_sig.output()), "Event<T>");

    let quit_idx = find_def_idx(&ws_reader, RDefKind::Variant, "Event::Quit");
    assert_eq!(ws_reader.def_ty_for_def(quit_idx).unwrap().ctor_shape(), CtorShape::Unit);

    // TyAlias, Const, Static, Static mut, AssocConst (trait + impl)
    let lmap_idx = find_def_idx(&ws_reader, RDefKind::TyAlias, "LocalMap");
    assert_eq!(ws_arena.display_ty(ws_reader.def_ty_for_def(lmap_idx).unwrap().ty()), "Result<Event<V>, &'static str>");

    let fps_idx = find_def_idx(&ws_reader, RDefKind::Const, "MAX_FPS");
    assert_eq!(ws_arena.display_ty(ws_reader.def_ty_for_def(fps_idx).unwrap().ty()), "u32");

    let gname_idx = find_def_idx(&ws_reader, RDefKind::Static, "GLOBAL_NAME");
    let gname_dt = ws_reader.def_ty_for_def(gname_idx).unwrap();
    assert!(!gname_dt.is_mut_static());

    let cnt_idx = find_def_idx(&ws_reader, RDefKind::Static, "COUNTER");
    let cnt_dt = ws_reader.def_ty_for_def(cnt_idx).unwrap();
    assert!(cnt_dt.is_mut_static());
    assert_eq!(ws_arena.display_ty(cnt_dt.ty()), "u64");

    let assoc_consts: Vec<_> =
        ws_reader.definitions().iter().enumerate().filter(|(_, d)| RDefKind::from(d.kind) == RDefKind::AssocConst && ws_reader.get_str(d.name_offset()) == "ID").collect();
    assert_eq!(assoc_consts.len(), 2);
    for (ac_idx, _) in assoc_consts {
        assert_eq!(ws_arena.display_ty(ws_reader.def_ty_for_def(ac_idx as u32).unwrap().ty()), "u16");
    }

    // 3. Sysroot (`core` + `std`): `Range::start`/`end`, `Option::Some`(.0 + ctor) / `None`, `std::io::Result`
    struct SysrootG3Probe(bool);
    impl Callbacks for SysrootG3Probe {
        fn after_analysis<'tcx>(&mut self, _c: &rustc_interface::interface::Compiler, tcx: TyCtxt<'tcx>) -> Compilation {
            for krate in ["core", "std"] {
                let (mut writer, _) = sysroot::extract_sysroot_crate(tcx, krate, None).unwrap();
                let mut bytes = Vec::new();
                writer.write_to(&mut std::io::Cursor::new(&mut bytes)).unwrap();
                let tmp = std::env::temp_dir().join(format!("ridx_{krate}_g3_{}.ridx", std::process::id()));
                fs::write(&tmp, &bytes).unwrap();
                let reader = IndexReader::open(&tmp).unwrap();
                let _ = fs::remove_file(&tmp);
                let arena = reader.ty_arena();
                if krate == "core" {
                    let range_idx = find_def_idx(&reader, RDefKind::Struct, "core::ops::range::Range");
                    assert_eq!(reader.find_field(range_idx, "start").unwrap().vis, ItemVis::Public);
                    assert_eq!(reader.find_field(range_idx, "end").unwrap().vis, ItemVis::Public);

                    let some_idx = find_def_idx(&reader, RDefKind::Variant, "core::option::Option::Some");
                    assert_eq!(reader.def_ty_for_def(some_idx).unwrap().ctor_shape(), CtorShape::Tuple);
                    assert_eq!(arena.display_ty(reader.find_field(some_idx, "0").unwrap().ty), "T");
                    assert_eq!(arena.display_ty(reader.fn_sig_for_def(some_idx).unwrap().output()), "Option<T>");
                } else {
                    let io_res = reader
                        .definitions()
                        .iter()
                        .enumerate()
                        .find_map(|(idx, d)| {
                            let p = reader.get_str(d.path_offset());
                            (RDefKind::from(d.kind) == RDefKind::TyAlias && (p == "std::io::Result" || p == "std::io::error::Result")).then_some(idx as u32)
                        })
                        .unwrap_or_else(|| {
                            let aliases: Vec<&str> =
                                reader.definitions().iter().filter(|d| RDefKind::from(d.kind) == RDefKind::TyAlias).map(|d| reader.get_str(d.path_offset())).take(20).collect();
                            panic!("missing std::io::Result, sample TyAliases: {aliases:?}")
                        });
                    assert_eq!(arena.display_ty(reader.def_ty_for_def(io_res).unwrap().ty()), "Result<T, Error>");

                    let out_idx = find_def_idx(&reader, RDefKind::Struct, "process::Output");
                    assert_eq!(reader.find_field(out_idx, "status").unwrap().vis, ItemVis::Public);
                    assert_eq!(reader.find_field(out_idx, "stdout").unwrap().vis, ItemVis::Public);
                }
            }
            self.0 = true;
            Compilation::Stop
        }
    }

    let probe_src = temp_dir.join("probe_sysroot_g3.rs");
    fs::write(&probe_src, "pub fn noop() {}\n").unwrap();
    let probe_args = vec![
        "rustc".to_string(),
        probe_src.to_str().unwrap().to_string(),
        "--crate-name".to_string(),
        "probe_sysroot_g3".to_string(),
        "--crate-type".to_string(),
        "lib".to_string(),
        "--edition=2021".to_string(),
        "--emit=metadata".to_string(),
        "-o".to_string(),
        temp_dir.join("libprobe_sysroot_g3.rmeta").to_str().unwrap().to_string(),
        "--sysroot".to_string(),
        sysroot_str,
    ];
    let mut probe = SysrootG3Probe(false);
    assert!(run_with_callbacks(&probe_args, &mut probe).is_ok());
    assert!(probe.0, "SysrootG3Probe did not run");

    let _ = fs::remove_dir_all(&temp_dir);
}
