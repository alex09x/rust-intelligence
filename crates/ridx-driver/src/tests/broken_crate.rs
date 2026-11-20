/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! A crate with compile errors still gets an index of everything that resolved.

use super::*;

#[test]
fn test_driver_indexes_a_crate_that_does_not_compile() {
    let _lock = DRIVER_TEST_LOCK.lock().unwrap();
    let temp_dir = std::env::temp_dir().join(format!("ridx_driver_broken_crate_{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp_dir);
    let ws_dir = temp_dir.join("ws_broken");
    fs::create_dir_all(&ws_dir).unwrap();
    let src_file = ws_dir.join("lib.rs");
    // Line 3 has an unresolved name (E0433) and line 4 a type mismatch (E0308); both still call `helper`.
    let code = "pub fn helper(x: i32) -> i32 { x + 1 }\n\
pub fn caller() -> i32 { helper(1) + helper(2) }\n\
pub fn unresolved() -> usize { let m = BTreeMap::new(); m.len() + helper(3) as usize }\n\
pub fn mismatched() -> bool { helper(4) }\n";
    fs::write(&src_file, code).unwrap();
    let out_dir = temp_dir.join("out");
    fs::create_dir_all(out_dir.join(".deps")).unwrap();
    let rmeta_file = temp_dir.join("libws_broken.rmeta");
    let sysroot = std::process::Command::new("rustc").args(["--print", "sysroot"]).output().unwrap();
    let args = vec![
        "rustc".to_string(),
        src_file.to_str().unwrap().to_string(),
        "--crate-name".to_string(),
        "ws_broken".to_string(),
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
    // The compiler still reports the crate as failed.
    assert!(run_driver_with_env(args, env).is_err());

    let out_ridx = fs::read_dir(&out_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|ext| ext == "ridx"))
        .expect("a broken crate still gets a .ridx index");
    let reader = ridx_format::IndexReader::open(&out_ridx).unwrap();
    let mut helper_lines: Vec<u32> = reader.references().iter().filter(|r| reader.get_str(r.target_path_offset()).ends_with("helper")).map(|r| r.span().start_line()).collect();
    helper_lines.sort_unstable();
    assert_eq!(helper_lines, vec![2, 2, 3, 4], "calls to helper, including those in the bodies with errors");
    let paths: Vec<&str> = reader.definitions().iter().map(|d| reader.get_str(d.path_offset())).collect();
    for name in ["helper", "caller", "unresolved", "mismatched"] {
        assert!(paths.iter().any(|p| p.ends_with(name)), "definition {name} in {paths:?}");
    }
    let _ = fs::remove_dir_all(&temp_dir);
}
