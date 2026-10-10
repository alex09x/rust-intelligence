mod common;

use common::unique_temp_dir;
use ridx_format::{DefKind, IndexReader, IndexWriter, RawDefinition, WorkspaceIndex};
use std::fs::{self, File};
use std::io::BufWriter;

#[test]
fn test_version_mismatch_rejected() {
    let test_dir = unique_temp_dir("ridx_ver_test");

    let manifest_future = serde_json::json!({
        "version": "2.0",
        "format_version": 999, // Future format version
        "active_variant_id": "var_future",
        "variant_dir": "variants/var_future",
        "workspace_root": test_dir.display().to_string(),
        "manifest_path": test_dir.join("Cargo.toml").display().to_string(),
        "profile": "dev",
    });

    let var_dir = test_dir.join("variants/var_future");
    fs::create_dir_all(&var_dir).unwrap();

    let mut writer = IndexWriter::new("test_crate", "0.1.0");
    let f = File::create(var_dir.join("test_crate.ridx")).unwrap();
    writer.write_to(&mut BufWriter::new(f)).unwrap();

    fs::write(
        test_dir.join("ridx_manifest.json"),
        serde_json::to_string_pretty(&manifest_future).unwrap(),
    )
    .unwrap();

    let res = WorkspaceIndex::open(&test_dir);
    assert!(res.is_err(), "should reject future format_version");
    let err = res.err().unwrap();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    assert!(ridx_format::is_version_mismatch(&err));

    let _ = fs::remove_dir_all(&test_dir);
}

#[test]
fn test_normalized_source_hash_crlf_and_bom() {
    let lf_bytes = b"pub fn hello() {\n    println!(\"hello\");\n}\n";
    let crlf_bytes = b"pub fn hello() {\r\n    println!(\"hello\");\r\n}\r\n";
    let bom_crlf_bytes = b"\xEF\xBB\xBFpub fn hello() {\r\n    println!(\"hello\");\r\n}\r\n";

    let hash_lf = ridx_format::normalized_source_hash(lf_bytes);
    let hash_crlf = ridx_format::normalized_source_hash(crlf_bytes);
    let hash_bom_crlf = ridx_format::normalized_source_hash(bom_crlf_bytes);

    assert_eq!(
        hash_lf, hash_crlf,
        "CRLF and LF must produce identical normalized hash"
    );
    assert_eq!(
        hash_lf, hash_bom_crlf,
        "BOM+CRLF and LF must produce identical normalized hash"
    );
}

#[test]
fn test_v1_format_rejection() {
    let test_dir = unique_temp_dir("ridx_v1_rejection");

    let mut writer = IndexWriter::new("old_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "old_func".to_string(),
        path: "old_crate::old_func".to_string(),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 10,
    });
    let valid_path = test_dir.join("valid_v2.ridx");
    writer
        .write_to(&mut BufWriter::new(File::create(&valid_path).unwrap()))
        .unwrap();

    let mut bytes = fs::read(&valid_path).unwrap();
    // Overwrite format version in Header (offset 4..8) with 1
    bytes[4..8].copy_from_slice(&1u32.to_le_bytes());
    let v1_path = test_dir.join("fake_v1.ridx");
    fs::write(&v1_path, &bytes).unwrap();

    let res = IndexReader::open(&v1_path);
    assert!(res.is_err(), "v1 index format must be rejected");
    let err = res.err().unwrap();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    assert!(err.to_string().contains("Unsupported RIDX version: 1"));

    let _ = fs::remove_dir_all(&test_dir);
}

#[test]
fn test_binary_cache_inputs_raw_hashing() {
    let crlf: &[u8] = &[0, 13, 10, 255];
    let lf: &[u8] = &[0, 10, 255];
    let h1 = ridx_format::fnv1a_64(crlf);
    let h2 = ridx_format::fnv1a_64(lf);
    assert_ne!(h1, h2, "raw binary hashing must not equate CRLF and LF");
}

