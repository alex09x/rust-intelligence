/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! A reference whose span contains its own target's definition is not a use of the name: the
//! driver writes one for the call the `#[test]` harness generates (plain, the whole function) and
//! one for the harness's use of the generated test constant (macro, the whole file or module).
//! The reader's lookups skip them; every other reference, single- or multi-line, stays.

use super::*;
use crate::format::DefKind;
use crate::writer::{IndexWriter, RawDefinition, RawReference};

fn def(def_id: u32, kind: DefKind, name: &str, span: (u32, u32, u32, u32)) -> RawDefinition {
    RawDefinition {
        def_id,
        kind,
        visibility: 0,
        is_macro_expansion: false,
        name: name.to_string(),
        path: format!("fx::{name}"),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: span.0,
        start_col: span.1,
        end_line: span.2,
        end_col: span.3,
    }
}

fn reference(
    span: (u32, u32, u32, u32),
    target: (Option<u32>, &str, DefKind, u32),
    is_macro_expansion: bool,
) -> RawReference {
    RawReference {
        file: "src/lib.rs".to_string(),
        start_line: span.0,
        start_col: span.1,
        end_line: span.2,
        end_col: span.3,
        target_def_id: target.0,
        target_path: format!("fx::{}", target.1),
        target_unit: None,
        target_file: Some("src/lib.rs".to_string()),
        target_line: target.3,
        target_col: 1,
        kind: target.2,
        is_write: false,
        is_macro_expansion,
    }
}

/// A shard shaped like the one `ridx index` writes for
///
/// ```text
///  1  #[test]               // Const outer 1:1-1:8 (the generated test constant)
///  2  fn outer() {          // Fn outer 2:4-2:9
///  5      struct Inner;     // Struct Inner 5:12-5:17
///  7      helper();         // plain Fn ref 7:5-7:11
///  8      assert!(...);     // single-line macro ref 8:5-8:12 to panic
///  9      helper!()         // single-line macro ref 9:5-9:11 to helper
/// 10  }
/// 12  impl_helper! {        // multi-line macro ref 12:1-14:2 to helper (an invocation whose
/// 14  }                     // expansion uses helper)
/// 20  fn helper() {}        // Fn helper 20:4-20:10 (def 3)
/// 38  impl_both! {          // two plain refs 38:1-45:2 with the path fx::helper: one to def 3
/// 40      fn helper() {}    // (whose span is outside 38:1-45:2) and one to def 5, the second
/// 45  }                     // definition of the same path (Fn helper 40:4-40:10, enclosed)
/// 50  fn zero_end() {}      // Fn zero_end with the span 50:4-0:0 of an index without end spans,
///                           // and a plain ref 50:4-50:12 at the declaration
/// ```
///
/// plus the two harness references, both with the path `fx::outer`: a plain `Fn` reference to the
/// function spanning the whole item (2:1-10:2) and a macro `Const` reference to the generated
/// constant spanning the whole file (1:1-30:2). Definition spans are name spans, as the driver
/// writes them.
static FIXTURE_COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn fixture() -> (std::path::PathBuf, IndexReader<Mmap>) {
    // One directory per call: the tests run in parallel and each removes its own directory.
    let n = FIXTURE_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_name_span_{}_{n}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("fx.ridx");
    let mut w = IndexWriter::new("fx", "0.1.0");
    w.add_definition(def(1, DefKind::Fn, "outer", (2, 4, 2, 9)));
    w.add_definition(def(2, DefKind::Struct, "Inner", (5, 12, 5, 17)));
    w.add_definition(def(3, DefKind::Fn, "helper", (20, 4, 20, 10)));
    w.add_definition(def(4, DefKind::Const, "outer", (1, 1, 1, 8)));
    w.add_definition(def(5, DefKind::Fn, "helper", (40, 4, 40, 10)));
    w.add_definition(def(6, DefKind::Fn, "zero_end", (50, 4, 0, 0)));
    w.add_reference(reference(
        (2, 1, 10, 2),
        (Some(1), "outer", DefKind::Fn, 2),
        false,
    ));
    w.add_reference(reference(
        (1, 1, 30, 2),
        (Some(4), "outer", DefKind::Const, 1),
        true,
    ));
    w.add_reference(reference(
        (7, 5, 7, 11),
        (Some(3), "helper", DefKind::Fn, 20),
        false,
    ));
    w.add_reference(reference(
        (8, 5, 8, 12),
        (None, "panic", DefKind::Fn, 0),
        true,
    ));
    w.add_reference(reference(
        (9, 5, 9, 11),
        (Some(3), "helper", DefKind::Fn, 20),
        true,
    ));
    w.add_reference(reference(
        (12, 1, 14, 2),
        (Some(3), "helper", DefKind::Fn, 20),
        true,
    ));
    w.add_reference(reference(
        (50, 4, 50, 12),
        (Some(6), "zero_end", DefKind::Fn, 50),
        false,
    ));
    w.add_reference(reference(
        (38, 1, 45, 2),
        (Some(3), "helper", DefKind::Fn, 20),
        false,
    ));
    w.add_reference(reference(
        (38, 1, 45, 2),
        (Some(5), "helper", DefKind::Fn, 40),
        false,
    ));
    let mut f = std::fs::File::create(&path).unwrap();
    w.write_to(&mut f).unwrap();
    drop(f);
    let reader = IndexReader::open(&path).unwrap();
    (dir, reader)
}

