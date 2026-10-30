/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use super::*;

#[test]
fn test_driver_generic_supertrait_and_where_clause() {
    let _lock = DRIVER_TEST_LOCK.lock().unwrap();
    let temp_dir = std::env::temp_dir().join(format!("ridx_driver_gen2_{}", std::process::id()));
    let _ = fs::create_dir_all(&temp_dir);
    let ws_dir = temp_dir.join("ws_gen2");
    fs::create_dir_all(&ws_dir).unwrap();
    let src_file = ws_dir.join("lib.rs");
    let code = r#"
pub mod de {
    pub trait Visitor {
        fn visit_str(&mut self, s: &str);
    }
}
pub fn test_where<V>(mut v: V) where V: de::Visitor {
    v.visit_str("hello");
}

pub trait BaseTrait {
    fn base_method(&self);
}
pub trait SecondBaseTrait {
    fn second_method(&self);
}
pub trait SubTrait: BaseTrait + SecondBaseTrait {
    fn sub_method(&self);
}
pub fn test_sub<S: SubTrait>(s: S) {
    s.sub_method();
}
"#;
    fs::write(&src_file, code).unwrap();
    let out_dir = temp_dir.join("out");
    fs::create_dir_all(&out_dir).unwrap();
    let deps_dir = out_dir.join(".deps");
    fs::create_dir_all(&deps_dir).unwrap();

    let rmeta_file = temp_dir.join("libws_gen2.rmeta");
    let sysroot = std::process::Command::new("rustc").args(["--print", "sysroot"]).output().unwrap();
    let sysroot_str = String::from_utf8_lossy(&sysroot.stdout).trim().to_string();

    let args = vec![
        "rustc".to_string(),
        src_file.to_str().unwrap().to_string(),
        "--crate-name".to_string(),
        "ws_gen2".to_string(),
        "--crate-type".to_string(),
        "lib".to_string(),
        "--emit=metadata".to_string(),
        "-o".to_string(),
        rmeta_file.to_str().unwrap().to_string(),
        "--sysroot".to_string(),
        sysroot_str,
    ];

    std::env::set_var("RIDX_OUT_DIR", &out_dir);
    std::env::set_var("CARGO_MANIFEST_DIR", &ws_dir);
    std::env::set_var("CARGO_PKG_VERSION", "0.1.0");

    let res = run_driver(args);
    assert!(res.is_ok());

    let out_ridx = fs::read_dir(&out_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|ext| ext == "ridx"))
        .expect("expected .ridx index to exist in out_dir");

    let reader = ridx_format::IndexReader::open(&out_ridx).unwrap();

    // 1. Where clause: V bounded by de::Visitor
    let types = reader.types();
    let v_types: Vec<_> = types.iter().filter(|t| reader.get_str(t.type_offset) == "V").collect();
    assert!(!v_types.is_empty(), "expected type entry for V");
    assert!(
        v_types.iter().any(|t| reader.get_str(t.owner_path_offset) == "de::Visitor"),
        "where-clause generic V must resolve owner to de::Visitor, got {:?}",
        v_types.iter().map(|t| reader.get_str(t.owner_path_offset)).collect::<Vec<_>>()
    );

    // 2. Generic bound S: SubTrait
    let s_types: Vec<_> = types.iter().filter(|t| reader.get_str(t.type_offset) == "S").collect();
    assert!(!s_types.is_empty(), "expected type entry for S");
    assert!(s_types.iter().any(|t| reader.get_str(t.owner_path_offset) == "SubTrait"), "generic parameter S must resolve owner to SubTrait");

    // 3. Multiple supertraits deref chain: SubTrait -> BaseTrait AND SecondBaseTrait
    let targets = reader.find_deref_targets("SubTrait");
    assert!(targets.contains(&"BaseTrait"), "SubTrait must declare target BaseTrait");
    assert!(targets.contains(&"SecondBaseTrait"), "SubTrait must declare target SecondBaseTrait");
    let chain = reader.resolve_deref_chain("SubTrait", 4);
    assert!(chain.contains(&"BaseTrait"), "BFS chain must contain BaseTrait");
    assert!(chain.contains(&"SecondBaseTrait"), "BFS chain must contain SecondBaseTrait");

    std::env::remove_var("RIDX_OUT_DIR");
    std::env::remove_var("CARGO_MANIFEST_DIR");
    std::env::remove_var("CARGO_PKG_VERSION");
    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_workspace_trait_impl_links() {
    let _lock = DRIVER_TEST_LOCK.lock().unwrap();
    let temp_dir = std::env::temp_dir().join(format!("ridx_ws_trait_test_{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).unwrap();

    let ws_dir = temp_dir.join("ws");
    fs::create_dir_all(&ws_dir).unwrap();
    let src_file = ws_dir.join("lib.rs");
    let code = r#"
pub struct MyIter(pub usize);
impl Iterator for MyIter {
    type Item = usize;
    fn next(&mut self) -> Option<Self::Item> {
        if self.0 > 0 { self.0 -= 1; Some(self.0) } else { None }
    }
}
pub struct MyDisplay;
impl std::fmt::Display for MyDisplay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MyDisplay")
    }
}
"#;
    fs::write(&src_file, code).unwrap();
    let out_dir = temp_dir.join("out");
    fs::create_dir_all(&out_dir).unwrap();
    let deps_dir = out_dir.join(".deps");
    fs::create_dir_all(&deps_dir).unwrap();

    let rmeta_file = temp_dir.join("libws_trait.rmeta");
    let sysroot = std::process::Command::new("rustc").args(["--print", "sysroot"]).output().unwrap();
    let sysroot_str = String::from_utf8_lossy(&sysroot.stdout).trim().to_string();

    let args = vec![
        "rustc".to_string(),
        src_file.to_str().unwrap().to_string(),
        "--crate-name".to_string(),
        "ws_trait".to_string(),
        "--crate-type".to_string(),
        "lib".to_string(),
        "--emit=metadata".to_string(),
        "-o".to_string(),
        rmeta_file.to_str().unwrap().to_string(),
        "--sysroot".to_string(),
        sysroot_str,
    ];

    std::env::set_var("RIDX_OUT_DIR", &out_dir);
    std::env::set_var("CARGO_MANIFEST_DIR", &ws_dir);
    std::env::set_var("CARGO_PKG_VERSION", "0.1.0");

    let res = run_driver(args);
    assert!(res.is_ok());

    let out_ridx = fs::read_dir(&out_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|ext| ext == "ridx"))
        .expect("expected .ridx index to exist in out_dir");

    let reader = ridx_format::IndexReader::open(&out_ridx).unwrap();

    // Verify MyIter has deref/trait link to Iterator
    let iter_targets = reader.find_deref_targets("MyIter");
    assert!(iter_targets.iter().any(|&t| t.contains("Iterator")), "MyIter must link to Iterator, got: {:?}", iter_targets);

    // Verify MyDisplay has deref/trait link to Display
    let disp_targets = reader.find_deref_targets("MyDisplay");
    assert!(disp_targets.iter().any(|&t| t.contains("Display")), "MyDisplay must link to Display, got: {:?}", disp_targets);

    std::env::remove_var("RIDX_OUT_DIR");
    std::env::remove_var("CARGO_MANIFEST_DIR");
    std::env::remove_var("CARGO_PKG_VERSION");
    let _ = fs::remove_dir_all(&temp_dir);
}
