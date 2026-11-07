/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! End-to-end compiler tests for `GenericParamEntry` and `WherePredEntry` (1.2-G4)
//! across workspace crates, external dependencies, and sysroot (`core` + `alloc` + `std`).

use super::*;
use ridx_format::{DefKind as RDefKind, GenericParamKind, IndexReader, SimplifiedSelfKind, WherePredKind};

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
fn test_driver_generics_and_where_preds_ws_dep_and_sysroot() {
    let _lock = DRIVER_TEST_LOCK.lock().unwrap();
    let temp_dir = std::env::temp_dir().join(format!("ridx_driver_generics_{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp_dir);
    let cache_dir = temp_dir.join("cache");
    let reg_dir = temp_dir.join(".cargo/registry/src/dep_g4");
    let ws_dir = temp_dir.join("ws_g4");
    let out_dir = temp_dir.join("out");
    fs::create_dir_all(&reg_dir).unwrap();
    fs::create_dir_all(&ws_dir).unwrap();
    fs::create_dir_all(out_dir.join(".deps")).unwrap();

    let sysroot = std::process::Command::new("rustc").args(["--print", "sysroot"]).output().unwrap();
    let sysroot_str = String::from_utf8_lossy(&sysroot.stdout).trim().to_string();

    // 1. External dependency `dep_g4`: default type + const params, `?Sized`, GATs, supertraits
    let dep_src = reg_dir.join("lib.rs");
    let dep_code = r#"
#![allow(dead_code)]
pub struct DepRing<T = u8, const CAP: usize = 64> {
    buf: [T; CAP],
}

pub struct DepUnsized<T: ?Sized>(pub T);

pub trait DepBase: Clone + Send {}

pub trait DepLending<In = Self>: DepBase {
    type Borrowed<'a>: Copy where Self: 'a;
    fn into_self(self) -> Self where Self: Sized { self }
}
"#;
    fs::write(&dep_src, dep_code).unwrap();
    let dep_rmeta = temp_dir.join("libdep_g4.rmeta");
    let dep_args = vec![
        "rustc".to_string(),
        dep_src.to_str().unwrap().to_string(),
        "--crate-name".to_string(),
        "dep_g4".to_string(),
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
        .find(|p| p.file_name().and_then(|s| s.to_str()).is_some_and(|s| s.starts_with("dep_g4") && s.ends_with(".json")))
        .expect("dep_g4 sidecar");
    let dep_sidecar: serde_json::Value = serde_json::from_slice(&fs::read(&dep_sidecar_path).unwrap()).unwrap();
    let dep_reader = IndexReader::open(dep_sidecar["object_path"].as_str().unwrap()).unwrap();
    let dep_arena = dep_reader.ty_arena();

    // DepRing<T = u8, const CAP: usize = 64>
    let ring_idx = find_def_idx(&dep_reader, RDefKind::Struct, "DepRing");
    let ring_gp: Vec<_> = dep_reader.generic_params_for_def(ring_idx).collect();
    assert_eq!(ring_gp.len(), 2);
    assert_eq!((ring_gp[0].name, ring_gp[0].kind), ("T", GenericParamKind::Type));
    assert!(ring_gp[0].has_default() && ring_gp[0].is_sized());
    assert_eq!(dep_arena.display_ty(ring_gp[0].default_ty().unwrap()), "u8");
    assert_eq!((ring_gp[1].name, ring_gp[1].kind), ("CAP", GenericParamKind::Const));
    assert!(ring_gp[1].has_default());
    assert_eq!(dep_arena.display_ty(ring_gp[1].const_ty().unwrap()), "usize");
    // Ordinary Sized param T does NOT bloat where_preds
    assert_eq!(dep_reader.where_preds_for_def(ring_idx).len(), 0);

    // DepUnsized<T: ?Sized>: is_sized() MUST be false!
    let unsz_idx = find_def_idx(&dep_reader, RDefKind::Struct, "DepUnsized");
    let unsz_gp: Vec<_> = dep_reader.generic_params_for_def(unsz_idx).collect();
    assert_eq!(unsz_gp.len(), 1);
    assert!(!unsz_gp[0].is_sized());

    // DepLending<In = Self>: Self (is_self, !is_sized) + In = Self + supertrait DepBase
    let lend_idx = find_def_idx(&dep_reader, RDefKind::Trait, "DepLending");
    let lend_gp: Vec<_> = dep_reader.generic_params_for_def(lend_idx).collect();
    assert_eq!(lend_gp.len(), 2);
    assert!(lend_gp[0].is_self() && !lend_gp[0].is_sized());
    assert_eq!(lend_gp[1].name, "In");
    assert_eq!(dep_arena.display_ty(lend_gp[1].default_ty().unwrap()), "Self");
    let lend_wp: Vec<_> = dep_reader.where_preds_for_def(lend_idx).collect();
    assert!(
        lend_wp.iter().any(|w| w.kind() == WherePredKind::TraitBound && w.trait_def().and_then(|td| dep_arena.def(td)).is_some_and(|d| d.path.ends_with("DepBase"))),
        "missing DepBase supertrait in DepLending where_preds"
    );

    // GAT `Borrowed<'a>: Copy where Self: 'a`
    let gat_idx = find_def_idx(&dep_reader, RDefKind::AssocTy, "DepLending::Borrowed");
    let gat_gp: Vec<_> = dep_reader.generic_params_for_def(gat_idx).collect();
    assert_eq!(gat_gp.len(), 1);
    assert_eq!((gat_gp[0].name, gat_gp[0].kind), ("'a", GenericParamKind::Lifetime));
    let gat_wp: Vec<_> = dep_reader.where_preds_for_def(gat_idx).collect();
    assert!(gat_wp.iter().any(|w| w.kind() == WherePredKind::TraitBound));
    assert!(gat_wp.iter().any(|w| w.kind() == WherePredKind::TypeOutlives));

    // Trait method `into_self` preserves `where Self: Sized`!
    let into_self_idx = find_def_idx(&dep_reader, RDefKind::AssocFn, "DepLending::into_self");
    let into_self_wp: Vec<_> = dep_reader.where_preds_for_def(into_self_idx).collect();
    assert!(
        into_self_wp.iter().any(|w| w.kind() == WherePredKind::TraitBound
            && dep_arena.display_ty(w.lhs_ty()) == "Self"
            && w.trait_def().and_then(|td| dep_arena.def(td)).is_some_and(|d| d.path.ends_with("Sized"))),
        "missing `Self: Sized` on DepLending::into_self"
    );

    // 2. Workspace crate `ws_g4`: ProjectionEq, HRTB, TypeOutlives, RegionOutlives, APIT synthetic param, blanket Impl bounds
    let ws_src = ws_dir.join("lib.rs");
    let ws_code = r#"
#![allow(dead_code)]
pub fn complex_bounds<'a, 'b: 'a, I, F>(iter: I, pred: F, extra: impl Clone) -> bool
where
    I: Iterator<Item = &'a str> + 'b,
    F: for<'x> Fn(&'x str) -> bool,
{
    let _ = extra.clone();
    iter.into_iter().any(pred)
}

pub trait Marker<T> {}

impl<I, V> Marker<V> for I
where
    I: Iterator<Item = V>,
{}
"#;
    fs::write(&ws_src, ws_code).unwrap();
    let ws_rmeta = out_dir.join("libws_g4.rmeta");
    let ws_args = vec![
        "rustc".to_string(),
        ws_src.to_str().unwrap().to_string(),
        "--crate-name".to_string(),
        "ws_g4".to_string(),
        "--crate-type".to_string(),
        "lib".to_string(),
        "--edition=2021".to_string(),
        "--emit=metadata".to_string(),
        "-o".to_string(),
        ws_rmeta.to_str().unwrap().to_string(),
        "--extern".to_string(),
        format!("dep_g4={}", dep_rmeta.display()),
        "--sysroot".to_string(),
        sysroot_str.clone(),
    ];
    let mut ws_env = DriverEnv::for_test(&out_dir);
    ws_env.cache_dir = cache_dir;
    ws_env.manifest_dir = Some(ws_dir);
    assert!(run_driver_with_env(ws_args, ws_env).is_ok());

    let ws_ridx = fs::read_dir(&out_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.file_name().and_then(|s| s.to_str()).is_some_and(|s| s.starts_with("ws_g4") && s.ends_with(".ridx")))
        .expect("ws_g4.ridx");
    let ws_reader = IndexReader::open(&ws_ridx).unwrap();
    let ws_arena = ws_reader.ty_arena();

    let fn_idx = find_def_idx(&ws_reader, RDefKind::Fn, "complex_bounds");
    let fn_gp: Vec<_> = ws_reader.generic_params_for_def(fn_idx).collect();
    // 'a, 'b, I, F, impl Clone (synthetic)
    assert_eq!(fn_gp.len(), 5);
    assert_eq!(fn_gp[0].kind, GenericParamKind::Lifetime);
    assert_eq!(fn_gp[1].kind, GenericParamKind::Lifetime);
    assert!(fn_gp.iter().any(|g| g.is_synthetic() && g.kind == GenericParamKind::Type));

    let fn_wp: Vec<_> = ws_reader.where_preds_for_def(fn_idx).collect();
    assert!(fn_wp.iter().any(|w| w.kind() == WherePredKind::RegionOutlives));
    assert!(fn_wp.iter().any(|w| w.kind() == WherePredKind::TypeOutlives));
    assert!(fn_wp.iter().any(|w| w.kind() == WherePredKind::ProjectionEq && w.assoc_name() == Some("Item")));
    assert!(fn_wp.iter().any(|w| w.kind() == WherePredKind::TraitBound && w.is_hrtb()));

    // Blanket `impl<I, V> Marker<V> for I where I: Iterator<Item = V>`
    let blanket_impls: Vec<_> = ws_reader.trait_impls_for_simplified(SimplifiedSelfKind::Param).collect();
    assert_eq!(blanket_impls.len(), 1);
    let b_idx = blanket_impls[0].index();
    let b_gp: Vec<_> = ws_reader.generic_params_for_impl(b_idx).collect();
    assert_eq!(b_gp.len(), 2);
    let b_wp: Vec<_> = ws_reader.where_preds_for_impl(b_idx).collect();
    assert!(b_wp.iter().any(|w| w.kind() == WherePredKind::TraitBound && w.trait_def().and_then(|td| ws_arena.def(td)).is_some_and(|d| d.path.ends_with("Iterator"))));
    assert!(b_wp.iter().any(|w| w.kind() == WherePredKind::ProjectionEq && w.assoc_name() == Some("Item")));

    // 3. Sysroot (`core` + `alloc`): `Iterator::collect` (`Self: Sized` + `B: FromIterator<Self::Item>`),
    // blanket `impl<I: Iterator> IntoIterator for I`, and `Vec<T, A = Global>` default param!
    struct SysrootG4Probe(bool);
    impl Callbacks for SysrootG4Probe {
        fn after_analysis<'tcx>(&mut self, _c: &rustc_interface::interface::Compiler, tcx: TyCtxt<'tcx>) -> Compilation {
            for krate in ["core", "alloc"] {
                let (mut writer, _) = sysroot::extract_sysroot_crate(tcx, krate, None).unwrap();
                let mut bytes = Vec::new();
                writer.write_to(&mut std::io::Cursor::new(&mut bytes)).unwrap();
                let tmp = std::env::temp_dir().join(format!("ridx_{krate}_g4_{}.ridx", std::process::id()));
                fs::write(&tmp, &bytes).unwrap();
                let reader = IndexReader::open(&tmp).unwrap();
                let _ = fs::remove_file(&tmp);
                let arena = reader.ty_arena();
                if krate == "core" {
                    let collect_idx = find_def_idx(&reader, RDefKind::AssocFn, "Iterator::collect");
                    let collect_gp: Vec<_> = reader.generic_params_for_def(collect_idx).collect();
                    assert_eq!(collect_gp.len(), 1);
                    assert_eq!(collect_gp[0].name, "B");
                    let collect_wp: Vec<_> = reader.where_preds_for_def(collect_idx).collect();
                    assert!(
                        collect_wp.iter().any(|w| arena.display_ty(w.lhs_ty()) == "Self" && w.trait_def().and_then(|td| arena.def(td)).is_some_and(|d| d.path.ends_with("Sized"))),
                        "Iterator::collect missing `Self: Sized`"
                    );
                    assert!(
                        collect_wp
                            .iter()
                            .any(|w| arena.display_ty(w.lhs_ty()) == "B" && w.trait_def().and_then(|td| arena.def(td)).is_some_and(|d| d.path.ends_with("FromIterator"))),
                        "Iterator::collect missing `B: FromIterator<Self::Item>`"
                    );

                    // Blanket `impl<I: Iterator> IntoIterator for I` (anonymous sysroot impl, def_index == None!)
                    let into_iter_blanket = reader
                        .trait_impls_for_simplified(SimplifiedSelfKind::Param)
                        .find(|imp| imp.trait_def().and_then(|td| arena.def(td)).is_some_and(|d| d.path.ends_with("::IntoIterator")))
                        .expect("blanket impl<I: Iterator> IntoIterator for I in core");
                    assert_eq!(into_iter_blanket.def_index(), None);
                    let ii_idx = into_iter_blanket.index();
                    assert_eq!(reader.generic_params_for_impl(ii_idx).len(), 1);
                    assert!(
                        reader.where_preds_for_impl(ii_idx).any(|w| w.trait_def().and_then(|td| arena.def(td)).is_some_and(|d| d.path.ends_with("::Iterator"))),
                        "blanket IntoIterator impl missing `I: Iterator` bound"
                    );
                } else {
                    let vec_idx = find_def_idx(&reader, RDefKind::Struct, "alloc::vec::Vec");
                    let vec_gp: Vec<_> = reader.generic_params_for_def(vec_idx).collect();
                    assert_eq!(vec_gp.len(), 2);
                    assert_eq!(vec_gp[0].name, "T");
                    assert!(vec_gp[0].is_sized() && !vec_gp[0].has_default());
                    assert_eq!(vec_gp[1].name, "A");
                    assert!(vec_gp[1].has_default());
                    assert_eq!(arena.display_ty(vec_gp[1].default_ty().unwrap()), "Global");
                }
            }
            self.0 = true;
            Compilation::Stop
        }
    }

    let probe_src = temp_dir.join("probe_sysroot_g4.rs");
    fs::write(&probe_src, "pub fn noop() {}\n").unwrap();
    let probe_args = vec![
        "rustc".to_string(),
        probe_src.to_str().unwrap().to_string(),
        "--crate-name".to_string(),
        "probe_sysroot_g4".to_string(),
        "--crate-type".to_string(),
        "lib".to_string(),
        "--edition=2021".to_string(),
        "--emit=metadata".to_string(),
        "-o".to_string(),
        temp_dir.join("libprobe_sysroot_g4.rmeta").to_str().unwrap().to_string(),
        "--sysroot".to_string(),
        sysroot_str,
    ];
    let mut probe = SysrootG4Probe(false);
    assert!(run_with_callbacks(&probe_args, &mut probe).is_ok());
    assert!(probe.0, "SysrootG4Probe did not run");

    let _ = fs::remove_dir_all(&temp_dir);
}
