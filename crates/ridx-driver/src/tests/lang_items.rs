/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! End-to-end compiler tests for `LangItemEntry` extraction and unstable `core` lang-item
//! materialization (`Try`, `FromResidual`, `Residual`, `Fn*::call*`, `AsyncFn*`, `Deref*`,
//! `Index*`, `IntoIterator`, `Future`, operators, ranges, `OwnedBox`, `String`) (1.2-G6).

use super::*;
use ridx_format::{DefKind as RDefKind, IndexReader, LangItemKind, SimplifiedSelfKind, WorkspaceIndex};

#[test]
fn test_driver_sysroot_lang_items_full_coverage_and_unstable_bypass() {
    let _lock = DRIVER_TEST_LOCK.lock().unwrap();
    let temp_dir = std::env::temp_dir().join(format!("ridx_driver_lang_items_{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp_dir);
    let sys_dir = temp_dir.join("sysroot").join("cas_g6");
    let ws_dir = temp_dir.join("ws");
    fs::create_dir_all(&sys_dir).unwrap();
    fs::create_dir_all(&ws_dir).unwrap();

    let sysroot = std::process::Command::new("rustc").args(["--print", "sysroot"]).output().unwrap();
    let sysroot_str = String::from_utf8_lossy(&sysroot.stdout).trim().to_string();

    struct SysrootG6Probe {
        sys_dir: std::path::PathBuf,
        ran: bool,
    }

    impl Callbacks for SysrootG6Probe {
        fn after_analysis<'tcx>(&mut self, _c: &rustc_interface::interface::Compiler, tcx: TyCtxt<'tcx>) -> Compilation {
            for krate in ["core", "alloc", "std"] {
                let (mut writer, _) = sysroot::extract_sysroot_crate(tcx, krate, None).unwrap();
                writer.set_unit_key(format!("{krate}-1.98.1"));
                let mut bytes = Vec::new();
                writer.write_to(&mut std::io::Cursor::new(&mut bytes)).unwrap();
                fs::write(self.sys_dir.join(format!("{krate}.ridx")), &bytes).unwrap();
            }
            self.ran = true;
            Compilation::Stop
        }
    }

    let probe_src = temp_dir.join("probe_g6.rs");
    fs::write(&probe_src, "pub fn noop() {}\n").unwrap();
    let probe_args = vec![
        "rustc".to_string(),
        probe_src.to_str().unwrap().to_string(),
        "--crate-name".to_string(),
        "probe_g6".to_string(),
        "--crate-type".to_string(),
        "lib".to_string(),
        "--edition=2021".to_string(),
        "--emit=metadata".to_string(),
        "-o".to_string(),
        temp_dir.join("libprobe_g6.rmeta").to_str().unwrap().to_string(),
        "--sysroot".to_string(),
        sysroot_str,
    ];
    let mut probe = SysrootG6Probe { sys_dir: sys_dir.clone(), ran: false };
    assert!(run_with_callbacks(&probe_args, &mut probe).is_ok());
    assert!(probe.ran, "SysrootG6Probe did not run");

    let core_reader = IndexReader::open(sys_dir.join("core.ridx")).unwrap();
    let core_arena = core_reader.ty_arena();
    let alloc_reader = IndexReader::open(sys_dir.join("alloc.ridx")).unwrap();
    let alloc_arena = alloc_reader.ty_arena();

    // 1. Every `core` LangItemKind (`0..=111` except `OwnedBox` and `String` in `alloc`) must exist
    // in `core.ridx` with a materialized local `DefEntry`!
    for raw in 0..=LangItemKind::KNOWN_MAX {
        let kind = LangItemKind::from_u16(raw);
        if matches!(kind, LangItemKind::OwnedBox | LangItemKind::String) {
            let hit = alloc_reader.lang_item(kind).unwrap_or_else(|| panic!("missing {kind:?} in alloc.ridx"));
            let def_idx = hit.def_index.unwrap_or_else(|| panic!("{kind:?} missing def_index in alloc.ridx"));
            assert!((def_idx as usize) < alloc_reader.definitions().len());
            assert_eq!(alloc_arena.def(hit.ty_def).unwrap().local_def_idx, Some(def_idx));
        } else {
            let hit = core_reader.lang_item(kind).unwrap_or_else(|| panic!("missing {kind:?} in core.ridx"));
            let def_idx = hit.def_index.unwrap_or_else(|| panic!("{kind:?} missing def_index in core.ridx"));
            assert!((def_idx as usize) < core_reader.definitions().len());
            assert_eq!(core_arena.def(hit.ty_def).unwrap().local_def_idx, Some(def_idx));
        }
    }

    // 2. Unstable `?` operator items (`Try`, `FromResidual`, `Residual`) must have full `FnSig` & `GenericParam`s!
    let try_view = core_reader.lang_item(LangItemKind::Try).unwrap();
    assert_eq!(core_arena.def(try_view.ty_def).unwrap().path, "core::ops::try_trait::Try");
    let branch_view = core_reader.lang_item(LangItemKind::TryBranch).unwrap();
    let branch_sig = core_reader.fn_sig_for_def(branch_view.def_index.unwrap()).expect("FnSig for Try::branch");
    assert!(branch_sig.has_self());
    assert!(core_arena.display_ty(branch_sig.output()).contains("ControlFlow"));

    let from_res_trait = core_reader.lang_item(LangItemKind::FromResidual).unwrap();
    assert!(core_arena.def(from_res_trait.ty_def).unwrap().path.ends_with("::FromResidual"));
    let from_res_gp: Vec<_> = core_reader.generic_params_for_def(from_res_trait.def_index.unwrap()).collect();
    assert_eq!(from_res_gp.len(), 2);
    assert!(from_res_gp[0].is_self());
    assert_eq!(from_res_gp[1].name, "R");
    assert!(from_res_gp[1].has_default(), "FromResidual<R = <Self as Try>::Residual> must preserve default R");

    let from_res_fn = core_reader.lang_item(LangItemKind::FromResidualFn).unwrap();
    let from_res_sig = core_reader.fn_sig_for_def(from_res_fn.def_index.unwrap()).expect("FnSig for FromResidual::from_residual");
    assert_eq!(from_res_sig.inputs().len(), 1);

    let res_trait = core_reader.lang_item(LangItemKind::Residual).unwrap();
    assert!(core_arena.def(res_trait.ty_def).unwrap().path.ends_with("::Residual"));
    let res_try_ty = core_reader.lang_item(LangItemKind::ResidualTryType).unwrap();
    assert_eq!(RDefKind::from(core_reader.definitions()[res_try_ty.def_index.unwrap() as usize].kind), RDefKind::AssocTy);

    // `impl<T, E> Try for Result<T, E>` in `core` binds `Output = T` and `Residual = Result<Infallible, E>`!
    let result_view = core_reader.lang_item(LangItemKind::Result).unwrap();
    let result_try_impl = core_reader
        .trait_impls_for_simplified(SimplifiedSelfKind::Adt)
        .find(|imp| imp.trait_def() == Some(try_view.ty_def) && matches!(core_arena.kind(imp.self_ty()), ridx_format::TyKindView::Adt { def, .. } if def == result_view.ty_def))
        .expect("impl Try for Result<T, E>");
    let assoc_names: Vec<_> = result_try_impl.assoc_tys().map(|a| a.name).collect();
    assert!(assoc_names.contains(&"Output") && assoc_names.contains(&"Residual"));

    // 3. Unstable callable methods (`Fn::call`, `FnMut::call_mut`, `FnOnce::call_once`, `AsyncFn::async_call`) have `FnSig`!
    for m_kind in [
        LangItemKind::FnCall,
        LangItemKind::FnMutCallMut,
        LangItemKind::FnOnceCallOnce,
        LangItemKind::AsyncFnCall,
        LangItemKind::AsyncFnMutCallMut,
        LangItemKind::AsyncFnOnceCallOnce,
    ] {
        let mv = core_reader.lang_item(m_kind).unwrap();
        let sig = core_reader.fn_sig_for_def(mv.def_index.unwrap()).unwrap_or_else(|| panic!("missing FnSig for {m_kind:?}"));
        assert_eq!(sig.inputs().len(), 2, "{m_kind:?} must take (self_receiver, Args)");
    }

    // 4. `WorkspaceIndex::lang_item` resolves all 112 `LangItemKind`s across `core` + `alloc`!
    let mut ws_writer = IndexWriter::new("ws_g6", "0.1.0");
    ws_writer.set_unit_key("ws_g6-0.1.0");
    let mut ws_buf = Vec::new();
    ws_writer.write_to(&mut std::io::Cursor::new(&mut ws_buf)).unwrap();
    fs::write(ws_dir.join("ws_g6.ridx"), &ws_buf).unwrap();

    let mut ws = WorkspaceIndex::open(&ws_dir).unwrap();
    ws.attach_sysroot_shards_for_key_in(&temp_dir.join("sysroot"), "cas_g6");
    for raw in 0..=LangItemKind::KNOWN_MAX {
        let kind = LangItemKind::from_u16(raw);
        assert!(ws.lang_item(kind).is_some(), "WorkspaceIndex::lang_item failed for {kind:?}");
    }

    // 5. Unstable lang items must NOT pollute legacy `defs_by_owner` / `derefs` completion tables:
    // - stable `Iterator` must not gain nightly-unstable methods (`advance_by`, `intersperse`),
    // - unstable `core::ops::deref::Receiver` must not appear in `defs_by_owner` (`Target`) or `derefs`.
    let iter_owned: Vec<_> = ws
        .find_definitions_by_owner("core::iter::Iterator")
        .into_iter()
        .chain(ws.find_definitions_by_owner("core::iter::traits::iterator::Iterator"))
        .map(|d| d.name.to_string())
        .collect();
    assert!(iter_owned.iter().any(|n| n == "next"), "stable Iterator::next must remain in defs_by_owner");
    for unstable_m in ["advance_by", "array_chunks", "intersperse", "map_windows", "try_collect"] {
        assert!(!iter_owned.iter().any(|n| n == unstable_m), "nightly-unstable Iterator::{unstable_m} leaked into defs_by_owner");
        assert!(
            !core_reader.definitions().iter().any(|d| core_reader.get_str(d.name_offset()) == unstable_m),
            "nightly-unstable Iterator::{unstable_m} leaked into core.ridx DefEntry"
        );
    }
    assert!(ws.find_definitions_by_owner("core::ops::deref::Receiver").is_empty(), "unstable core::ops::deref::Receiver leaked into defs_by_owner");
    assert!(core_reader.find_deref_targets("core::ops::deref::LegacyReceiver").is_empty());

    let _ = fs::remove_dir_all(&temp_dir);
}
