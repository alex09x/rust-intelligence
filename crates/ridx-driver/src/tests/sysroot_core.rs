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
fn test_sysroot_core_metadata_prototype() {
    let _lock = DRIVER_TEST_LOCK.lock().unwrap();
    struct SysrootTestCallbacks {
        crates_stats: Vec<sysroot::SysrootExtractStats>,
        crates_data: Vec<(String, Vec<u8>)>,
        cas_key: String,
    }

    impl Callbacks for SysrootTestCallbacks {
        fn after_analysis<'tcx>(&mut self, _compiler: &rustc_interface::interface::Compiler, tcx: TyCtxt<'tcx>) -> Compilation {
            self.cas_key = sysroot::compute_sysroot_cas_key(tcx);
            let rust_src = sysroot::discover_rust_src_path(tcx);
            for cname in ["core", "alloc", "std"] {
                if let Some((mut writer, mut stats)) = sysroot::extract_sysroot_crate(tcx, cname, rust_src.as_deref()) {
                    let mut cursor = std::io::Cursor::new(Vec::new());
                    writer.write_to(&mut cursor).expect("failed to write sysroot index");
                    let buf = cursor.into_inner();
                    stats.index_bytes = buf.len();
                    self.crates_data.push((cname.to_string(), buf));
                    self.crates_stats.push(stats);
                }
            }
            Compilation::Continue
        }
    }

    let temp_dir = std::env::temp_dir().join(format!("ridx_sysroot_proto_{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).unwrap();
    let src_file = temp_dir.join("lib.rs");
    fs::write(&src_file, "pub fn dummy() {}\n").unwrap();
    let rmeta_file = temp_dir.join("libdummy.rmeta");

    let sysroot = std::process::Command::new("rustc").args(["--print", "sysroot"]).output().unwrap();
    let sysroot_str = String::from_utf8_lossy(&sysroot.stdout).trim().to_string();

    let args = vec![
        "rustc".to_string(),
        src_file.to_str().unwrap().to_string(),
        "--crate-name".to_string(),
        "dummy".to_string(),
        "--crate-type".to_string(),
        "lib".to_string(),
        "--emit=metadata".to_string(),
        "-o".to_string(),
        rmeta_file.to_str().unwrap().to_string(),
        "--sysroot".to_string(),
        sysroot_str,
    ];

    let mut callbacks = SysrootTestCallbacks { crates_stats: Vec::new(), crates_data: Vec::new(), cas_key: String::new() };

    let res = run_with_callbacks(&args, &mut callbacks);
    assert!(res.is_ok(), "compiler run must succeed");
    assert!(!callbacks.crates_stats.is_empty(), "must extract at least core sysroot crate");
    assert!(!callbacks.cas_key.is_empty(), "CAS key must be generated");

    // Write shards to disk for testing mmap and readers
    for (cname, data) in &callbacks.crates_data {
        let path = temp_dir.join(format!("{cname}.ridx"));
        fs::write(&path, data).unwrap();
    }

    // Measure cold-start mmap latency across all 3 shards
    let mmap_start = std::time::Instant::now();
    let mut readers = Vec::new();
    for (cname, _) in &callbacks.crates_data {
        let path = temp_dir.join(format!("{cname}.ridx"));
        let r = ridx_format::IndexReader::open(&path).expect("failed to mmap sysroot shard");
        readers.push(r);
    }
    let mmap_latency_us = mmap_start.elapsed().as_micros();

    let core_reader = readers.iter().find(|r| r.unit_key().starts_with("core") || r.types().is_empty()).unwrap();
    let defs = core_reader.definitions();

    // Verify Option::is_some
    let is_some_def = defs.iter().find(|d| core_reader.get_str(d.name_offset) == "is_some");
    assert!(is_some_def.is_some(), "is_some method must exist in core index");

    // Verify Iterator::next
    let next_def = defs.iter().find(|d| core_reader.get_str(d.name_offset) == "next" && core_reader.get_str(d.path_offset).contains("Iterator"));
    assert!(next_def.is_some(), "Iterator::next method must exist in core index");

    // Verify primitive inherent impls: str::len with owner "str"
    let str_defs = core_reader.find_definitions_by_owner("str");
    let has_len = str_defs.iter().any(|d| core_reader.get_str(d.name_offset) == "len");
    assert!(has_len, "str owner must contain len method via find_definitions_by_owner('str')");

    let derefs = core_reader.deref_entries();
    assert!(!derefs.is_empty(), "core index must contain deref/supertrait entries");

    // Verify Option trait implementation links (Option -> Clone, Option -> IntoIterator)
    let opt_targets = core_reader.find_deref_targets("core::option::Option");
    let has_clone = opt_targets.iter().any(|&t| t.contains("Clone"));
    let has_into_iter = opt_targets.iter().any(|&t| t.contains("IntoIterator"));
    assert!(has_clone, "Option must have Deref/trait link to Clone, got: {:?}", opt_targets);
    assert!(has_into_iter, "Option must have Deref/trait link to IntoIterator, got: {:?}", opt_targets);

    // Verify alloc shard: Vec -> slice (Deref), Vec -> Extend (trait), String -> str (Deref)
    if let Some(alloc_reader) = readers.iter().find(|r| r.unit_key().starts_with("alloc")) {
        let vec_targets = alloc_reader.find_deref_targets("alloc::vec::Vec");
        assert!(vec_targets.contains(&"slice"), "Vec must have Deref link to slice, got: {:?}", vec_targets);
        let has_extend = vec_targets.iter().any(|&t| t.contains("Extend"));
        assert!(has_extend, "Vec must have trait link to Extend, got: {:?}", vec_targets);

        let string_targets = alloc_reader.find_deref_targets("alloc::string::String");
        assert!(string_targets.contains(&"str"), "String must have Deref link to str, got: {:?}", string_targets);

        let vec_chain = alloc_reader.resolve_deref_chain("alloc::vec::Vec", 8);
        assert!(vec_chain.contains(&"slice"), "Vec deref chain must contain slice: {:?}", vec_chain);

        let str_chain = alloc_reader.resolve_deref_chain("alloc::string::String", 8);
        assert!(str_chain.contains(&"str"), "String deref chain must contain str: {:?}", str_chain);
    }

    if let Some(std_reader) = readers.iter().find(|r| r.unit_key().starts_with("std")) {
        let f64_defs = std_reader.find_definitions_by_owner("f64");
        assert!(!f64_defs.is_empty(), "std must contain f64 inherent methods like sqrt/sin");
    }

    // Verify WorkspaceIndex automatic discovery and attachment of sysroot shards
    let test_cache_dir = temp_dir.join("test_cache");
    let cas_dir = test_cache_dir.join("sysroot").join(&callbacks.cas_key);
    fs::create_dir_all(&cas_dir).unwrap();
    for (cname, data) in &callbacks.crates_data {
        fs::write(cas_dir.join(format!("{cname}.ridx")), data).unwrap();
    }

    let proj_dir = temp_dir.join("my_proj");
    fs::create_dir_all(&proj_dir).unwrap();
    let mut proj_writer = IndexWriter::new("my_proj", "0.1.0");
    proj_writer.set_unit_key("my_proj-0.1.0");
    let mut proj_buf = Vec::new();
    proj_writer.write_to(&mut std::io::Cursor::new(&mut proj_buf)).unwrap();
    fs::write(proj_dir.join("my_proj.ridx"), &proj_buf).unwrap();

    let mut ws = ridx_format::workspace::WorkspaceIndex::open(&proj_dir).expect("WorkspaceIndex::open must succeed");
    ws.attach_sysroot_shards_for_key_in(&test_cache_dir.join("sysroot"), &callbacks.cas_key);

    let ws_str_defs = ws.find_definitions_by_owner("str");
    assert!(ws_str_defs.iter().any(|d| d.name == "len"), "WorkspaceIndex must find str::len from sysroot");
    let ws_vec_chain = ws.resolve_deref_chain("alloc::vec::Vec", 8);
    assert!(ws_vec_chain.contains(&"slice"), "WorkspaceIndex must resolve Vec -> slice via deref chain");
    let ws_opt_chain = ws.resolve_deref_chain("core::option::Option", 8);
    assert!(ws_opt_chain.iter().any(|&t| t.contains("Clone")), "WorkspaceIndex must find Option: Clone link in deref chain");

    // Verify type alias inherent method resolution (AtomicBool, AtomicUsize, NonZeroUsize)
    let atomic_bool_defs = core_reader.find_definitions_by_owner("core::sync::atomic::AtomicBool");
    assert!(atomic_bool_defs.iter().any(|d| core_reader.get_str(d.name_offset) == "load"), "AtomicBool owner must contain load method");
    assert!(atomic_bool_defs.iter().any(|d| core_reader.get_str(d.name_offset) == "store"), "AtomicBool owner must contain store method");
    let atomic_bool_load = atomic_bool_defs.iter().find(|d| core_reader.get_str(d.name_offset) == "load").unwrap();
    assert_eq!(core_reader.get_str(atomic_bool_load.path_offset), "core::sync::atomic::Atomic::<bool>::load", "AtomicBool::load def_path must remain canonical rustc path");

    let atomic_usize_defs = core_reader.find_definitions_by_owner("core::sync::atomic::AtomicUsize");
    assert!(atomic_usize_defs.iter().any(|d| core_reader.get_str(d.name_offset) == "fetch_add"), "AtomicUsize owner must contain fetch_add method");

    let nonzero_usize_defs = core_reader.find_definitions_by_owner("core::num::nonzero::NonZeroUsize");
    assert!(nonzero_usize_defs.iter().any(|d| core_reader.get_str(d.name_offset) == "leading_zeros"), "NonZeroUsize owner must contain leading_zeros method");
    assert!(nonzero_usize_defs.iter().any(|d| core_reader.get_str(d.name_offset) == "get"), "NonZeroUsize owner must contain get method");

    let ws_nonzero_get = ws.find_definition_by_path("core::num::nonzero::NonZeroUsize::get").expect("NonZeroUsize::get must resolve via WorkspaceIndex owner fallback");
    assert_eq!(ws_nonzero_get.name, "get");

    let ws_atomic_load = ws.find_definition_by_path("core::sync::atomic::AtomicBool::load").expect("AtomicBool::load must resolve via WorkspaceIndex owner fallback");
    assert_eq!(ws_atomic_load.name, "load");
    assert_eq!(ws_atomic_load.path, "core::sync::atomic::Atomic::<bool>::load");

    let ws_atomic_store = ws.find_definition_by_path("core::sync::atomic::AtomicBool::store").expect("AtomicBool::store must resolve via WorkspaceIndex owner fallback");
    assert_eq!(ws_atomic_store.name, "store");
    assert_eq!(ws_atomic_store.path, "core::sync::atomic::Atomic::<bool>::store");

    let ws_atomic_usize_load = ws.find_definition_by_path("core::sync::atomic::AtomicUsize::load").expect("AtomicUsize::load must resolve via WorkspaceIndex owner fallback");
    assert_eq!(ws_atomic_usize_load.name, "load");
    assert_eq!(ws_atomic_usize_load.path, "core::sync::atomic::Atomic::<usize>::load");

    // Format metrics JSON for reporting
    let mut stats_json_parts = Vec::new();
    for s in &callbacks.crates_stats {
        stats_json_parts.push(format!(
            "{{\"crate\":\"{}\",\"definitions\":{},\"derefs\":{},\"elapsed_ms\":{:.2},\"shard_bytes\":{}}}",
            s.crate_name, s.total_definitions, s.total_derefs, s.elapsed_ms, s.index_bytes
        ));
    }
    let all_metrics = format!("{{\"cas_key\":\"{}\",\"mmap_cold_start_us\":{},\"crates\":[{}]}}", callbacks.cas_key, mmap_latency_us, stats_json_parts.join(","));
    let _ = fs::write("crates/ridx-driver/proto_stats.json", &all_metrics);

    println!("\n=== SYSROOT METADATA EXTRACTION PROTOYPE METRICS ===");
    println!("  CAS Key:               {}", callbacks.cas_key);
    println!("  Cold start mmap (3x):  {} µs", mmap_latency_us);
    for s in &callbacks.crates_stats {
        println!("  [{}] defs: {}, derefs: {}, time: {:.2} ms, size: {} KB", s.crate_name, s.total_definitions, s.total_derefs, s.elapsed_ms, s.index_bytes / 1024);
    }

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_sysroot_completeness_audit() {
    let _lock = DRIVER_TEST_LOCK.lock().unwrap();
    struct CheckAuditCallbacks {
        report: Option<crate::sysroot::audit::SysrootCompletenessReport>,
    }
    impl rustc_driver::Callbacks for CheckAuditCallbacks {
        fn after_analysis<'tcx>(&mut self, _c: &rustc_interface::interface::Compiler, tcx: rustc_middle::ty::TyCtxt<'tcx>) -> rustc_driver::Compilation {
            self.report = Some(crate::sysroot::audit::audit_sysroot_completeness(tcx));
            rustc_driver::Compilation::Stop
        }
    }

    let temp_dir = std::env::temp_dir().join(format!("ridx_sysroot_audit_test_{}", std::process::id()));
    let _ = fs::create_dir_all(&temp_dir);
    let dummy_rs = temp_dir.join("lib.rs");
    fs::write(&dummy_rs, "extern crate alloc;\n").unwrap();
    let sysroot_out = std::process::Command::new("rustc").args(["--print", "sysroot"]).output().unwrap();
    let sysroot_path = String::from_utf8(sysroot_out.stdout).unwrap().trim().to_string();
    let rmeta = temp_dir.join("libaudit_test.rmeta");
    let args = vec![
        "rustc".to_string(),
        dummy_rs.to_string_lossy().into_owned(),
        "--crate-name".to_string(),
        "audit_test".to_string(),
        "--crate-type".to_string(),
        "lib".to_string(),
        "--emit=metadata".to_string(),
        "-o".to_string(),
        rmeta.to_string_lossy().into_owned(),
        "--sysroot".to_string(),
        sysroot_path,
    ];
    let mut cb = CheckAuditCallbacks { report: None };
    let res = crate::run_with_callbacks(&args, &mut cb);
    let _ = fs::remove_dir_all(&temp_dir);
    assert!(res.is_ok(), "sysroot completeness audit compiler run must succeed");

    let rep = cb.report.expect("audit callback must produce report");
    assert_eq!(rep.crates.len(), 3, "must audit core, alloc, std");
    let total_ridx: usize = rep.crates.iter().map(|c| c.ridx_total_defs).sum();
    let total_impls_missing: usize = rep.crates.iter().map(|c| c.trait_impl_blocks_missing).sum();
    let total_fields_missing: usize = rep.crates.iter().map(|c| c.pub_named_fields_missing + c.pub_tuple_fields_missing).sum();
    assert!(total_ridx > 7_000, "expected >7000 sysroot ridx defs, got {total_ridx}");
    assert!(total_impls_missing > 2_000, "expected >2000 missing trait impl blocks, got {total_impls_missing}");
    assert!(total_fields_missing > 50, "expected >50 missing public fields, got {total_fields_missing}");
    for required in ["core::ops", "core::fmt", "core::iter", "alloc::collections", "std::io", "std::collections"] {
        assert!(rep.families.iter().any(|f| f.family == required), "missing family {required}");
    }
}