fn spans(refs: &[&RefEntry]) -> Vec<(u32, u32, u32, u32)> {
    let mut out: Vec<_> = refs
        .iter()
        .map(|r| {
            let s = r.span();
            (s.start_line(), s.start_col(), s.end_line(), s.end_col())
        })
        .collect();
    out.sort_unstable();
    out
}

#[test]
fn whole_item_reference_does_not_answer_definition_on_a_nested_item() {
    let (dir, reader) = fixture();
    // Definition on the nested struct's name: the struct, not the enclosing test function.
    let at_inner = reader.find_definition_at("src/lib.rs", 5, 14).unwrap();
    assert_eq!(
        (at_inner.path, at_inner.kind),
        ("fx::Inner", DefKind::Struct)
    );
    assert_eq!((at_inner.start_line, at_inner.start_col), (5, 12));
    assert!(reader.find_innermost_ref_at_file_id(0, 5, 14).is_none());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn whole_item_reference_is_not_among_the_references() {
    let (dir, reader) = fixture();
    // References to the test function: neither the whole-item span 2:1-10:2 nor the harness's
    // whole-file span 1:1-30:2 is one of them.
    assert_eq!(
        spans(&reader.find_references("fx::outer")),
        Vec::<(u32, u32, u32, u32)>::new()
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn single_line_reference_in_the_body_still_resolves() {
    let (dir, reader) = fixture();
    let at_call = reader.find_definition_at("src/lib.rs", 7, 6).unwrap();
    assert_eq!((at_call.path, at_call.kind), ("fx::helper", DefKind::Fn));
    assert_eq!((at_call.start_line, at_call.start_col), (20, 4));
    assert!(reader.find_references("fx::helper").iter().any(|r| {
        let s = r.span();
        (s.start_line(), s.start_col(), s.end_line(), s.end_col()) == (7, 5, 7, 11)
    }));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn single_line_macro_reference_stays_in_references() {
    let (dir, reader) = fixture();
    assert!(reader.find_references("fx::helper").iter().any(|r| {
        let s = r.span();
        (s.start_line(), s.start_col(), s.end_line(), s.end_col()) == (9, 5, 9, 11)
    }));
    assert_eq!(
        spans(&reader.find_references("fx::panic")),
        vec![(8, 5, 8, 12)]
    );
    let at_macro = reader.find_definition_at("src/lib.rs", 8, 7).unwrap();
    assert_eq!(
        (at_macro.path, at_macro.is_macro_expansion),
        ("fx::panic", true)
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn whole_file_macro_reference_does_not_answer_definition_inside_it() {
    let (dir, reader) = fixture();
    // Line 25 is inside the harness's whole-file reference and outside every definition.
    assert!(reader.find_innermost_ref_at_file_id(0, 25, 1).is_none());
    assert!(reader.find_definition_at("src/lib.rs", 25, 1).is_none());
    // On the function's own name the definition answers.
    let on_name = reader.find_definition_at("src/lib.rs", 20, 6).unwrap();
    assert_eq!((on_name.path, on_name.kind), ("fx::helper", DefKind::Fn));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn multi_line_macro_reference_to_another_item_stays() {
    let (dir, reader) = fixture();
    // The invocation span 12:1-14:2 does not contain helper's definition: it is a use of helper.
    assert_eq!(
        spans(&reader.find_references("fx::helper")),
        vec![(7, 5, 7, 11), (9, 5, 9, 11), (12, 1, 14, 2), (38, 1, 45, 2)]
    );
    let inside = reader.find_definition_at("src/lib.rs", 13, 1).unwrap();
    assert_eq!((inside.path, inside.kind), ("fx::helper", DefKind::Fn));
    assert_eq!((inside.start_line, inside.start_col), (20, 4));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn only_the_definition_the_reference_points_to_counts_when_a_path_has_two() {
    let (dir, reader) = fixture();
    // Two references share the span 38:1-45:2 and the path fx::helper. The one pointing at def 5
    // (40:4) begins before its own target and reaches past it: skipped. The one pointing at def 3
    // (20:4) does not: kept, although the span contains the other definition of the same path.
    let kept: Vec<u32> = reader
        .find_references("fx::helper")
        .iter()
        .filter(|r| r.span().start_line() == 38)
        .map(|r| r.target_def_id())
        .collect();
    assert_eq!(kept, vec![3]);
    let inside = reader.find_definition_at("src/lib.rs", 42, 1).unwrap();
    assert_eq!((inside.path, inside.start_line), ("fx::helper", 20));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn reference_at_a_definition_without_an_end_span_is_kept() {
    let (dir, reader) = fixture();
    // The definition of zero_end ends at 0:0, before its start: it is never enclosed, so the
    // reference 50:4-50:12 at its declaration stays and still answers definition.
    assert_eq!(
        spans(&reader.find_references("fx::zero_end")),
        vec![(50, 4, 50, 12)]
    );
    let at_decl = reader.find_definition_at("src/lib.rs", 50, 11).unwrap();
    assert_eq!((at_decl.path, at_decl.start_line), ("fx::zero_end", 50));
    let _ = std::fs::remove_dir_all(dir);
}
