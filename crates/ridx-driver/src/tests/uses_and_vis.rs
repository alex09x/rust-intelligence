/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! End-to-end compiler tests for `DefEntry.visibility` (`ItemVis`) and `UseEntry` tables
//! across workspace crates, external dependencies, and sysroot (`core`, `alloc`, `std`) (1.2-G7).

use super::*;
use ridx_format::{DefKind as RDefKind, IndexReader, ItemVis, UseOwner, UsePrimTy};

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
fn test_driver_visibility_and_use_table_ws_dep_and_sysroot() {
    let _lock = DRIVER_TEST_LOCK.lock().unwrap();
    let temp_dir = std::env::temp_dir().join(format!("ridx_driver_uses_vis_{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp_dir);
    let cache_dir = temp_dir.join("cache");
    let reg_dir = temp_dir.join(".cargo/registry/src/dep_g7");
    let ws_dir = temp_dir.join("ws_g7");
    let out_dir = temp_dir.join("out");
    fs::create_dir_all(&reg_dir).unwrap();
    fs::create_dir_all(&ws_dir).unwrap();
    fs::create_dir_all(out_dir.join(".deps")).unwrap();

    let sysroot = std::process::Command::new("rustc").args(["--print", "sysroot"]).output().unwrap();
    let sysroot_str = String::from_utf8_lossy(&sysroot.stdout).trim().to_string();

    // 1. External dependency `dep_g7`: tuple vs opaque struct re-exports, private-mod glob re-export, #[macro_export]
    let dep_src = reg_dir.join("lib.rs");
    let dep_code = r#"
#![allow(dead_code)]
mod detail {
    pub struct DepPair(pub u32, pub u32);
    pub struct DepOpaque(u32);
    pub struct SecretExport(pub u8);
    pub trait TraitAlpha { fn alpha(&self) -> u32 { 1 } }
    pub trait TraitBeta { fn beta(&self) -> u32 { 2 } }

    #[macro_export]
    macro_rules! dep_mac {
        () => { 42u32 };
    }
}

pub use detail::{DepOpaque, DepPair, TraitAlpha, TraitBeta};
pub use detail::*;
"#;
    fs::write(&dep_src, dep_code).unwrap();
    let dep_rmeta = temp_dir.join("libdep_g7.rmeta");
    let dep_args = vec![
        "rustc".to_string(),
        dep_src.to_str().unwrap().to_string(),
        "--crate-name".to_string(),
        "dep_g7".to_string(),
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

    let dep_sidecar_path = fs::read_dir(out_dir.join(".deps"))
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.file_name().and_then(|s| s.to_str()).is_some_and(|s| s.starts_with("dep_g7") && s.ends_with(".json")))
        .expect("dep_g7 sidecar");
    let dep_sidecar: serde_json::Value = serde_json::from_slice(&fs::read(&dep_sidecar_path).unwrap()).unwrap();
    let dep_reader = IndexReader::open(dep_sidecar["object_path"].as_str().unwrap()).unwrap();
    let dep_arena = dep_reader.ty_arena();

    // Multi-namespace re-export: DepPair has TypeNS + ValueNS; DepOpaque has TypeNS only
    let pair_use = dep_reader.use_by_name(UseOwner::CrateRoot, "DepPair").expect("DepPair reexport");
    assert_eq!(pair_use.vis, ItemVis::Public);
    assert!(pair_use.is_reexport());
    let pair_td = pair_use.type_def.expect("DepPair type_def");
    assert_eq!(pair_use.value_def, Some(pair_td), "tuple struct ctor normalizes to Struct TyDefId");
    assert_eq!(dep_arena.def(pair_td).unwrap().path, "dep_g7::detail::DepPair");

    let opaque_use = dep_reader.use_by_name(UseOwner::CrateRoot, "DepOpaque").expect("DepOpaque reexport");
    assert!(opaque_use.type_def.is_some());
    assert!(opaque_use.value_def.is_none(), "private-field tuple struct must not expose value ctor");

    // Flattened private submodule glob re-export (`SecretExport`) + #[macro_export] (`dep_mac`)
    let secret_use = dep_reader.use_by_name(UseOwner::CrateRoot, "SecretExport").expect("SecretExport flattened");
    assert_eq!(secret_use.vis, ItemVis::Public);
    assert!(secret_use.type_def.is_some() && secret_use.value_def.is_some());

    let mac_use = dep_reader.use_by_name(UseOwner::CrateRoot, "dep_mac").expect("dep_mac root export");
    assert_eq!(mac_use.vis, ItemVis::Public);
    assert!(mac_use.macro_def.is_some());

    // 2. Workspace crate `ws_g7`: all 4 ItemVis levels, extern crate, `_` trait imports, fn-local use, sysroot check
    let ws_src = ws_dir.join("lib.rs");
    let ws_code = r#"
#![allow(dead_code, unused_imports)]
extern crate dep_g7 as aliased_dep;

pub fn root_pub() {}
pub(crate) fn root_crate() {}
fn root_priv() {}

pub(crate) enum CrateEnum {
    Var { count: u32 },
}

pub(crate) trait CrateTrait {
    fn trait_method(&self);
}

pub mod outer {
    pub mod inner {
        use crate::aliased_dep::{TraitAlpha as _, TraitBeta as _};
        pub fn in_pub() {}
        pub(crate) fn in_crate() {}
        pub(super) fn in_super() {}
        fn in_priv() {
            struct FnLocalStruct;
            let _ = FnLocalStruct;
        }
    }
}

pub fn scoped_fn() -> u32 {
    use aliased_dep::*;
    dep_mac!()
}
"#;
    fs::write(&ws_src, ws_code).unwrap();
    let ws_rmeta = temp_dir.join("libws_g7.rmeta");
    let ws_args = vec![
        "rustc".to_string(),
        ws_src.to_str().unwrap().to_string(),
        "--crate-name".to_string(),
        "ws_g7".to_string(),
        "--crate-type".to_string(),
        "lib".to_string(),
        "--edition=2021".to_string(),
        "--extern".to_string(),
        format!("dep_g7={}", dep_rmeta.display()),
        "--emit=metadata".to_string(),
        "-o".to_string(),
        ws_rmeta.to_str().unwrap().to_string(),
        "--sysroot".to_string(),
        sysroot_str.clone(),
    ];
    let mut ws_env = DriverEnv::for_test(&out_dir);
    ws_env.cache_dir = cache_dir;
    ws_env.manifest_dir = Some(ws_dir);
    assert!(run_driver_with_env(ws_args, ws_env).is_ok());

    let ws_ridx = fs::read_dir(&out_dir).unwrap().filter_map(|e| e.ok()).map(|e| e.path()).find(|p| p.extension().and_then(|s| s.to_str()) == Some("ridx")).expect("ws_g7 ridx");
    let ws_reader = IndexReader::open(&ws_ridx).unwrap();
    let ws_arena = ws_reader.ty_arena();
    let vis_of = |k: RDefKind, suffix: &str| ws_reader.definitions()[find_def_idx(&ws_reader, k, suffix) as usize].item_vis();

    assert_eq!(vis_of(RDefKind::Fn, "root_pub"), ItemVis::Public);
    assert_eq!(vis_of(RDefKind::Fn, "root_crate"), ItemVis::Crate);
    assert_eq!(vis_of(RDefKind::Fn, "root_priv"), ItemVis::Private);
    assert_eq!(vis_of(RDefKind::Enum, "CrateEnum"), ItemVis::Crate);
    assert_eq!(vis_of(RDefKind::Variant, "CrateEnum::Var"), ItemVis::Crate);
    assert_eq!(vis_of(RDefKind::Field, "CrateEnum::Var::count"), ItemVis::Crate);
    assert_eq!(vis_of(RDefKind::AssocFn, "CrateTrait::trait_method"), ItemVis::Crate);
    assert_eq!(vis_of(RDefKind::Fn, "outer::inner::in_pub"), ItemVis::Public);
    assert_eq!(vis_of(RDefKind::Fn, "outer::inner::in_crate"), ItemVis::Crate);
    assert_eq!(vis_of(RDefKind::Fn, "outer::inner::in_super"), ItemVis::Restricted);
    assert_eq!(vis_of(RDefKind::Fn, "outer::inner::in_priv"), ItemVis::Private);
    assert_eq!(vis_of(RDefKind::Struct, "FnLocalStruct"), ItemVis::Private);

    // Prelude glob + extern crate + `_` trait imports + fn-local glob isolation
    let prelude = ws_reader.prelude_use().expect("prelude_use");
    assert!(prelude.is_glob() && prelude.is_prelude());
    assert_eq!(ws_arena.def(prelude.type_def.unwrap()).unwrap().path, "std::prelude::rust_2021");

    let ext = ws_reader.use_by_name(UseOwner::CrateRoot, "aliased_dep").expect("aliased_dep");
    assert!(ext.is_extern_crate());
    assert_eq!(ws_arena.def(ext.type_def.unwrap()).unwrap().path, "dep_g7");

    let inner_mod_idx = find_def_idx(&ws_reader, RDefKind::Mod, "outer::inner");
    let underscores: Vec<_> = ws_reader.underscore_uses_for_owner(UseOwner::Def(inner_mod_idx)).collect();
    assert_eq!(underscores.len(), 2, "both `use Trait as _;` imports preserved");

    let non_prelude_root_globs: Vec<_> = ws_reader.glob_uses_for_owner(UseOwner::CrateRoot).filter(|g| !g.is_prelude()).collect();
    assert!(non_prelude_root_globs.is_empty(), "fn-local glob must not leak into CrateRoot");
    let scoped_fn_idx = find_def_idx(&ws_reader, RDefKind::Fn, "scoped_fn");
    assert_eq!(ws_reader.glob_uses_for_owner(UseOwner::Def(scoped_fn_idx)).len(), 1);

    // 3. Sysroot `core` + `std` preludes and `primitive` modules
    struct SysrootG7Check(bool);
    impl Callbacks for SysrootG7Check {
        fn after_analysis<'tcx>(&mut self, _c: &rustc_interface::interface::Compiler, tcx: TyCtxt<'tcx>) -> Compilation {
            let (mut core_w, _) = sysroot::extract_sysroot_crate(tcx, "core", None).unwrap();
            let mut core_bytes = Vec::new();
            core_w.write_to(&mut std::io::Cursor::new(&mut core_bytes)).unwrap();
            let core_r = IndexReader::from_buffer(core_bytes).unwrap();

            let p2024_idx = find_def_idx(&core_r, RDefKind::Mod, "core::prelude::rust_2024");
            assert!(core_r.use_by_name(UseOwner::Def(p2024_idx), "Future").is_some());
            assert!(core_r.use_by_name(UseOwner::Def(p2024_idx), "IntoFuture").is_some());

            let prim_idx = find_def_idx(&core_r, RDefKind::Mod, "core::primitive");
            let u32_use = core_r.use_by_name(UseOwner::Def(prim_idx), "u32").expect("core::primitive::u32");
            assert!(u32_use.has_prim_ty() && u32_use.prim_ty == UsePrimTy::U32);

            let (mut std_w, _) = sysroot::extract_sysroot_crate(tcx, "std", None).unwrap();
            let mut std_bytes = Vec::new();
            std_w.write_to(&mut std::io::Cursor::new(&mut std_bytes)).unwrap();
            let std_r = IndexReader::from_buffer(std_bytes).unwrap();
            let std_arena = std_r.ty_arena();

            let p2021_idx = find_def_idx(&std_r, RDefKind::Mod, "std::prelude::rust_2021");
            let some_use = std_r.use_by_name(UseOwner::Def(p2021_idx), "Some").expect("Some in std prelude");
            assert!(some_use.type_def.is_some() && some_use.value_def.is_some(), "Some has Variant + VariantCtor");
            let vec_use = std_r.use_by_name(UseOwner::Def(p2021_idx), "Vec").expect("Vec in std prelude");
            assert_eq!(std_arena.def(vec_use.type_def.unwrap()).unwrap().path, "alloc::vec::Vec");
            let clone_use = std_r.use_by_name(UseOwner::Def(p2021_idx), "Clone").expect("Clone in std prelude");
            assert!(clone_use.type_def.is_some() && clone_use.macro_def.is_some(), "Clone has Trait + Derive macro");
            self.0 = true;
            Compilation::Stop
        }
    }
    let probe_src = temp_dir.join("probe_sys.rs");
    fs::write(&probe_src, "pub fn noop() {}\n").unwrap();
    let mut chk = SysrootG7Check(false);
    rustc_driver::run_compiler(
        &[
            "rustc".into(),
            probe_src.to_str().unwrap().into(),
            "--crate-name".into(),
            "probe_sys".into(),
            "--crate-type".into(),
            "lib".into(),
            "--edition=2021".into(),
            "--emit=metadata".into(),
            "-o".into(),
            temp_dir.join("libprobe_sys.rmeta").to_str().unwrap().into(),
            "--sysroot".into(),
            sysroot_str,
        ],
        &mut chk,
    );
    assert!(chk.0);
    let _ = fs::remove_dir_all(&temp_dir);
}
