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
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[test]
fn test_cache_helpers() {
    std::env::set_var("RIDX_CACHE_DIR", "/custom/cache");
    let dir = resolve_cache_dir();
    assert_eq!(dir, PathBuf::from("/custom/cache"));
    std::env::remove_var("RIDX_CACHE_DIR");

    let cache_base = Path::new("/var/cache/ridx");
    let p1 = cache_object_path(cache_base, "abcdef");
    assert_eq!(p1, cache_base.join("objects/ab/abcdef.ridx"));

    let p2 = cache_object_path(cache_base, "x");
    assert_eq!(p2, cache_base.join("objects/00/x.ridx"));
}

#[test]
fn test_format_struct_accessors() {
    let fe = FileEntry { path_offset: 12, _pad: 0, source_hash: 12345 };
    assert_eq!(fe.path_offset(), 12);
    assert_eq!(fe.source_hash(), 12345);

    let hdr = Header {
        magic: MAGIC,
        version: CURRENT_VERSION,
        crate_name_offset: 0,
        crate_version_offset: 0,
        string_pool_offset: 0,
        string_pool_len: 0,
        files_offset: 0,
        files_count: 0,
        unit_key_offset: 42,
        defs_offset: 0,
        defs_count: 0,
        generation_offset: 84,
        defs_by_hash_offset: 0,
        defs_by_hash_count: 0,
        _pad2: 0,
        refs_offset: 0,
        refs_count: 0,
        _pad3: 0,
        refs_by_target_offset: 0,
        refs_by_target_count: 0,
        _pad4: 0,
        types_offset: 0,
        types_count: 0,
        _pad5: 0,
        defs_by_owner_offset: 0,
        defs_by_owner_count: 0,
        _pad6: 0,
        derefs_offset: 0,
        derefs_count: 0,
        _pad7: 0,
    };
    let hdr_le = hdr.to_le();
    let hdr_from = hdr_le.from_le();
    assert_eq!(hdr_from.unit_key_offset(), 42);
    assert_eq!(hdr_from.generation_offset(), 84);

    let def = DefEntry { def_id: 1, name_offset: 10, path_offset: 20, sig_offset: 30, span: Span::new(0, 1, 1, 2, 2), kind: 1, visibility: 0, flags: 1, _pad: 0 };
    let def_le = def.to_le();
    let def_from = def_le.from_le();
    assert_eq!(def.def_id, def_from.def_id);

    let r = RefEntry {
        span: Span::new(0, 1, 1, 2, 2),
        target_def_id: 1,
        target_path_offset: 10,
        target_file_offset: 100,
        target_unit_offset: 200,
        target_line: 15,
        target_col: 4,
        kind: 1,
        is_write: 1,
        flags: FLAG_MACRO_EXPANSION,
        _pad: 0,
    };
    let r_le = r.to_le();
    let r_from = r_le.from_le();
    assert_eq!(r_from.target_file_offset(), 100);
    assert_eq!(r_from.target_unit_offset(), 200);
    assert_eq!(r_from.target_line(), 15);
    assert_eq!(r_from.target_col(), 4);
    assert!(r_from.is_macro_expansion());

    let ty = TypeEntry { span: Span::new(0, 1, 1, 2, 2), type_offset: 50, owner_path_offset: 120 };
    let ty_le = ty.to_le();
    let ty_from = ty_le.from_le();
    assert_eq!(ty.type_offset, ty_from.type_offset);
    assert_eq!(ty.owner_path_offset, ty_from.owner_path_offset);

    let h_entry = HashIndexEntry { hash: 9999, index: 7, _pad: [0; 4] };
    let h_le = h_entry.to_le();
    let h_from = h_le.from_le();
    assert_eq!(h_from.hash(), 9999);
    assert_eq!(h_from.index(), 7);

    let owner_entry = OwnerIndexEntry::new(123456789, 42, 10);
    let owner_le = owner_entry.to_le();
    let owner_from = owner_le.from_le();
    assert_eq!(owner_from.hash(), 123456789);
    assert_eq!(owner_from.owner_path_offset(), 42);
    assert_eq!(owner_from.def_index(), 10);

    let deref_entry = DerefEntry::new(987654321, 100, 200);
    let deref_le = deref_entry.to_le();
    let deref_from = deref_le.from_le();
    assert_eq!(deref_from.source_type_hash(), 987654321);
    assert_eq!(deref_from.source_type_offset(), 100);
    assert_eq!(deref_from.target_type_offset(), 200);
}

