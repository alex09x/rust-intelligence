/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! `Self::Variant` in patterns and struct literals is a reference to the variant.

use super::*;

#[test]
fn test_driver_self_variant_paths_in_patterns_are_references() {
    let _lock = DRIVER_TEST_LOCK.lock().unwrap();
    let temp_dir = std::env::temp_dir().join(format!("ridx_driver_self_variant_{}", std::process::id()));
    let ws_dir = temp_dir.join("ws_sv");
    fs::create_dir_all(&ws_dir).unwrap();
    let src_file = ws_dir.join("lib.rs");
    let code = "pub enum Kind {\n    Round,\n    Pair(u8, u8),\n    Cornered { corners: u32 },\n}\n\
impl Kind {\n    pub fn corners(&self) -> u32 {\n        match self {\n            Self::Round => 0,\n            Self::Pair(_, _) => 2,\n            Self::Cornered { corners } => *corners,\n        }\n    }\n\
    pub fn square() -> Self {\n        Self::Cornered { corners: 4 }\n    }\n}\n\
pub fn fmt_error(_: core::fmt::Error) {}\n";
    fs::write(&src_file, code).unwrap();
    let out_dir = temp_dir.join("out");
    fs::create_dir_all(out_dir.join(".deps")).unwrap();
    let rmeta_file = temp_dir.join("libws_sv.rmeta");
    let sysroot = std::process::Command::new("rustc").args(["--print", "sysroot"]).output().unwrap();
    let args = vec![
        "rustc".to_string(),
        src_file.to_str().unwrap().to_string(),
        "--crate-name".to_string(),
        "ws_sv".to_string(),
        "--crate-type".to_string(),
        "lib".to_string(),
        "--emit=metadata".to_string(),
        "-o".to_string(),
        rmeta_file.to_str().unwrap().to_string(),
        "--sysroot".to_string(),
        String::from_utf8_lossy(&sysroot.stdout).trim().to_string(),
    ];
    let mut env = DriverEnv::for_test(&out_dir);
    env.manifest_dir = Some(ws_dir.clone());
    env.pkg_version = "0.1.0".to_string();
    assert!(run_driver_with_env(args, env).is_ok());

    let out_ridx = fs::read_dir(&out_dir).unwrap().filter_map(|e| e.ok()).map(|e| e.path()).find(|p| p.extension().is_some_and(|ext| ext == "ridx")).expect("expected .ridx index");
    let reader = ridx_format::IndexReader::open(&out_ridx).unwrap();
    let variant_ref_lines = |name: &str| -> Vec<u32> {
        reader.references().iter().filter(|r| reader.get_str(r.target_path_offset()).ends_with(&format!("Kind::{name}"))).map(|r| r.span().start_line()).collect()
    };
    assert!(variant_ref_lines("Round").contains(&9), "Self::Round pattern: {:?}", variant_ref_lines("Round"));
    assert!(variant_ref_lines("Pair").contains(&10), "Self::Pair(..) pattern: {:?}", variant_ref_lines("Pair"));
    let cornered = variant_ref_lines("Cornered");
    assert!(cornered.contains(&11) && cornered.contains(&15), "Self::Cornered pattern and literal: {cornered:?}");
    // `Self` (in `-> Self` and `Self::Cornered`) refers to `Kind` itself.
    let kind_refs: Vec<u32> = reader
        .references()
        .iter()
        .filter(|r| reader.get_str(r.target_path_offset()).ends_with("ws_sv::Kind") || reader.get_str(r.target_path_offset()) == "Kind")
        .map(|r| r.span().start_line())
        .collect();
    assert!(kind_refs.contains(&14) && kind_refs.contains(&15), "Self as Kind: {kind_refs:?}");
    // `fmt` in `core::fmt::Error` refers to the module.
    let fmt_mod: Vec<u32> = reader
        .references()
        .iter()
        .filter(|r| reader.get_str(r.target_path_offset()).ends_with("fmt") && ridx_format::DefKind::from(r.kind) == ridx_format::DefKind::Mod)
        .map(|r| r.span().start_line())
        .collect();
    assert!(fmt_mod.contains(&18), "module segment: {fmt_mod:?}");
    let _ = fs::remove_dir_all(&temp_dir);
}
