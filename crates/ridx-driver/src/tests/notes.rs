/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Notes: generic arguments at calls and memory layouts at names, in rust-analyzer's wording.

use super::*;
use ridx_format::{NOTE_CALL_ARGS, NOTE_LAYOUT};

#[test]
fn test_driver_writes_call_args_and_layout_notes() {
    let _lock = DRIVER_TEST_LOCK.lock().unwrap();
    let temp_dir = std::env::temp_dir().join(format!("ridx_driver_notes_{}", std::process::id()));
    let ws_dir = temp_dir.join("ws_notes");
    fs::create_dir_all(&ws_dir).unwrap();
    let src_file = ws_dir.join("lib.rs");
    let code = "pub struct Circle {\n    pub radius: f64,\n}\n\
pub fn render<T: std::fmt::Display>(value: &T) -> String {\n    format!(\"{value}\")\n}\n\
pub fn run() -> usize {\n    let names: Vec<String> = Vec::new();\n    render(&1.5f64).len() + names.len()\n}\n";
    fs::write(&src_file, code).unwrap();
    let out_dir = temp_dir.join("out");
    fs::create_dir_all(out_dir.join(".deps")).unwrap();
    let rmeta_file = temp_dir.join("libws_notes.rmeta");
    let sysroot = std::process::Command::new("rustc").args(["--print", "sysroot"]).output().unwrap();
    let args = vec![
        "rustc".to_string(),
        src_file.to_str().unwrap().to_string(),
        "--crate-name".to_string(),
        "ws_notes".to_string(),
        "--crate-type".to_string(),
        "lib".to_string(),
        "--edition=2021".to_string(),
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
    let file_id = reader.find_file_id("lib.rs").or_else(|| reader.find_file_id(src_file.to_str().unwrap())).expect("file id");
    let all: Vec<(u32, u32, u32, &str)> = reader.notes().iter().map(|n| (n.span().start_line(), n.span().start_col(), n.kind(), reader.get_str(n.text_offset()))).collect();
    assert_eq!(reader.note_at(file_id, 1, 12, NOTE_LAYOUT), Some("size = 8, align = 0x8, no Drop"), "struct layout; notes: {all:?}");
    assert_eq!(reader.note_at(file_id, 2, 9, NOTE_LAYOUT), Some("size = 8, align = 0x8, offset = 0x0, no Drop"), "field layout; notes: {all:?}");
    assert_eq!(reader.note_at(file_id, 9, 5, NOTE_CALL_ARGS), Some("`T` = `f64`"), "free fn call; notes: {all:?}");
    assert_eq!(reader.note_at(file_id, 8, 35, NOTE_CALL_ARGS), Some("`T` = `String`"), "associated fn call; notes: {all:?}");
    assert_eq!(reader.note_at(file_id, 8, 9, NOTE_LAYOUT), Some("size = 24 (0x18), align = 0x8, needs Drop"), "binding layout; notes: {all:?}");
    let _ = fs::remove_dir_all(&temp_dir);
}