#[test]
fn test_merge_embedded_hashes_and_digest() {
    let mut map = HashMap::new();
    map.insert("src/lib.rs".to_string(), 12345u64);
    let digest = compute_source_snapshot_digest(&map);
    assert_ne!(digest, 0);

    let mut existing = HashMap::new();
    existing.insert("src/lib.rs".to_string(), 99999u64); // conflict
    existing.insert("src/other.rs".to_string(), 88888u64);

    let mut embedded = HashMap::new();
    embedded.insert("src/lib.rs".to_string(), 12345u64);

    merge_embedded_hashes_authoritative(&mut existing, &embedded, None);
    assert_eq!(existing.get("src/lib.rs"), Some(&12345u64));
    assert_eq!(existing.get("src/other.rs"), Some(&88888u64));
}

#[test]
fn test_def_kind_all_variants_and_as_str() {
    for val in 0..=33 {
        let kind = DefKind::from(val);
        let s = kind.as_str();
        assert!(!s.is_empty());
    }
}

#[test]
fn test_resolve_cache_dir_and_object_path() {
    let _lock = crate::workspace::ENV_LOCK.lock().unwrap();
    std::env::set_var("RIDX_CACHE_DIR", "/custom/ridx_cache");

    assert_eq!(resolve_cache_dir(), std::path::PathBuf::from("/custom/ridx_cache"));
    std::env::remove_var("RIDX_CACHE_DIR");

    std::env::set_var("XDG_CACHE_HOME", "/custom/xdg");
    let xdg_res = resolve_cache_dir();
    assert!(xdg_res.starts_with("/custom/xdg/ridx"));
    std::env::remove_var("XDG_CACHE_HOME");

    // Empty string variants and fallback (lines 30, 36, 45-46, 54)
    std::env::set_var("RIDX_CACHE_DIR", "   ");
    std::env::set_var("XDG_CACHE_HOME", "   ");
    std::env::set_var("HOME", "/custom/home");
    let home_res = resolve_cache_dir();
    assert!(home_res.starts_with("/custom/home/.cache/ridx"));

    std::env::set_var("HOME", "   ");
    let fallback_res = resolve_cache_dir();
    assert!(fallback_res.starts_with(".ridx_cache"));
    std::env::remove_var("RIDX_CACHE_DIR");
    std::env::remove_var("XDG_CACHE_HOME");
    std::env::remove_var("HOME");

    let obj1 = cache_object_path(Path::new("/cache"), "a");
    assert_eq!(obj1, std::path::PathBuf::from("/cache/objects/00/a.ridx"));

    let obj2 = cache_object_path(Path::new("/cache"), "12345");
    assert_eq!(obj2, std::path::PathBuf::from("/cache/objects/12/12345.ridx"));
}

