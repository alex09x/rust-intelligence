/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! A trait function named by a path in a body (`From::from(x)`) refers to the implementation its arguments select;
//! paths a derive or a desugaring wrote (`for`, `?`) keep naming the trait's function.

use super::*;

#[test]
fn test_driver_trait_fn_paths_refer_to_the_selected_impl() {
    let _lock = DRIVER_TEST_LOCK.lock().unwrap();
    let temp_dir = std::env::temp_dir().join(format!("ridx_driver_trait_fn_paths_{}", std::process::id()));
    let ws_dir = temp_dir.join("ws_tf");
    fs::create_dir_all(&ws_dir).unwrap();
    let src_file = ws_dir.join("lib.rs");
    let code = "pub struct A;\n\
impl From<u8> for A {\n    fn from(_: u8) -> A {\n        A\n    }\n}\n\
pub fn make() -> A {\n    From::from(1u8)\n}\n\
pub fn sum(v: Vec<u8>) -> u32 {\n    let mut n = 0;\n    for x in v {\n        n += x as u32;\n    }\n    n\n}\n\
pub fn first(x: Option<u8>) -> Option<u8> {\n    let y = x?;\n    Some(y)\n}\n\
#[derive(Default)]\npub struct D {\n    n: u8,\n}\n";
    fs::write(&src_file, code).unwrap();
    let out_dir = temp_dir.join("out");
    fs::create_dir_all(out_dir.join(".deps")).unwrap();
    let rmeta_file = temp_dir.join("libws_tf.rmeta");
    let sysroot = std::process::Command::new("rustc").args(["--print", "sysroot"]).output().unwrap();
    let args = vec![
        "rustc".to_string(),
        src_file.to_str().unwrap().to_string(),
        "--crate-name".to_string(),
        "ws_tf".to_string(),
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
    // `From::from(1u8)` on line 8: the `from` segment names `A`'s implementation, not the trait's declaration.
    let targets: Vec<&str> = reader.references().iter().filter(|r| r.span().start_line() == 8).map(|r| reader.get_str(r.target_path_offset())).collect();
    assert!(targets.contains(&"<A as core::convert::From<u8>>::from"), "From::from target: {targets:?}");
    assert!(!targets.iter().any(|t| t.ends_with("From::from")), "trait item still named: {targets:?}");
    // The derive's `Default::default()` for the field (line 21) names no implementation.
    let derived: Vec<&str> = reader.references().iter().filter(|r| r.span().start_line() == 21).map(|r| reader.get_str(r.target_path_offset())).collect();
    assert!(!derived.iter().any(|t| t.starts_with("<u8 as")), "derive resolved to an impl: {derived:?}");
    let _ = fs::remove_dir_all(&temp_dir);
}
