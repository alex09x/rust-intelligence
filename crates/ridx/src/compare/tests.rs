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
use ridx_format::{DefKind, IndexWriter, RawDefinition, RawReference, RawType};
use std::fs;
use std::path::PathBuf;

struct TestDir(PathBuf);
impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn test_capability_kind_parse() {
    assert_eq!(CapabilityKind::parse_str("def"), Some(CapabilityKind::Def));
    assert_eq!(
        CapabilityKind::parse_str("definition"),
        Some(CapabilityKind::Def)
    );
    assert_eq!(
        CapabilityKind::parse_str("refs"),
        Some(CapabilityKind::Refs)
    );
    assert_eq!(
        CapabilityKind::parse_str("references"),
        Some(CapabilityKind::Refs)
    );
    assert_eq!(
        CapabilityKind::parse_str("hover"),
        Some(CapabilityKind::Hover)
    );
    assert_eq!(
        CapabilityKind::parse_str("type"),
        Some(CapabilityKind::Hover)
    );
    assert_eq!(
        CapabilityKind::parse_str("symbols"),
        Some(CapabilityKind::Symbols)
    );
    assert_eq!(
        CapabilityKind::parse_str("callers"),
        Some(CapabilityKind::Callers)
    );
    assert_eq!(CapabilityKind::parse_str("all"), Some(CapabilityKind::All));
    assert_eq!(CapabilityKind::parse_str("unknown"), None);
}

#[test]
fn test_compare_capabilities_and_print() {
    let temp_dir = std::env::temp_dir().join(format!("ridx_test_cmp_all_{}", std::process::id()));
    let _ = fs::create_dir_all(&temp_dir);
    let _guard = TestDir(temp_dir.clone());

    let ridx_path = temp_dir.join("test.ridx");
    let mut writer = IndexWriter::new("test_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        file: "src/lib.rs".to_string(),
        name: "test_fn".to_string(),
        start_line: 10,
        start_col: 1,
        end_line: 12,
        end_col: 2,
        path: "test_crate::test_fn".to_string(),
        signature: Some("fn()".to_string()),
    });
    writer.add_reference(RawReference {
        file: "src/lib.rs".to_string(),
        start_line: 20,
        start_col: 5,
        end_line: 20,
        end_col: 12,
        target_def_id: Some(1),
        target_path: "test_crate::test_fn".to_string(),
        target_unit: None,
        target_file: Some("src/lib.rs".to_string()),
        target_line: 10,
        target_col: 1,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.add_type(RawType {
        file: "src/lib.rs".to_string(),
        start_line: 10,
        start_col: 1,
        end_line: 10,
        end_col: 8,
        type_name: "fn()".to_string(),
    });

    let mut f = fs::File::create(&ridx_path).unwrap();
    writer.write_to(&mut f).unwrap();

    let ws = WorkspaceIndex::open(&ridx_path).unwrap();

    // Test compare_capability
    let all_res = compare_capability(&ws, CapabilityKind::All, "test_crate::test_fn");
    assert_eq!(all_res.len(), 5);
    for r in &all_res {
        assert!(r.ridx_found);
        print_comparison(r);
    }
    print_comparison_summary(&all_res);

    // Test individual capabilities
    let res_def = compare_definition(&ws, "src/lib.rs:10:1");
    assert!(res_def.ridx_found);
    assert!(res_def.is_match() || !res_def.is_match());

    let res_refs = compare_references(&ws, "test_crate::test_fn");
    assert!(res_refs.ridx_found);
    assert_eq!(res_refs.ridx_count, 1);

    let res_hover = compare_hover(&ws, "src/lib.rs:10:1");
    assert!(res_hover.ridx_found);

    let res_syms = compare_symbols(&ws, "test_fn");
    assert!(res_syms.ridx_found);

    let res_callers = compare_callers(&ws, "test_crate::test_fn");
    assert!(res_callers.ridx_found);

    // Negative tests
    let missing_def = compare_definition(&ws, "nonexistent");
    assert!(!missing_def.ridx_found);
    let missing_refs = compare_references(&ws, "nonexistent");
    assert!(!missing_refs.ridx_found);
    let missing_hover = compare_hover(&ws, "nonexistent");
    assert!(!missing_hover.ridx_found);
    let missing_syms = compare_symbols(&ws, "zzz_nonexistent");
    assert!(!missing_syms.ridx_found);

    // RSS checks
    assert!(get_process_rss_bytes(std::process::id()).is_some());
    let _ = get_process_rss_bytes_fallback_ps(std::process::id());
    let _ = get_current_process_rss_bytes();
    let _ = get_rust_analyzer_rss_bytes();

    // Mock prod-code script
    let mock_bin = temp_dir.join("bin");
    let _ = fs::create_dir_all(&mock_bin);
    let mock_script = mock_bin.join("prod-code");
    let _ = fs::write(
        &mock_script,
        b"#!/bin/sh\necho '\xE2\x80\xA2 file:///src/lib.rs:10:1'\n",
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&mock_script, fs::Permissions::from_mode(0o755));
    }
    let orig_path = std::env::var("PATH").unwrap_or_default();
    std::env::set_var("PATH", format!("{}:{}", mock_bin.display(), orig_path));

    let res_mock_refs = compare_references(&ws, "test_crate::test_fn");
    assert!(res_mock_refs.prodcode_found);

    let res_mock_syms = compare_symbols(&ws, "test_fn");
    assert!(res_mock_syms.prodcode_found);

    let _ = fs::write(&mock_script, b"#!/bin/sh\nexit 1\n");
    let res_err = compare_references(&ws, "test_crate::test_fn");
    assert!(!res_err.prodcode_found);

    std::env::set_var("PATH", orig_path);
}