#[test]
fn test_join_resolving_overlap_variants() {
    let abs_rel = Path::new("/var/foo/bar.rs");
    assert_eq!(join_resolving_overlap(Path::new("/root"), abs_rel), abs_rel);

    let root = Path::new("/workspace/crates/app");
    let rel = Path::new("crates/app/src/lib.rs");
    let joined = join_resolving_overlap(root, rel);
    assert_eq!(joined, Path::new("/workspace/crates/app/src/lib.rs"));

    let rel_non = Path::new("other/file.rs");
    let joined_non = join_resolving_overlap(root, rel_non);
    assert_eq!(joined_non, Path::new("/workspace/crates/app/other/file.rs"));

    // Direct exists branch (line 81)
    let temp = std::env::temp_dir().join(format!("test_overlap_{}", std::process::id()));
    let _ = std::fs::create_dir_all(temp.join("src"));
    let test_file = temp.join("src/main.rs");
    let _ = std::fs::write(&test_file, b"fn main() {}");
    assert_eq!(join_resolving_overlap(&temp, Path::new("src/main.rs")), test_file);
    let _ = std::fs::remove_dir_all(&temp);

    // Span contains with mismatched file_id (line 418)
    let span = Span::new(1, 10, 1, 10, 5);
    assert!(!span.contains(999, 10, 2));
}

#[test]
fn test_repr_c_sizes_alignments_and_field_offsets() {
    macro_rules! check_layout {
        ($ty:ty, $sz:expr, $al:expr, [ $( $field:ident : $off:expr ),+ $(,)? ]) => {
            assert_eq!(std::mem::size_of::<$ty>(), $sz);
            assert_eq!(std::mem::align_of::<$ty>(), $al);
            $( assert_eq!(std::mem::offset_of!($ty, $field), $off); )+
        };
    }
    assert_eq!(std::mem::size_of::<DefKind>(), 1);
    assert_eq!(std::mem::align_of::<DefKind>(), 1);
    check_layout!(FileEntry, 16, 8, [path_offset: 0, _pad: 4, source_hash: 8]);
    check_layout!(Header, 160, 8, [
        magic: 0, version: 4, crate_name_offset: 8, crate_version_offset: 12,
        string_pool_offset: 16, string_pool_len: 24, files_offset: 32, files_count: 40,
        unit_key_offset: 44, defs_offset: 48, defs_count: 56, generation_offset: 60,
        defs_by_hash_offset: 64, defs_by_hash_count: 72, _pad2: 76,
        refs_offset: 80, refs_count: 88, _pad3: 92,
        refs_by_target_offset: 96, refs_by_target_count: 104, _pad4: 108,
        types_offset: 112, types_count: 120, _pad5: 124,
        defs_by_owner_offset: 128, defs_by_owner_count: 136, _pad6: 140,
        derefs_offset: 144, derefs_count: 152, _pad7: 156,
    ]);
    check_layout!(HeaderV2, 128, 8, [
        magic: 0, version: 4, crate_name_offset: 8, crate_version_offset: 12,
        string_pool_offset: 16, string_pool_len: 24, files_offset: 32, files_count: 40,
        unit_key_offset: 44, defs_offset: 48, defs_count: 56, generation_offset: 60,
        defs_by_hash_offset: 64, defs_by_hash_count: 72, _pad2: 76,
        refs_offset: 80, refs_count: 88, _pad3: 92,
        refs_by_target_offset: 96, refs_by_target_count: 104, _pad4: 108,
        types_offset: 112, types_count: 120, _pad5: 124,
    ]);
    check_layout!(Span, 20, 4, [
        file_id: 0, start_line: 4, start_col: 8, end_line: 12, end_col: 16,
    ]);
    check_layout!(DefEntry, 40, 4, [
        def_id: 0, kind: 4, visibility: 5, flags: 6, _pad: 7,
        name_offset: 8, path_offset: 12, sig_offset: 16, span: 20,
    ]);
    check_layout!(RefEntry, 48, 4, [
        span: 0, target_def_id: 20, target_path_offset: 24, target_file_offset: 28,
        target_unit_offset: 32, target_line: 36, target_col: 40,
        kind: 44, is_write: 45, flags: 46, _pad: 47,
    ]);
    check_layout!(TypeEntry, 28, 4, [span: 0, type_offset: 20, owner_path_offset: 24]);
    check_layout!(HashIndexEntry, 16, 8, [hash: 0, index: 8, _pad: 12]);
    check_layout!(OwnerIndexEntry, 16, 8, [hash: 0, owner_path_offset: 8, def_index: 12]);
    check_layout!(DerefEntry, 16, 8, [
        source_type_hash: 0, source_type_offset: 8, target_type_offset: 12,
    ]);
}

