/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use crate::overlay::DocumentOverlay;

#[test]
fn test_position_conversions_and_bounds() {
    let text = "let rocket = \"🚀\";\nlet grüßen = 42;";
    let ov = DocumentOverlay::new("file:///pos.rs", "/pos.rs", 1, text.to_string());

    // 1. Line out of bounds
    assert_eq!(ov.utf16_col_to_char_col(999, 5), 6);
    assert_eq!(ov.char_col_to_utf16_col(999, 6), 5);
    assert_eq!(ov.line_char_col_to_byte(999, 1), text.len());
    assert_eq!(ov.position_to_byte_offset(999, 0), text.len());

    // 2. byte_to_char_line_col
    let (l1, c1) = ov.byte_to_char_line_col(0);
    assert_eq!((l1, c1), (1, 1));
    let (l2, c2) = ov.byte_to_char_line_col(text.len());
    assert_eq!(l2, 2);
    assert!(c2 > 1);

    // 3. Last line without trailing newline
    assert_eq!(ov.utf16_col_to_char_col(2, 4), 5);
    assert_eq!(ov.char_col_to_utf16_col(2, 5), 4);
    assert!(ov.line_char_col_to_byte(2, 5) > 0);
    assert!(ov.position_to_byte_offset(1, 4) > 0);

    // 4. Content changes with missing text (line 163 in overlay/mod.rs)
    let mut ov_changes = DocumentOverlay::new("file:///ch.rs", "/ch.rs", 1, "fn test() {}\n".to_string());
    let invalid_change = serde_json::json!({
        "range": {
            "start": { "line": 0, "character": 0 },
            "end": { "line": 0, "character": 2 }
        }
    });
    ov_changes.apply_content_changes(2, &[invalid_change]);
    assert_eq!(ov_changes.text, "fn test() {}\n");

    // 5. OverlayManager reconcile_with_disk (lines 158-162 in overlay/manager.rs)
    let mut mgr = crate::overlay::OverlayManager::new();
    let tmp_file = std::env::temp_dir().join(format!("ridx_recon_{}.rs", std::process::id()));
    std::fs::write(&tmp_file, "fn on_disk() {}\n").unwrap();
    let tmp_path = tmp_file.to_str().unwrap();
    let tmp_uri = format!("file://{}", tmp_path);
    mgr.open_document(&tmp_uri, tmp_path, 1, "fn on_disk() {}\n".to_string());
    let hash = mgr.get(tmp_path).unwrap().content_hash;
    mgr.reconcile_with_disk(tmp_path, hash);
    mgr.reconcile_with_disk(tmp_path, hash + 1);
    let _ = std::fs::remove_file(&tmp_file);
}
