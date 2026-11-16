/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! `LineMap` answers exactly what the per-line mapping functions answered before it existed.

use crate::overlay::{map_text_line_to_live, DocumentOverlay, LineMap};

/// The per-line algorithm as it stood before `LineMap`: common prefix and suffix of lines,
/// recomputed from both texts on every call.
fn reference_to_live(baseline_text: &str, live_text: &str, baseline_line: u32) -> Option<u32> {
    let b_idx = baseline_line.saturating_sub(1) as usize;
    let baseline_lines: Vec<&str> = baseline_text.lines().collect();
    if b_idx >= baseline_lines.len() {
        return None;
    }
    if baseline_text == live_text {
        return Some(baseline_line);
    }
    let curr_lines: Vec<&str> = live_text.lines().collect();
    let mut prefix_common = 0;
    while prefix_common < curr_lines.len() && prefix_common < baseline_lines.len() && curr_lines[prefix_common] == baseline_lines[prefix_common] {
        prefix_common += 1;
    }
    if b_idx < prefix_common {
        return Some(baseline_line);
    }
    let mut suffix_common = 0;
    while suffix_common < curr_lines.len().saturating_sub(prefix_common)
        && suffix_common < baseline_lines.len().saturating_sub(prefix_common)
        && curr_lines[curr_lines.len() - 1 - suffix_common] == baseline_lines[baseline_lines.len() - 1 - suffix_common]
    {
        suffix_common += 1;
    }
    if b_idx >= baseline_lines.len().saturating_sub(suffix_common) {
        let offset_from_end = baseline_lines.len() - 1 - b_idx;
        let live_line_idx = curr_lines.len() - 1 - offset_from_end;
        return Some((live_line_idx + 1) as u32);
    }
    None
}

const CASES: &[(&str, &str)] = &[
    ("a\nb\nc\n", "a\nb\nc\n"),
    ("a\nb\nc", "a\nb\nc\n"),
    ("a\nb\nc\n", "x\na\nb\nc\n"),
    ("a\nb\nc\n", "a\nb\nc\nx\ny\n"),
    ("a\nb\nc\nd\n", "a\nB\nc\nd\n"),
    ("a\nb\nc\n", "A\nb\nc\n"),
    ("a\nb\nc\n", "a\nb\nC\n"),
    ("a\nb\nc\nd\n", "a\nd\n"),
    ("a\nb\nc\nd\n", "a\n"),
    ("a\nb\nc\nd\n", "d\n"),
    ("a\nb\nc\n", "a\nb\nb\nc\n"),
    ("a\nb\n", "x\ny\n"),
    ("", "a\nb\n"),
    ("a\nb\n", ""),
    ("", ""),
    ("a\r\nb\r\nc\r\n", "a\r\nb\r\nc\r\n"),
    ("a\r\nb\r\nc\r\n", "a\nb\nc\n"),
    ("a\r\nb\r\nc\r\n", "// top\r\na\r\nb\r\nc\r\n"),
    ("fn a() {}\nfn b() {}\nfn c() {}\n", "fn a() {}\nfn c() {}\n"),
    ("fn a() {}\nfn b() {}\n", "fn a() {}\n\nfn b() {}\n"),
];

/// Wherever the prefix-and-suffix algorithm answered, the map answers the same; between edits it may answer more, with
/// a line of the same text, both directions agreeing.
#[test]
fn test_line_map_extends_the_per_line_algorithm_in_both_directions() {
    for (baseline, live) in CASES {
        let map = LineMap::new(baseline, live);
        let span = baseline.lines().count().max(live.lines().count()) as u32 + 2;
        let (b_lines, l_lines): (Vec<&str>, Vec<&str>) = (baseline.lines().collect(), live.lines().collect());
        for line in 0..=span {
            if let Some(expected) = reference_to_live(baseline, live, line) {
                assert_eq!(map.to_live(line), Some(expected), "to_live({line}) for {baseline:?} -> {live:?}");
            }
            if let Some(expected) = reference_to_live(live, baseline, line) {
                assert_eq!(map.to_baseline(line), Some(expected), "to_baseline({line}) for {baseline:?} -> {live:?}");
            }
            if let Some(l) = map.to_live(line).filter(|_| line > 0) {
                assert_eq!(b_lines[line as usize - 1], l_lines[l as usize - 1], "the same text");
                assert_eq!(map.to_baseline(l), Some(line), "both directions agree");
            }
            assert_eq!(map_text_line_to_live(baseline, live, line), map.to_live(line), "map_text_line_to_live({line}) for {baseline:?} -> {live:?}");
        }
    }
}

