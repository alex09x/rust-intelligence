/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! A definition at the cursor wins over a reference whose span encloses it. Inside a `macro_rules!`
//! invocation every reference of the expansion carries the invocation's span, so on the name of an
//! item written in the invocation the innermost reference is one of them; the item is the answer.

use super::*;
use crate::format::DefKind;
use crate::writer::{IndexWriter, RawDefinition, RawReference};
use std::path::{Path, PathBuf};

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
    target: (Option<u32>, &str, DefKind),
    is_macro_expansion: bool,
) -> RawReference {
    RawReference {
        file: "src/lib.rs".to_string(),
        start_line: span.0,
        start_col: span.1,
        end_line: span.2,
        end_col: span.3,
        target_def_id: target.0,
        target_path: target.1.to_string(),
        target_unit: None,
        target_file: target.0.map(|_| "src/lib.rs".to_string()),
        target_line: 0,
        target_col: 0,
        kind: target.2,
        is_write: false,
        is_macro_expansion,
    }
}

/// Writes `fx.ridx` into `dir`: a shard shaped like the one `ridx index` writes for
///
/// ```text
///  1  rt_test! {                  // macro ref 1:1-30:2 to alloc::sync::Arc, external: one of the
///                                 // expansion's references, all with the invocation's span
///  5      fn socket() {}          // Fn socket 5:12-5:17
///  7      Arc::new(1);            // plain ref 7:5-7:8 to Arc, as wide as the name
///  8      m!(const WIDE: u8 = 1;) // macro ref 8:5-8:30 to Arc; Const WIDE 8:12-8:16 inside it
/// 10      struct Boom;            // Struct Boom 10:12-10:16 and a plain self-reference 10:12-10:16
/// 30  }
/// ```
///
/// Definition spans are name spans, as the driver writes them.
pub(crate) fn write_fixture(dir: &Path) -> PathBuf {
    let path = dir.join("fx.ridx");
    let mut w = IndexWriter::new("fx", "0.1.0");
    w.add_definition(def(1, DefKind::Fn, "socket", (5, 12, 5, 17)));
    w.add_definition(def(2, DefKind::Const, "WIDE", (8, 12, 8, 16)));
    w.add_definition(def(3, DefKind::Struct, "Boom", (10, 12, 10, 16)));
    let arc = "alloc::sync::Arc";
    w.add_reference(reference((1, 1, 30, 2), (None, arc, DefKind::Struct), true));
    w.add_reference(reference((7, 5, 7, 8), (None, arc, DefKind::Struct), false));
    w.add_reference(reference((8, 5, 8, 30), (None, arc, DefKind::Struct), true));
    w.add_reference(reference(
        (10, 12, 10, 16),
        (Some(3), "fx::Boom", DefKind::Struct),
        false,
    ));
    let mut f = std::fs::File::create(&path).unwrap();
    w.write_to(&mut f).unwrap();
    path
}

static FIXTURE_COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn fixture() -> (PathBuf, IndexReader<Mmap>) {
    // One directory per call: the tests run in parallel and each removes its own directory.
    let n = FIXTURE_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_cursor_prec_{}_{n}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = write_fixture(&dir);
    let reader = IndexReader::open(&path).unwrap();
    (dir, reader)
}

#[test]
fn item_inside_a_macro_invocation_answers_definition_on_its_name() {
    let (dir, reader) = fixture();
    // The innermost reference at 5:14 is the invocation-span reference to Arc; the function's
    // name span lies inside it, so the function answers.
    let at_name = reader.find_definition_at("src/lib.rs", 5, 14).unwrap();
    assert_eq!((at_name.path, at_name.kind), ("fx::socket", DefKind::Fn));
    assert_eq!((at_name.start_line, at_name.start_col), (5, 12));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn inside_the_invocation_on_no_item_the_reference_still_answers() {
    let (dir, reader) = fixture();
    let inside = reader.find_definition_at("src/lib.rs", 20, 1).unwrap();
    assert_eq!(
        (inside.path, inside.kind),
        ("alloc::sync::Arc", DefKind::Struct)
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn item_inside_a_single_line_reference_wider_than_its_name_answers() {
    let (dir, reader) = fixture();
    let at_name = reader.find_definition_at("src/lib.rs", 8, 14).unwrap();
    assert_eq!((at_name.path, at_name.kind), ("fx::WIDE", DefKind::Const));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_reference_as_wide_as_its_name_answers_the_reference() {
    let (dir, reader) = fixture();
    let on_arc = reader.find_definition_at("src/lib.rs", 7, 6).unwrap();
    assert_eq!(
        (on_arc.path, on_arc.kind),
        ("alloc::sync::Arc", DefKind::Struct)
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_self_reference_with_the_definition_span_keeps_the_reference() {
    let (dir, reader) = fixture();
    // Equal spans: the reference answers, and it resolves to the struct itself.
    let on_boom = reader.find_definition_at("src/lib.rs", 10, 13).unwrap();
    assert_eq!((on_boom.path, on_boom.kind), ("fx::Boom", DefKind::Struct));
    let self_ref = reader.find_innermost_ref_at("src/lib.rs", 10, 13).unwrap();
    assert_eq!(self_ref.target_def_id(), 3);
    let _ = std::fs::remove_dir_all(dir);
}
