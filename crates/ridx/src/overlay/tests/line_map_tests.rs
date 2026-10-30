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

#[test]
fn test_line_map_matches_the_per_line_algorithm_in_both_directions() {
    for (baseline, live) in CASES {
        let map = LineMap::new(baseline, live);
        let span = baseline.lines().count().max(live.lines().count()) as u32 + 2;
        for line in 0..=span {
            assert_eq!(map.to_live(line), reference_to_live(baseline, live, line), "to_live({line}) for {baseline:?} -> {live:?}");
            assert_eq!(map.to_baseline(line), reference_to_live(live, baseline, line), "to_baseline({line}) for {baseline:?} -> {live:?}");
            assert_eq!(map_text_line_to_live(baseline, live, line), map.to_live(line), "map_text_line_to_live({line}) for {baseline:?} -> {live:?}");
        }
    }
}

/// The map is prefix and suffix only: lines inserted at the top shift everything below them,
/// an edited line loses its counterpart, and a line between two edits is lost as well.
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
    assert_eq!(two_edits.to_live(1), None);
    assert_eq!(two_edits.to_live(2), None);
    assert_eq!(two_edits.to_live(3), Some(5));
    assert_eq!(two_edits.to_live(4), Some(6));
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
