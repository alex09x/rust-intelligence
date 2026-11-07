/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Roundtrip and `O(log N)` query tests for `CfgEntry`, `enabled_features`, and `target_triple`.

use ridx_format::{CfgEntry, Header, HeaderV2, IndexReader, IndexWriter, MAGIC, NO_CFG_VALUE};
use std::io::Cursor;

fn aligned_bytes(raw: Vec<u8>) -> Vec<u8> {
    let mut words = vec![0u64; raw.len().div_ceil(8)];
    let dst = unsafe { std::slice::from_raw_parts_mut(words.as_mut_ptr() as *mut u8, words.len() * 8) };
    dst[..raw.len()].copy_from_slice(&raw);
    let mut out = unsafe {
        let ptr = words.as_mut_ptr() as *mut u8;
        let cap = words.capacity() * 8;
        let len = raw.len();
        std::mem::forget(words);
        Vec::from_raw_parts(ptr, len, cap)
    };
    out.truncate(raw.len());
    out
}

#[test]
fn cfg_table_sorts_deduplicates_and_queries_flags_and_features() {
    assert_eq!(std::mem::size_of::<CfgEntry>(), 8);
    assert_eq!(std::mem::size_of::<Header>(), 400);

    let mut w = IndexWriter::new("cfg_demo", "0.1.0");
    w.set_target_triple("x86_64-unknown-linux-gnu");
    w.add_cfg_key_value("feature", "serde");
    w.add_cfg_key_value("feature", "alloc");
    w.add_cfg_key_value("feature", "serde"); // duplicate
    w.add_cfg_key_value("feature", ""); // empty value != bare flag
    w.add_cfg_flag("unix");
    w.add_cfg_flag("unix"); // duplicate
    w.add_cfg_flag("debug_assertions");
    w.add_cfg_key_value("target_os", "linux");
    w.add_cfg_key_value("target_pointer_width", "64");
    w.add_cfg_key_value("target_has_atomic", "64");
    w.add_cfg_key_value("target_has_atomic", "8");
    assert_eq!(w.cfg_count(), 11);

    let mut raw = Vec::new();
    w.write_to(&mut Cursor::new(&mut raw)).unwrap();
    let buf = aligned_bytes(raw);
    let reader = IndexReader::from_buffer(buf).unwrap();

    assert_eq!(reader.target_triple(), "x86_64-unknown-linux-gnu");
    assert_eq!(reader.cfgs().len(), 9);

    assert!(reader.has_cfg_flag("unix"));
    assert!(reader.has_cfg_flag("debug_assertions"));
    assert!(!reader.has_cfg_flag("windows"));
    assert!(!reader.has_cfg_flag("feature")); // bare `#[cfg(feature)]` was NOT added, even though `feature = ""` was!

    assert!(reader.has_cfg_key_value("feature", ""));
    assert!(reader.has_cfg_key_value("feature", "alloc"));
    assert!(reader.has_cfg_key_value("feature", "serde"));
    assert!(!reader.has_cfg_key_value("feature", "std"));
    assert!(reader.has_cfg_key_value("target_os", "linux"));
    assert_eq!(reader.cfgs()[0].from_le().value_offset, NO_CFG_VALUE);

    let feats: Vec<&str> = reader.enabled_features().collect();
    assert_eq!(feats, vec!["", "alloc", "serde"]);

    let atomics: Vec<&str> = reader.cfg_values("target_has_atomic").collect();
    assert_eq!(atomics, vec!["64", "8"]);
}

#[test]
fn v2_legacy_header_has_empty_cfgs_and_target_triple() {
    let pool = b"\0legacy\x000.1.0\0";
    let hdr_size = std::mem::size_of::<HeaderV2>();
    let pool_off = (hdr_size.div_ceil(8) * 8) as u64;
    let hdr = HeaderV2 {
        magic: MAGIC,
        version: 2,
        crate_name_offset: 1,
        crate_version_offset: 8,
        string_pool_offset: pool_off,
        string_pool_len: pool.len() as u64,
        files_offset: pool_off,
        files_count: 0,
        unit_key_offset: 0,
        defs_offset: pool_off,
        defs_count: 0,
        generation_offset: 0,
        defs_by_hash_offset: pool_off,
        defs_by_hash_count: 0,
        _pad2: 0,
        refs_offset: pool_off,
        refs_count: 0,
        _pad3: 0,
        refs_by_target_offset: pool_off,
        refs_by_target_count: 0,
        _pad4: 0,
        types_offset: pool_off,
        types_count: 0,
        _pad5: 0,
    };
    let mut raw = vec![0u8; pool_off as usize + pool.len()];
    let hdr_bytes = unsafe { std::slice::from_raw_parts(&hdr as *const HeaderV2 as *const u8, hdr_size) };
    raw[..hdr_size].copy_from_slice(hdr_bytes);
    raw[pool_off as usize..].copy_from_slice(pool);
    let reader = IndexReader::from_buffer(aligned_bytes(raw)).unwrap();
    assert_eq!(reader.target_triple(), "");
    assert!(reader.cfgs().is_empty());
    assert_eq!(reader.enabled_features().len(), 0);
}