#[test]
fn test_v2_file_table_requires_full_16_byte_entry() {
    let test_dir = unique_temp_dir("ridx_test_file_table_16");

    let mut writer = IndexWriter::new("test_mod", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "test_fn".to_string(),
        path: "test_mod::test_fn".to_string(),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 10,
    });

    let valid_path = test_dir.join("valid.ridx");
    writer
        .write_to(&mut BufWriter::new(File::create(&valid_path).unwrap()))
        .unwrap();

    let buf = fs::read(&valid_path).unwrap();
    let file_len = buf.len();

    // 1. Truncated file entry: files_count = 1, but files_offset leaves only 8 bytes (< 16 required bytes)
    let mut truncated_buf = buf.clone();
    let files_offset = (file_len - 8) as u64;
    // Header files_offset is at offset 32..40, files_count is at 40..44
    truncated_buf[32..40].copy_from_slice(&files_offset.to_le_bytes());
    truncated_buf[40..44].copy_from_slice(&1u32.to_le_bytes());
    let trunc_path = test_dir.join("trunc.ridx");
    fs::write(&trunc_path, &truncated_buf).unwrap();

    let res = IndexReader::open(&trunc_path);
    assert!(
        res.is_err(),
        "IndexReader::open must reject truncated file table where 16-byte FileEntry extends beyond EOF"
    );

    // 2. Unaligned files table: files_offset not aligned to 8 bytes
    let mut unaligned_buf = buf;
    let unaligned_offset = 36u64; // multiple of 4, but not 8
    unaligned_buf[32..40].copy_from_slice(&unaligned_offset.to_le_bytes());
    unaligned_buf[40..44].copy_from_slice(&1u32.to_le_bytes());
    let unaligned_path = test_dir.join("unaligned.ridx");
    fs::write(&unaligned_path, &unaligned_buf).unwrap();

    let res_unaligned = IndexReader::open(&unaligned_path);
    assert!(
        res_unaligned.is_err(),
        "IndexReader::open must reject files table whose offset is not 8-byte aligned"
    );

    let _ = fs::remove_dir_all(&test_dir);
}

#[test]
fn test_driver_hash_cache_by_path_size_mtime() {
    let test_dir = unique_temp_dir("ridx_drv_hash_test");
    let bin_path = test_dir.join("mock_driver");
    let cache_dir = test_dir.join("cache");

    // Write initial driver mock (20 bytes)
    fs::write(&bin_path, b"driver mock binary 1").unwrap();

    // Use explicit path and cache_dir parameter injection (no global env mutation)
    ridx_format::clear_driver_hash_cache_in(Some(&cache_dir));

    let h1 = ridx_format::get_driver_hash_with(Some(&bin_path), Some(&cache_dir));
    assert_ne!(h1, 0);

    // Call second time: should hit in-memory and disk cache
    let h2 = ridx_format::get_driver_hash_with(Some(&bin_path), Some(&cache_dir));
    assert_eq!(h1, h2);

    // Verify disk cache file was created in isolated cache dir
    assert!(cache_dir.join("driver_hash.cache").is_file());

    // Test mtime branch with IDENTICAL size:
    // Sleep >= 1.05s to ensure mtime_secs advances, then write content of exact same length (20 bytes)
    std::thread::sleep(std::time::Duration::from_millis(1050));
    fs::write(&bin_path, b"driver mock binary 2").unwrap();

    let h3 = ridx_format::get_driver_hash_with(Some(&bin_path), Some(&cache_dir));
    assert_ne!(
        h1, h3,
        "hash must update when driver mtime changes even if binary size is identical"
    );

    // Clean up isolated cache dir
    ridx_format::clear_driver_hash_cache_in(Some(&cache_dir));
    assert!(!cache_dir.join("driver_hash.cache").exists());
    let _ = fs::remove_dir_all(&test_dir);
}