/// Lines inserted at the top shift everything below them, an edited line loses its counterpart, and a line between
/// two edits keeps it.
#[test]
fn test_line_map_shift_edit_and_removal() {
    let shifted = LineMap::new("a\nb\nc\n", "x\ny\na\nb\nc\n");
    assert_eq!(shifted.to_live(1), Some(3));
    assert_eq!(shifted.to_live(2), Some(4));
    assert_eq!(shifted.to_live(3), Some(5));
    assert_eq!(shifted.to_live(4), None);
    assert_eq!(shifted.to_baseline(1), None);
    assert_eq!(shifted.to_baseline(2), None);
    assert_eq!(shifted.to_baseline(3), Some(1));
    assert_eq!(shifted.to_baseline(5), Some(3));

    let edited = LineMap::new("a\nb\nc\nd\n", "a\nB\nc\nd\n");
    assert_eq!(edited.to_live(1), Some(1));
    assert_eq!(edited.to_live(2), None);
    assert_eq!(edited.to_live(3), Some(3));
    assert_eq!(edited.to_baseline(2), None);

    let removed = LineMap::new("a\nb\nc\n", "a\nc\n");
    assert_eq!(removed.to_live(2), None);
    assert_eq!(removed.to_live(3), Some(2));
    assert_eq!(removed.to_baseline(2), Some(3));

    let two_edits = LineMap::new("a\nb\nc\nd\n", "x\ny\na\nB\nc\nd\n");
    assert_eq!(two_edits.to_live(1), Some(3), "`a`, between the lines added above and the line edited below");
    assert_eq!(two_edits.to_live(2), None, "the edited line");
    assert_eq!(two_edits.to_live(3), Some(5));
    assert_eq!(two_edits.to_live(4), Some(6));
    assert_eq!(two_edits.to_baseline(3), Some(1));
}

#[test]
fn test_overlay_line_map_follows_the_compiled_baseline() {
    let mut ov = DocumentOverlay::new_with_provenance("file:///test.rs", "/test.rs", 1, "top\nmid\nbot\n".to_string(), false);
    assert_eq!(ov.line_map(), None);
    ov.advance_compiled_baseline("top\nold_mid\nbot\n".to_string());
    let map = ov.line_map().expect("baseline set");
    for line in 0..5 {
        assert_eq!(map.to_live(line), ov.map_compiled_line_to_live(line));
        assert_eq!(map.to_baseline(line), ov.map_line_to_compiled(line));
    }
    assert_eq!(map.to_live(2), None);
    assert_eq!(map.to_live(3), Some(3));
}

#[test]
fn test_a_stale_workspace_file_synced_with_disk_has_no_line_map() {
    let dir = std::env::temp_dir().join(format!("ridx_line_map_stale_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("lib.rs");
    let text = "fn changed_on_disk() {}\n";
    std::fs::write(&file, text).unwrap();
    let path = file.to_str().unwrap();
    // A workspace file the index hashed differently: changed on disk, no snapshot. Its spans are stale.
    let stale = DocumentOverlay::new_with_source_hash("file:///x/lib.rs", path, 1, text.to_string(), Some(0x1234));
    assert!(stale.line_map().is_none(), "a stale workspace file must abstain, not map 1:1 onto old spans");
    // A std or dependency source the index recorded no hash for maps line for line.
    let unhashed = DocumentOverlay::new_with_source_hash("file:///x/lib.rs", path, 1, text.to_string(), None);
    assert_eq!(unhashed.line_map().and_then(|m| m.to_baseline(1)), Some(1));
    let _ = std::fs::remove_dir_all(&dir);
}
