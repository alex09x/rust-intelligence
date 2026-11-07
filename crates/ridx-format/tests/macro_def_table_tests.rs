/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Roundtrip, binary-search, and legacy header compatibility tests for `MacroDefEntry`.

use ridx_format::{
    DefKind, HeaderV2, IndexReader, IndexWriter, MacroDelim, MacroEdition, RawDefinition, RawMacroDef, MACRO_FLAG_BUILTIN, MACRO_FLAG_EXPORTED, MACRO_FLAG_LOCAL_INNER,
    MACRO_FLAG_MACRO_RULES, MAGIC,
};
use std::io::Cursor;

fn sample_macro_def(id: u32, name: &str) -> RawDefinition {
    RawDefinition {
        def_id: id,
        kind: DefKind::Macro,
        visibility: 0,
        is_macro_expansion: false,
        name: name.to_string(),
        path: format!("mac_crate::{name}"),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: id,
        start_col: 1,
        end_line: id + 1,
        end_col: 2,
    }
}

#[test]
fn macro_def_table_sorts_deduplicates_and_queries_by_def_index() {
    let mut w = IndexWriter::new("mac_crate", "0.1.0");
    let d0 = w.add_definition(sample_macro_def(1, "first_mac"));
    let d1 = w.add_definition(sample_macro_def(2, "fn_unrelated"));
    let d2 = w.add_definition(sample_macro_def(3, "exported_mac"));
    let d3 = w.add_definition(sample_macro_def(4, "decl_v2_mac"));

    // Insert out of order + duplicate d2 to verify deterministic sort & dedup
    w.add_macro_def(RawMacroDef { def_index: d3, body: "($v:expr) { $v * 2 }", flags: 0, delim: MacroDelim::Brace, edition: MacroEdition::Edition2024 });
    w.add_macro_def(RawMacroDef {
        def_index: d2,
        body: "($x:expr) => ($x + 1);",
        flags: MACRO_FLAG_MACRO_RULES | MACRO_FLAG_EXPORTED | MACRO_FLAG_LOCAL_INNER,
        delim: MacroDelim::Paren,
        edition: MacroEdition::Edition2021,
    });
    w.add_macro_def(RawMacroDef { def_index: d2, body: "duplicate_ignored", flags: MACRO_FLAG_MACRO_RULES, delim: MacroDelim::Brace, edition: MacroEdition::Edition2015 });
    w.add_macro_def(RawMacroDef {
        def_index: d0,
        body: "[ $($t:tt)* ] => [ stringify!($($t)*) ];",
        flags: MACRO_FLAG_MACRO_RULES | MACRO_FLAG_BUILTIN,
        delim: MacroDelim::Bracket,
        edition: MacroEdition::Edition2018,
    });

    let mut bytes = Vec::new();
    w.write_to(&mut Cursor::new(&mut bytes)).unwrap();

    let reader = IndexReader::from_buffer(bytes).unwrap();
    assert_eq!(reader.macro_defs().len(), 3);
    assert!(reader.macro_def_for_def(d1).is_none());

    let m0 = reader.macro_def_for_def(d0).unwrap();
    assert!(m0.is_macro_rules() && m0.is_builtin() && !m0.is_exported());
    assert_eq!(m0.delim, MacroDelim::Bracket);
    assert_eq!(m0.edition, MacroEdition::Edition2018);
    assert_eq!(m0.body, "[ $($t:tt)* ] => [ stringify!($($t)*) ];");

    let m2 = reader.macro_def_for_def(d2).unwrap();
    assert!(m2.is_macro_rules() && m2.is_exported() && m2.has_local_inner_macros());
    assert_eq!(m2.delim, MacroDelim::Paren);
    assert_eq!(m2.edition, MacroEdition::Edition2021);
    assert_eq!(m2.body, "($x:expr) => ($x + 1);");

    let m3 = reader.macro_def_for_def(d3).unwrap();
    assert!(!m3.is_macro_rules() && !m3.is_exported() && !m3.is_builtin());
    assert_eq!(m3.delim, MacroDelim::Brace);
    assert_eq!(m3.edition, MacroEdition::Edition2024);
    assert_eq!(m3.body, "($v:expr) { $v * 2 }");

    let indices: Vec<u32> = reader.macro_defs_iter().map(|v| v.def_index).collect();
    assert_eq!(indices, vec![d0, d2, d3]);
}

#[test]
fn v2_legacy_header_has_empty_macro_defs() {
    let mut buf = vec![0u8; 160];
    let pool = b"\0legacy\x000.1.0\0";
    buf[128..128 + pool.len()].copy_from_slice(pool);
    let hdr = HeaderV2 {
        magic: MAGIC,
        version: 2,
        crate_name_offset: 1,
        crate_version_offset: 8,
        string_pool_offset: 128,
        string_pool_len: pool.len() as u64,
        files_offset: 144,
        files_count: 0,
        unit_key_offset: 0,
        defs_offset: 144,
        defs_count: 0,
        generation_offset: 0,
        defs_by_hash_offset: 144,
        defs_by_hash_count: 0,
        _pad2: 0,
        refs_offset: 144,
        refs_count: 0,
        _pad3: 0,
        refs_by_target_offset: 144,
        refs_by_target_count: 0,
        _pad4: 0,
        types_offset: 144,
        types_count: 0,
        _pad5: 0,
    }
    .to_le();
    let hdr_bytes = unsafe { std::slice::from_raw_parts((&hdr as *const HeaderV2) as *const u8, 128) };
    buf[..128].copy_from_slice(hdr_bytes);

    let reader = IndexReader::from_buffer(buf).unwrap();
    assert!(reader.macro_defs().is_empty());
    assert!(reader.macro_def_for_def(0).is_none());
}
