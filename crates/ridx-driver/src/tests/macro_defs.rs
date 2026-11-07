/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! End-to-end `ridx-driver` tests for Step 1.4 (`G8`): declarative macro definitions
//! (`macro_rules!`, `#[macro_export(local_inner_macros)]`, meta-macro-generated `macro_rules!`,
//! proc-macro exclusion, and `sysroot` `.rmeta` extraction across `core`, `alloc`, and `std`).

use super::*;
use ridx_format::{DefKind as RDefKind, IndexReader, MacroDelim, MacroEdition};
use rustc_driver::{Callbacks, Compilation};
use rustc_middle::ty::TyCtxt;

fn find_def_idx<B: AsRef<[u8]>>(reader: &IndexReader<B>, kind: RDefKind, path_suffix: &str) -> u32 {
    reader
        .definitions()
        .iter()
        .enumerate()
        .find_map(|(i, raw)| {
            let d = raw.from_le();
            let p = reader.get_str(d.path_offset);
            (RDefKind::from(d.kind) == kind && (p == path_suffix || p.ends_with(path_suffix))).then_some(i as u32)
        })
        .unwrap_or_else(|| panic!("missing {kind:?} definition `{path_suffix}` in crate `{}`", reader.crate_name()))
}

#[test]
fn test_driver_macro_defs_ws_dep_meta_proc_macro_and_sysroot() {
    let _lock = DRIVER_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let temp_dir = std::env::temp_dir().join(format!("ridx_test_macro_defs_g8_{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp_dir);

    let cache_dir = temp_dir.join("cache");
    let reg_dir = temp_dir.join(".cargo/registry/src/dep_g8");
    let ws_dir = temp_dir.join("ws_g8");
    let pm_dir = temp_dir.join("pm_g8");
    let out_dir = temp_dir.join("out");
    for d in [&reg_dir, &ws_dir, &pm_dir] {
        fs::create_dir_all(d).unwrap();
    }
    fs::create_dir_all(out_dir.join(".deps")).unwrap();

    let sysroot = std::process::Command::new("rustc").args(["--print", "sysroot"]).output().unwrap();
    let sysroot_str = String::from_utf8_lossy(&sysroot.stdout).trim().to_string();

    // 1. External dependency `dep_g8` (Edition 2018): Paren delim, local_inner_macros, and meta-macro-generated macro_rules!
    let dep_src = reg_dir.join("lib.rs");
    let dep_code = r#"
#[macro_export]
macro_rules! dep_helper {
    ($v:expr) => { ($v + 1) };
}

#[macro_export(local_inner_macros)]
macro_rules! dep_add (
    ($a:expr, $b:expr) => { dep_helper!($a) + $b };
);

macro_rules! make_exported_bracket_mac {
    ($mac_name:ident) => {
        #[macro_export]
        macro_rules! $mac_name [
            ($item:expr) => [ $item * 10 ];
        ];
    };
}
make_exported_bracket_mac!(generated_tenx);
"#;
    fs::write(&dep_src, dep_code).unwrap();
    let dep_rmeta = temp_dir.join("libdep_g8.rmeta");
    let dep_args = vec![
        "rustc".to_string(),
        dep_src.to_str().unwrap().to_string(),
        "--crate-name".to_string(),
        "dep_g8".to_string(),
        "--crate-type".to_string(),
        "lib".to_string(),
        "--edition=2018".to_string(),
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
        .find(|p| p.file_name().and_then(|s| s.to_str()).is_some_and(|s| s.starts_with("dep_g8") && s.ends_with(".json")))
        .expect("dep_g8 sidecar");
    let dep_sidecar: serde_json::Value = serde_json::from_slice(&fs::read(&dep_sidecar_path).unwrap()).unwrap();
    let dep_reader = IndexReader::open(dep_sidecar["object_path"].as_str().unwrap()).unwrap();

    let add_idx = find_def_idx(&dep_reader, RDefKind::Macro, "dep_add");
    let add_mac = dep_reader.macro_def_for_def(add_idx).expect("dep_add MacroDefEntry");
    assert!(add_mac.is_macro_rules() && add_mac.is_exported() && add_mac.has_local_inner_macros() && !add_mac.is_builtin());
    assert_eq!(add_mac.delim, MacroDelim::Paren);
    assert_eq!(add_mac.edition, MacroEdition::Edition2018);
    assert!(add_mac.body.contains("dep_helper!") && add_mac.body.contains("$a"), "unexpected body: {}", add_mac.body);

    let gen_idx = find_def_idx(&dep_reader, RDefKind::Macro, "generated_tenx");
    let gen_mac = dep_reader.macro_def_for_def(gen_idx).expect("generated_tenx MacroDefEntry");
    assert!(gen_mac.is_macro_rules() && gen_mac.is_exported() && !gen_mac.has_local_inner_macros());
    assert_eq!(gen_mac.delim, MacroDelim::Bracket);
    assert!(gen_mac.body.contains("$item") && gen_mac.body.contains("10"), "meta-macro inner body must be extracted: {}", gen_mac.body);

    // 2. Workspace crate `ws_g8` (Edition 2024): private local macro_rules! + regular fn (not in macro_defs)
    let ws_src = ws_dir.join("lib.rs");
    fs::write(
        &ws_src,
        r#"
macro_rules! local_only {
    ($x:expr) => { dep_g8::dep_add!($x, 5) };
}

pub fn compute(v: i32) -> i32 {
    local_only!(v)
}
"#,
    )
    .unwrap();
    let ws_rmeta = temp_dir.join("libws_g8.rmeta");
    let ws_args = vec![
        "rustc".to_string(),
        ws_src.to_str().unwrap().to_string(),
        "--crate-name".to_string(),
        "ws_g8".to_string(),
        "--crate-type".to_string(),
        "lib".to_string(),
        "--edition=2024".to_string(),
        "--extern".to_string(),
        format!("dep_g8={}", dep_rmeta.display()),
        "--emit=metadata".to_string(),
        "-o".to_string(),
        ws_rmeta.to_str().unwrap().to_string(),
        "--sysroot".to_string(),
        sysroot_str.clone(),
    ];
    let mut ws_env = DriverEnv::for_test(&out_dir);
    ws_env.cache_dir = cache_dir.clone();
    ws_env.manifest_dir = Some(ws_dir);
    assert!(run_driver_with_env(ws_args, ws_env).is_ok());

    let ws_ridx = fs::read_dir(&out_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.file_name().and_then(|s| s.to_str()).is_some_and(|s| s.starts_with("ws_g8") && s.ends_with(".ridx")))
        .expect("ws_g8 ridx");
    let ws_reader = IndexReader::open(&ws_ridx).unwrap();
    assert_eq!(ws_reader.macro_defs().len(), 1);
    let loc_idx = find_def_idx(&ws_reader, RDefKind::Macro, "local_only");
    let loc_mac = ws_reader.macro_def_for_def(loc_idx).expect("local_only MacroDefEntry");
    assert!(loc_mac.is_macro_rules() && !loc_mac.is_exported() && !loc_mac.has_local_inner_macros());
    assert_eq!(loc_mac.delim, MacroDelim::Brace);
    assert_eq!(loc_mac.edition, MacroEdition::Edition2024);
    assert!(loc_mac.body.contains("dep_add!"));

    let fn_idx = find_def_idx(&ws_reader, RDefKind::Fn, "compute");
    assert!(ws_reader.macro_def_for_def(fn_idx).is_none());

    // 3. Proc-macro crate `pm_g8`: procedural macro functions must NOT enter `macro_defs`
    let pm_src = pm_dir.join("lib.rs");
    fs::write(
        &pm_src,
        r#"
extern crate proc_macro;
use proc_macro::TokenStream;

#[proc_macro]
pub fn passthrough_pm(input: TokenStream) -> TokenStream {
    input
}
"#,
    )
    .unwrap();
    let pm_args = vec![
        "rustc".to_string(),
        pm_src.to_str().unwrap().to_string(),
        "--crate-name".to_string(),
        "pm_g8".to_string(),
        "--crate-type".to_string(),
        "proc-macro".to_string(),
        "--edition=2021".to_string(),
        "--emit=metadata".to_string(),
        "-o".to_string(),
        temp_dir.join("libpm_g8.rmeta").to_str().unwrap().to_string(),
        "--sysroot".to_string(),
        sysroot_str.clone(),
    ];
    let mut pm_env = DriverEnv::for_test(&out_dir);
    pm_env.cache_dir = cache_dir;
    pm_env.manifest_dir = Some(pm_dir);
    assert!(run_driver_with_env(pm_args, pm_env).is_ok());

    let pm_ridx = fs::read_dir(&out_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.file_name().and_then(|s| s.to_str()).is_some_and(|s| s.starts_with("pm_g8") && s.ends_with(".ridx")))
        .expect("pm_g8 ridx");
    let pm_reader = IndexReader::open(&pm_ridx).unwrap();
    assert!(pm_reader.macro_defs().is_empty(), "proc-macro fns must not be recorded in declarative macro_defs");

    // 4. Sysroot `.rmeta` extraction (`core`, `alloc`, `std` re-exports + builtin flags)
    struct SysrootG8Check(bool);
    impl Callbacks for SysrootG8Check {
        fn after_analysis<'tcx>(&mut self, _c: &rustc_interface::interface::Compiler, tcx: TyCtxt<'tcx>) -> Compilation {
            let open_sys = |name: &str| {
                let (mut w, _) = sysroot::extract_sysroot_crate(tcx, name, None).unwrap();
                let mut bytes = Vec::new();
                w.write_to(&mut std::io::Cursor::new(&mut bytes)).unwrap();
                IndexReader::from_buffer(bytes).unwrap()
            };

            let core_r = open_sys("core");
            let matches_idx = find_def_idx(&core_r, RDefKind::Macro, "core::matches");
            let matches_mac = core_r.macro_def_for_def(matches_idx).expect("core::matches MacroDefEntry");
            assert!(matches_mac.is_macro_rules() && !matches_mac.is_builtin());
            assert!(matches_mac.body.contains("match"), "core::matches body: {}", matches_mac.body);

            let fmt_args_idx = find_def_idx(&core_r, RDefKind::Macro, "core::format_args");
            let fmt_args_mac = core_r.macro_def_for_def(fmt_args_idx).expect("core::format_args MacroDefEntry");
            assert!(fmt_args_mac.is_builtin(), "core::format_args must carry MACRO_FLAG_BUILTIN");

            let alloc_r = open_sys("alloc");
            let alloc_vec_idx = find_def_idx(&alloc_r, RDefKind::Macro, "alloc::vec");
            let alloc_vec_mac = alloc_r.macro_def_for_def(alloc_vec_idx).expect("alloc::vec MacroDefEntry");
            assert!(alloc_vec_mac.is_macro_rules() && !alloc_vec_mac.is_builtin() && !alloc_vec_mac.body.is_empty());

            let std_r = open_sys("std");
            let std_vec_idx = find_def_idx(&std_r, RDefKind::Macro, "std::vec");
            let std_vec_mac = std_r.macro_def_for_def(std_vec_idx).expect("std::vec re-export MacroDefEntry");
            assert_eq!(std_vec_mac.body, alloc_vec_mac.body);

            let std_matches_idx = find_def_idx(&std_r, RDefKind::Macro, "std::matches");
            let std_matches_mac = std_r.macro_def_for_def(std_matches_idx).expect("std::matches re-export MacroDefEntry");
            assert_eq!(std_matches_mac.body, matches_mac.body);

            self.0 = true;
            Compilation::Stop
        }
    }
    let probe_src = temp_dir.join("probe_sys_g8.rs");
    fs::write(&probe_src, "pub fn noop() {}\n").unwrap();
    let mut chk = SysrootG8Check(false);
    rustc_driver::run_compiler(
        &[
            "rustc".into(),
            probe_src.to_str().unwrap().into(),
            "--crate-name".into(),
            "probe_sys_g8".into(),
            "--crate-type".into(),
            "lib".into(),
            "--edition=2021".into(),
            "--emit=metadata".into(),
            "-o".into(),
            temp_dir.join("libprobe_sys_g8.rmeta").to_str().unwrap().into(),
            "--sysroot".into(),
            sysroot_str,
        ],
        &mut chk,
    );
    assert!(chk.0);
    let _ = fs::remove_dir_all(&temp_dir);
}
