/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Field names in enum variant literals and patterns are references to the variant's fields.

use super::*;

#[test]
fn test_driver_enum_variant_field_references() {
    let _lock = DRIVER_TEST_LOCK.lock().unwrap();
    let temp_dir = std::env::temp_dir().join(format!("ridx_driver_variant_fields_{}", std::process::id()));
    let ws_dir = temp_dir.join("ws_vf");
    fs::create_dir_all(&ws_dir).unwrap();
    let src_file = ws_dir.join("lib.rs");
    let code = "pub enum Kind {\n    Round,\n    Cornered { corners: u32 },\n}\n\
pub fn make() -> Kind {\n    Kind::Cornered { corners: 4 }\n}\n\
pub fn count(k: &Kind) -> u32 {\n    match k {\n        Kind::Cornered { corners: n } => *n,\n        Kind::Round => 0,\n    }\n}\n\
impl Kind {\n    pub fn square() -> Self {\n        Self::Cornered { corners: 4 }\n    }\n}\n";
    fs::write(&src_file, code).unwrap();
    let out_dir = temp_dir.join("out");
    fs::create_dir_all(out_dir.join(".deps")).unwrap();
    let rmeta_file = temp_dir.join("libws_vf.rmeta");
    let sysroot = std::process::Command::new("rustc").args(["--print", "sysroot"]).output().unwrap();
    let args = vec![
        "rustc".to_string(),
        src_file.to_str().unwrap().to_string(),
        "--crate-name".to_string(),
        "ws_vf".to_string(),
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
    let field_refs: Vec<u32> = reader
        .references()
        .iter()
        .filter(|r| reader.get_str(r.target_path_offset()).ends_with("corners") && ridx_format::DefKind::from(r.kind) == ridx_format::DefKind::Field)
        .map(|r| r.span().start_line())
        .collect();
    for line in [6, 10, 16] {
        assert!(field_refs.contains(&line), "expected a `corners` field reference on line {line}, got lines {field_refs:?}");
    }
    let _ = fs::remove_dir_all(&temp_dir);
}
