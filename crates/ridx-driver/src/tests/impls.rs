/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! End-to-end compiler tests for structured `ImplEntry` + `AssocTyEntry` extraction (1.2-G5)
//! across workspace crates, immutable dependencies, and sysroot (`core`), verifying zero filtering
//! and non-collapsing of `&T` (`RefShared`), `&mut T` (`RefMut`), `Param`, tuples, `fn` pointers,
//! `#[derive]`, and marker traits.

use super::*;
use ridx_format::{IndexReader, SimplifiedSelfKind};

#[test]
fn test_driver_impls_and_assoc_tys_unified_ws_dep_and_sysroot() {
    let _lock = DRIVER_TEST_LOCK.lock().unwrap();
    let temp_dir = std::env::temp_dir().join(format!("ridx_driver_impls_{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp_dir);
    let cache_dir = temp_dir.join("cache");
    let reg_dir = temp_dir.join(".cargo/registry/src/dep_impl");
    let ws_dir = temp_dir.join("ws_impl");
    let out_dir = temp_dir.join("out");
    fs::create_dir_all(&reg_dir).unwrap();
    fs::create_dir_all(&ws_dir).unwrap();
    fs::create_dir_all(out_dir.join(".deps")).unwrap();

    let sysroot = std::process::Command::new("rustc").args(["--print", "sysroot"]).output().unwrap();
    let sysroot_str = String::from_utf8_lossy(&sysroot.stdout).trim().to_string();

    // 1. Compile immutable dependency `dep_impl` with `#[derive(Clone, Copy)]`, inherent impl, and generic trait
    let dep_src = reg_dir.join("lib.rs");
    let dep_code = r#"
#[derive(Clone, Copy)]
pub struct Boxed<T>(pub T);

impl<T> Boxed<T> {
    pub fn into_inner(self) -> T {
        self.0
    }
}

pub trait Feed<In> {
    type Output;
    fn feed(self, x: In) -> Self::Output;
}
"#;
    fs::write(&dep_src, dep_code).unwrap();
    let dep_rmeta = temp_dir.join("libdep_impl.rmeta");
    let dep_args = vec![
        "rustc".to_string(),
        dep_src.to_str().unwrap().to_string(),
        "--crate-name".to_string(),
        "dep_impl".to_string(),
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
        .find(|p| p.file_name().and_then(|s| s.to_str()).is_some_and(|s| s.starts_with("dep_impl") && s.ends_with(".json")))
        .expect("dep_impl sidecar");
    let sidecar: serde_json::Value = serde_json::from_slice(&fs::read(&sidecar_path).unwrap()).unwrap();
    let dep_obj_path = sidecar["object_path"].as_str().unwrap().to_string();

    let dep_reader = IndexReader::open(&dep_obj_path).expect("open dep_impl shard");
    let dep_arena = dep_reader.ty_arena();
    // `Boxed<T>` must have 1 inherent impl + 2 derived trait impls (`Clone`, `Copy` marker trait!)
    let dep_inh: Vec<_> = dep_reader.inherent_impls_for_simplified(SimplifiedSelfKind::Adt).collect();
    assert_eq!(dep_inh.len(), 1);
    assert_eq!(dep_arena.display_ty(dep_inh[0].self_ty()), "Boxed<T>");
    assert_eq!(dep_inh[0].assoc_def_indices().len(), 1);

    let dep_tr: Vec<_> = dep_reader.trait_impls_for_simplified(SimplifiedSelfKind::Adt).collect();
    let dep_tr_paths: Vec<&str> = dep_tr.iter().filter_map(|i| i.trait_def().and_then(|d| dep_arena.def(d)).map(|v| v.path)).collect();
    assert!(dep_tr_paths.iter().any(|p| p.ends_with("::Clone")), "derived Clone impl missing: {dep_tr_paths:?}");
    assert!(dep_tr_paths.iter().any(|p| p.ends_with("::Copy")), "derived Copy marker impl missing: {dep_tr_paths:?}");

    // 2. Compile workspace crate `ws_impl` exercising `&T` vs `&mut T`, blanket `Param`, tuple, fn-ptr, unsafe impl
    let ws_src = ws_dir.join("lib.rs");
    let ws_code = r#"
pub trait BlanketTag {}
impl<T> BlanketTag for T {}

pub trait PairMarker {}
impl<A, B> PairMarker for (A, B) {}

pub trait FnCallable<R> {
    type Ret;
}
impl<R> FnCallable<R> for fn() -> R {
    type Ret = R;
}

pub unsafe trait UnsafeSyncTag {}
pub struct LocalCell<T>(pub T);
unsafe impl<T> UnsafeSyncTag for LocalCell<T> {}

pub struct Wrapper<T>(pub dep_impl::Boxed<T>);

impl<'a, T: Copy> dep_impl::Feed<u32> for &'a Wrapper<T> {
    type Output = T;
    fn feed(self, _x: u32) -> T {
        (self.0).0
    }
}

impl<'a, T> dep_impl::Feed<u32> for &'a mut Wrapper<T> {
    type Output = usize;
    fn feed(self, x: u32) -> usize {
        x as usize
    }
}
"#;
    fs::write(&ws_src, ws_code).unwrap();
    let ws_rmeta = temp_dir.join("libws_impl.rmeta");
    let ws_args = vec![
        "rustc".to_string(),
        ws_src.to_str().unwrap().to_string(),
        "--crate-name".to_string(),
        "ws_impl".to_string(),
        "--crate-type".to_string(),
        "lib".to_string(),
        "--edition=2021".to_string(),
        "--emit=metadata".to_string(),
        "--extern".to_string(),
        format!("dep_impl={}", dep_rmeta.display()),
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
        .find(|p| p.file_name().and_then(|s| s.to_str()).is_some_and(|s| s.starts_with("ws_impl") && s.ends_with(".ridx")))
        .expect("ws_impl.ridx");
    let ws_reader = IndexReader::open(&ws_idx_path).unwrap();
    let ws_arena = ws_reader.ty_arena();

    // `&Wrapper<T>` (RefShared, Output = T) vs `&mut Wrapper<T>` (RefMut, Output = usize) must NOT collapse!
    let ref_shared: Vec<_> = ws_reader.trait_impls_for_simplified(SimplifiedSelfKind::RefShared).collect();
    assert_eq!(ref_shared.len(), 1);
    assert_eq!(ws_arena.display_ty(ref_shared[0].self_ty()), "&'l0 Wrapper<T>");
    let out_shared = ref_shared[0].find_assoc_ty("Output").expect("RefShared Output");
    assert_eq!(ws_arena.display_ty(out_shared), "T");
    assert_eq!(ref_shared[0].assoc_def_indices().len(), 1);

    let ref_mut: Vec<_> = ws_reader.trait_impls_for_simplified(SimplifiedSelfKind::RefMut).collect();
    assert_eq!(ref_mut.len(), 1);
    assert_eq!(ws_arena.display_ty(ref_mut[0].self_ty()), "&'l0 mut Wrapper<T>");
    let out_mut = ref_mut[0].find_assoc_ty("Output").expect("RefMut Output");
    assert_eq!(ws_arena.display_ty(out_mut), "usize");

    // Blanket `Param`, `Tuple`, `FnPtr`, and `unsafe impl` on `Adt`
    assert_eq!(ws_reader.trait_impls_for_simplified(SimplifiedSelfKind::Param).len(), 1);
    assert_eq!(ws_reader.trait_impls_for_simplified(SimplifiedSelfKind::Tuple).len(), 1);
    let fn_impls: Vec<_> = ws_reader.trait_impls_for_simplified(SimplifiedSelfKind::FnPtr).collect();
    assert_eq!(fn_impls.len(), 1);
    assert_eq!(ws_arena.display_ty(fn_impls[0].find_assoc_ty("Ret").unwrap()), "R");

    let adt_tr: Vec<_> = ws_reader.trait_impls_for_simplified(SimplifiedSelfKind::Adt).collect();
    assert!(adt_tr.iter().any(|i| i.is_unsafe()), "unsafe impl flag missing on LocalCell");

    // 3. Verify sysroot (`core`) achieves 100% parity with `tcx.trait_impls_in_crate(core)` + `dyn Any` inherent impl
    struct SysrootImplProbe(Option<(usize, usize)>);
    impl Callbacks for SysrootImplProbe {
        fn after_analysis<'tcx>(&mut self, _c: &rustc_interface::interface::Compiler, tcx: TyCtxt<'tcx>) -> Compilation {
            let core_cnum = tcx.crates(()).iter().copied().find(|&c| tcx.crate_name(c).as_str() == "core").unwrap();
            let expected_trait_impls = tcx.trait_impls_in_crate(core_cnum).len();

            let (mut writer, _stats) = sysroot::extract_sysroot_crate(tcx, "core", None).expect("extract core");
            let mut bytes = Vec::new();
            writer.write_to(&mut std::io::Cursor::new(&mut bytes)).unwrap();
            let tmp = std::env::temp_dir().join(format!("ridx_core_impls_{}.ridx", std::process::id()));
            fs::write(&tmp, &bytes).unwrap();
            let reader = IndexReader::open(&tmp).unwrap();
            let _ = fs::remove_file(&tmp);
            let arena = reader.ty_arena();

            let recorded_trait_impls = reader.impls_iter().filter(|i| i.is_trait()).count();

            // Blanket `impl<T, U> Into<U> for T` in Param bucket
            let has_into_blanket =
                reader.trait_impls_for_simplified(SimplifiedSelfKind::Param).any(|i| i.trait_def().and_then(|d| arena.def(d)).is_some_and(|v| v.path.ends_with("::Into")));
            assert!(has_into_blanket, "blanket `impl<T, U> Into<U> for T` missing in core Param bucket");

            // `&[T]: IntoIterator` vs `&mut [T]: IntoIterator` preserved in distinct RefShared / RefMut buckets
            let has_ref_slice_iter = reader
                .trait_impls_for_simplified(SimplifiedSelfKind::RefShared)
                .any(|i| i.trait_def().and_then(|d| arena.def(d)).is_some_and(|v| v.path.ends_with("::IntoIterator")) && i.find_assoc_ty("Item").is_some());
            let has_mut_slice_iter = reader
                .trait_impls_for_simplified(SimplifiedSelfKind::RefMut)
                .any(|i| i.trait_def().and_then(|d| arena.def(d)).is_some_and(|v| v.path.ends_with("::IntoIterator")) && i.find_assoc_ty("Item").is_some());
            assert!(has_ref_slice_iter && has_mut_slice_iter, "&[T] / &mut [T] IntoIterator missing");

            // Marker trait `Copy` for `u32` preserved in Uint bucket
            let has_u32_copy = reader
                .trait_impls_for_simplified(SimplifiedSelfKind::Uint)
                .any(|i| arena.display_ty(i.self_ty()) == "u32" && i.trait_def().and_then(|d| arena.def(d)).is_some_and(|v| v.path.ends_with("::Copy")));
            assert!(has_u32_copy, "Copy marker impl for u32 missing");

            // Inherent `impl dyn Any` recorded in Dynamic bucket
            let has_dyn_any =
                reader.inherent_impls_for_simplified(SimplifiedSelfKind::Dynamic).any(|i| arena.display_ty(i.self_ty()).contains("Any") && !i.assoc_def_indices().is_empty());
            assert!(has_dyn_any, "inherent `impl dyn Any` missing in core Dynamic bucket");

            self.0 = Some((expected_trait_impls, recorded_trait_impls));
            Compilation::Stop
        }
    }

    let dummy_src = temp_dir.join("sys_impl_probe.rs");
    fs::write(&dummy_src, "").unwrap();
    let probe_args = vec![
        "rustc".to_string(),
        dummy_src.to_str().unwrap().to_string(),
        "--crate-name".to_string(),
        "sys_impl_probe".to_string(),
        "--crate-type".to_string(),
        "lib".to_string(),
        "--emit=metadata".to_string(),
        "-o".to_string(),
        temp_dir.join("libsys_impl_probe.rmeta").to_str().unwrap().to_string(),
        "--sysroot".to_string(),
        sysroot_str,
    ];
    let mut probe = SysrootImplProbe(None);
    assert!(run_with_callbacks(&probe_args, &mut probe).is_ok());
    let (expected_trait_impls, recorded_trait_impls) = probe.0.expect("probe ran");
    assert_eq!(expected_trait_impls, recorded_trait_impls, "100% of core trait impls must be recorded");

    let _ = fs::remove_dir_all(&temp_dir);
}
