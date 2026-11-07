/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Backward-compatibility and section-bounds validation tests for `v5` typed tables
//! against legacy `v2` (128B), `v3` (160B), and `v4` (176B) headers, plus malformed
//! `v5` headers (Step 1.5).

use ridx_format::{Header, HeaderV2, IndexReader, CURRENT_FORMAT_VERSION, HEADER_V3_SIZE, HEADER_V4_SIZE, MAGIC};

#[repr(align(8))]
struct AlignedBuf([u8; 512]);

fn copy_aligned(bytes: &[u8]) -> Box<AlignedBuf> {
    let mut b = Box::new(AlignedBuf([0u8; 512]));
    b.0[..bytes.len()].copy_from_slice(bytes);
    b
}

#[test]
fn legacy_v2_v3_v4_headers_gracefully_expose_empty_v5_tables() {
    let check_empty = |raw: &[u8]| {
        let aligned = copy_aligned(raw);
        let r = IndexReader::from_buffer(&aligned.0[..raw.len()]).unwrap();
        assert!(r.ty_defs().is_empty());
        assert!(r.ty_entries().is_empty());
        assert!(r.ty_bounds().is_empty());
        assert!(r.ty_args_raw().is_empty());
        assert!(r.fn_sigs().is_empty());
        assert!(r.impls().is_empty());
        assert!(r.assoc_tys().is_empty());
        assert!(r.fields().is_empty());
        assert!(r.def_tys().is_empty());
        assert!(r.generic_params().is_empty());
        assert!(r.where_preds().is_empty());
        assert!(r.lang_items().is_empty());
        assert!(r.uses().is_empty());
        assert!(r.cfgs().is_empty());
        assert!(r.macro_defs().is_empty());
        assert_eq!(r.target_triple(), "");
    };

    // v4 (176-byte header)
    let mut hdr = Header {
        magic: MAGIC,
        version: 4,
        string_pool_offset: HEADER_V4_SIZE as u64,
        string_pool_len: 8,
        files_offset: (HEADER_V4_SIZE + 8) as u64,
        defs_offset: (HEADER_V4_SIZE + 8) as u64,
        defs_by_hash_offset: (HEADER_V4_SIZE + 8) as u64,
        refs_offset: (HEADER_V4_SIZE + 8) as u64,
        refs_by_target_offset: (HEADER_V4_SIZE + 8) as u64,
        types_offset: (HEADER_V4_SIZE + 8) as u64,
        defs_by_owner_offset: (HEADER_V4_SIZE + 8) as u64,
        derefs_offset: (HEADER_V4_SIZE + 8) as u64,
        notes_offset: (HEADER_V4_SIZE + 8) as u64,
        ..Header::default()
    }
    .to_le();
    let mut raw_v4 = Vec::from(unsafe { std::slice::from_raw_parts((&hdr as *const Header) as *const u8, HEADER_V4_SIZE) });
    raw_v4.extend_from_slice(&[0u8; 8]);
    check_empty(&raw_v4);

    // v3 (160-byte header)
    hdr.version = 3u32.to_le();
    hdr.string_pool_offset = (HEADER_V3_SIZE as u64).to_le();
    let off3 = ((HEADER_V3_SIZE + 8) as u64).to_le();
    hdr.files_offset = off3;
    hdr.defs_offset = off3;
    hdr.defs_by_hash_offset = off3;
    hdr.refs_offset = off3;
    hdr.refs_by_target_offset = off3;
    hdr.types_offset = off3;
    hdr.defs_by_owner_offset = off3;
    hdr.derefs_offset = off3;
    let mut raw_v3 = Vec::from(unsafe { std::slice::from_raw_parts((&hdr as *const Header) as *const u8, HEADER_V3_SIZE) });
    raw_v3.extend_from_slice(&[0u8; 8]);
    check_empty(&raw_v3);

    // v2 (128-byte header)
    let mut h2: HeaderV2 = unsafe { std::mem::zeroed() };
    h2.magic = MAGIC;
    h2.version = 2u32.to_le();
    h2.string_pool_offset = 128u64.to_le();
    h2.string_pool_len = 8u64.to_le();
    let off2 = 136u64.to_le();
    h2.files_offset = off2;
    h2.defs_offset = off2;
    h2.defs_by_hash_offset = off2;
    h2.refs_offset = off2;
    h2.refs_by_target_offset = off2;
    h2.types_offset = off2;
    let mut raw_v2 = Vec::from(unsafe { std::slice::from_raw_parts((&h2 as *const HeaderV2) as *const u8, 128) });
    raw_v2.extend_from_slice(&[0u8; 8]);
    check_empty(&raw_v2);
}

#[test]
fn truncated_or_misaligned_v5_sections_are_rejected_safely() {
    // 1. Truncated v5 header (< 416 bytes) rejected without UB
    let mut short_v5 = vec![0u8; 200];
    short_v5[..4].copy_from_slice(&MAGIC);
    short_v5[4..8].copy_from_slice(&CURRENT_FORMAT_VERSION.to_le_bytes());
    let aligned_short = copy_aligned(&short_v5);
    assert!(IndexReader::from_buffer(&aligned_short.0[..short_v5.len()]).is_err());

    // 2. Out-of-bounds macro_defs_count rejected by validate_sections
    let hdr_sz = std::mem::size_of::<Header>();
    let end = (hdr_sz + 8) as u64;
    let bad_hdr = Header {
        magic: MAGIC,
        version: CURRENT_FORMAT_VERSION,
        string_pool_offset: hdr_sz as u64,
        string_pool_len: 8,
        files_offset: end,
        defs_offset: end,
        defs_by_hash_offset: end,
        refs_offset: end,
        refs_by_target_offset: end,
        types_offset: end,
        defs_by_owner_offset: end,
        derefs_offset: end,
        notes_offset: end,
        ty_defs_offset: end,
        ty_entries_offset: end,
        ty_bounds_offset: end,
        ty_args_offset: end,
        fn_sigs_offset: end,
        impls_offset: end,
        assoc_tys_offset: end,
        fields_offset: end,
        def_tys_offset: end,
        generic_params_offset: end,
        where_preds_offset: end,
        lang_items_offset: end,
        uses_offset: end,
        cfgs_offset: end,
        macro_defs_offset: end,
        macro_defs_count: 4, // points past EOF!
        ..Header::default()
    }
    .to_le();
    let mut raw_bad = Vec::from(unsafe { std::slice::from_raw_parts((&bad_hdr as *const Header) as *const u8, hdr_sz) });
    raw_bad.extend_from_slice(&[0u8; 8]);
    let aligned_bad = copy_aligned(&raw_bad);
    assert!(IndexReader::from_buffer(&aligned_bad.0[..raw_bad.len()]).is_err());
}
