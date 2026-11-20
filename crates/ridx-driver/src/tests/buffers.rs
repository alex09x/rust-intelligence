/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! A source listed in the buffer list is compiled from its buffer's text, and its file is left as it is.

use super::*;

#[test]
fn test_driver_compiles_an_unsaved_buffer_instead_of_its_file() {
    let _lock = DRIVER_TEST_LOCK.lock().unwrap();
    let temp_dir = std::env::temp_dir().join(format!("ridx_driver_buffers_{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp_dir);
    let ws_dir = temp_dir.join("ws_buffers");
    fs::create_dir_all(&ws_dir).unwrap();
    let src_file = ws_dir.join("lib.rs");
    let saved = "pub fn helper(x: i32) -> i32 { x + 1 }\npub fn caller() -> i32 { helper(1) + helper(2) }\n";
    fs::write(&src_file, saved).unwrap();
    // The buffer adds two calls the saved file does not have.
    let buffer = temp_dir.join("buffer-lib.rs");
    fs::write(&buffer, format!("{saved}pub fn more() -> i32 {{ helper(5) + helper(6) }}\n")).unwrap();
    let list = temp_dir.join("buffers.json");
    fs::write(&list, serde_json::json!({ src_file.to_str().unwrap(): buffer.to_str().unwrap() }).to_string()).unwrap();

    let out_dir = temp_dir.join("out");
    fs::create_dir_all(out_dir.join(".deps")).unwrap();
    let sysroot = std::process::Command::new("rustc").args(["--print", "sysroot"]).output().unwrap();
    let args = vec![
        "rustc".to_string(),
        src_file.to_str().unwrap().to_string(),
        "--crate-name".to_string(),
        "ws_buffers".to_string(),
        "--crate-type".to_string(),
        "lib".to_string(),
        "--emit=metadata".to_string(),
        "-o".to_string(),
        temp_dir.join("libws_buffers.rmeta").to_str().unwrap().to_string(),
        "--sysroot".to_string(),
        String::from_utf8_lossy(&sysroot.stdout).trim().to_string(),
    ];
    let mut env = DriverEnv::for_test(&out_dir);
    env.manifest_dir = Some(ws_dir.clone());
    env.buffers = Some(list);
    assert!(run_driver_with_env(args, env).is_ok());

    let out_ridx = fs::read_dir(&out_dir).unwrap().filter_map(|e| e.ok()).map(|e| e.path()).find(|p| p.extension().is_some_and(|ext| ext == "ridx")).expect("a .ridx index");
    let reader = ridx_format::IndexReader::open(&out_ridx).unwrap();
    let mut helper_lines: Vec<u32> = reader.references().iter().filter(|r| reader.get_str(r.target_path_offset()).ends_with("helper")).map(|r| r.span().start_line()).collect();
    helper_lines.sort_unstable();
    assert_eq!(helper_lines, vec![2, 2, 3, 3], "the calls of the buffer, including the two it added");
    assert_eq!(fs::read_to_string(&src_file).unwrap(), saved, "the file on disk is not written");
    let _ = fs::remove_dir_all(&temp_dir);
}
