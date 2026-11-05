/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Notes round-trip through the writer and reader, and version 3 indexes still open.

use crate::format::{NOTE_CALL_ARGS, NOTE_LAYOUT};
use crate::writer::{IndexWriter, RawNote};
use crate::IndexReader;
use std::io::Cursor;

fn note(line: u32, col: u32, kind: u32, text: &str) -> RawNote {
    RawNote { file: "src/lib.rs".into(), start_line: line, start_col: col, end_line: line, end_col: col + 3, kind, text: text.into() }
}

fn written(w: &mut IndexWriter) -> Vec<u8> {
    let mut buf = Cursor::new(Vec::new());
    w.write_to(&mut buf).unwrap();
    buf.into_inner()
}

/// An 8-byte aligned copy of `bytes`, as the reader requires.
fn aligned(bytes: &[u8]) -> Vec<u64> {
    let mut words = vec![0u64; bytes.len().div_ceil(8)];
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), words.as_mut_ptr() as *mut u8, bytes.len()) };
    words
}

struct Words(Vec<u64>, usize);
impl AsRef<[u8]> for Words {
    fn as_ref(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.0.as_ptr() as *const u8, self.1) }
    }
}

#[test]
fn test_notes_round_trip_by_span_and_kind() {
    let mut w = IndexWriter::new("demo", "0.1.0");
    w.add_note(note(9, 5, NOTE_LAYOUT, "size = 8, align = 0x8, no Drop"));
    w.add_note(note(3, 12, NOTE_CALL_ARGS, "`T` = `String`"));
    w.add_note(note(3, 12, NOTE_LAYOUT, "size = 24 (0x18), align = 0x8, needs Drop"));
    let bytes = written(&mut w);
    let len = bytes.len();
    let reader = IndexReader::from_buffer(Words(aligned(&bytes), len)).unwrap();
    let file_id = reader.find_file_id("src/lib.rs").unwrap();
    assert_eq!(reader.notes().len(), 3);
    assert_eq!(reader.note_at(file_id, 3, 12, NOTE_CALL_ARGS), Some("`T` = `String`"));
    assert_eq!(reader.note_at(file_id, 3, 12, NOTE_LAYOUT), Some("size = 24 (0x18), align = 0x8, needs Drop"));
    assert_eq!(reader.note_at(file_id, 9, 5, NOTE_LAYOUT), Some("size = 8, align = 0x8, no Drop"));
    assert_eq!(reader.note_at(file_id, 9, 6, NOTE_LAYOUT), None);
}

#[test]
fn test_version_3_index_opens_without_notes() {
    let mut w = IndexWriter::new("demo", "0.1.0");
    w.add_note(note(1, 1, NOTE_LAYOUT, "dropped"));
    let mut bytes = written(&mut w);
    // A version 3 writer emitted the same sections without the notes fields: mark the file as version 3.
    bytes[4..8].copy_from_slice(&3u32.to_le_bytes());
    let len = bytes.len();
    let reader = IndexReader::from_buffer(Words(aligned(&bytes), len)).unwrap();
    assert_eq!(reader.header().version, 3);
    assert!(reader.notes().is_empty(), "a version 3 header has no notes table");
    assert_eq!(reader.crate_name(), "demo");
}