#[test]
fn test_index_writer_golden_bytes() {
    use crate::writer::{IndexWriter, RawDefinition, RawReference, RawType};
    let mut w = IndexWriter::new("golden_crate", "0.1.0");
    w.set_unit_key("golden_crate-0123456789abcdef");
    w.set_generation("gen-fedcba9876543210");
    w.add_file_hash("src/lib.rs", 0x1111_2222_3333_4444);
    w.add_file_hash("src/model.rs", 0x5555_6666_7777_8888);
    w.add_file_hash("src/util.rs", 0x9999_aaaa_bbbb_cccc);
    let mk_def = |id, kind, vis, mac, path: &str, sig: Option<&str>, f: &str, l: u32| RawDefinition {
        def_id: id,
        kind,
        visibility: vis,
        is_macro_expansion: mac,
        name: path.rsplit("::").next().unwrap().into(),
        path: path.into(),
        signature: sig.map(Into::into),
        file: f.into(),
        start_line: l,
        start_col: 1,
        end_line: l + 2,
        end_col: 2,
    };
    let (p1, s1) = ("golden_crate::model::Widget", Some("pub struct Widget"));
    let (p2, s2) = ("golden_crate::model::Widget::render", Some("pub fn render(&self) -> usize"));
    let (p3, s3) = ("golden_crate::util::helper", Some("pub(crate) fn helper(x: u32) -> u32"));
    let (m, p4) = ("src/model.rs", "golden_crate::Widget");
    w.add_definition(mk_def(1, DefKind::Struct, 0, false, p1, s1, m, 1));
    w.add_definition_with_owners(mk_def(2, DefKind::AssocFn, 0, false, p2, s2, m, 12), vec![p1.into()]);
    w.add_definition(mk_def(3, DefKind::Fn, 1, true, p3, s3, "src/util.rs", 3));
    let d4 = mk_def(4, DefKind::Use, 0, false, p4, None, "src/lib.rs", 5);
    w.add_reexport_with_owners(d4, vec!["golden_crate".into()]);
    let mk_ref = |l, c2, id, path: &str, unit: Option<&str>, tl, tc, kind, wr| RawReference {
        file: "src/lib.rs".into(),
        start_line: l,
        start_col: if wr { 9 } else { 5 },
        end_line: l,
        end_col: c2,
        target_def_id: Some(id),
        target_path: path.into(),
        target_file: Some("src/model.rs".into()),
        target_unit: unit.map(Into::into),
        target_line: tl,
        target_col: tc,
        kind,
        is_write: wr,
        is_macro_expansion: wr,
    };
    let u_key = Some("golden_crate-0123456789abcdef");
    w.add_reference(mk_ref(10, 11, 1, p1, u_key, 1, 1, DefKind::Struct, false));
    w.add_reference(mk_ref(11, 15, 2, p2, None, 12, 5, DefKind::AssocFn, true));
    let mk_ty = |f: &str, l, c2, name: &str| RawType { file: f.into(), start_line: l, start_col: 5, end_line: l, end_col: c2, type_name: name.into() };
    w.add_type_with_owner(mk_ty("src/lib.rs", 10, 20, p1), Some(p1.into()));
    w.add_type(mk_ty("src/util.rs", 4, 6, "u32"));
    w.add_deref(p1, "golden_crate::util::Target");
    let mut bytes = Vec::new();
    w.write_to(&mut std::io::Cursor::new(&mut bytes)).unwrap();
    assert_eq!(bytes.len(), 1096);
    assert_eq!(fnv1a_64(&bytes), 0xd45a_2e34_8f1b_8421);
}
