/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Overlay lifecycle fixture modeling unsaved document buffers and version shifts.

use crate::edit::apply_text_edits;
use crate::fixture::FixtureIndexBuilder;
use ridx_format::DefKind;
use serde_json::Value;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// Hermetic fixture modeling document overlay lifecycle: opening, unsaved edits,
/// version increment tracking, coordinate shifting, and saving to disk.
pub struct OverlayLifecycleFixture {
    _temp_dir: TempDir,
    crate_name: String,
    file_path: PathBuf,
    index_path: PathBuf,
    uri: String,
    version: i32,
    disk_content: String,
    overlay_content: Option<String>,
}

impl OverlayLifecycleFixture {
    /// Creates a new overlay lifecycle fixture with base content on disk and a compiled index.
    pub fn new(crate_name: &str, rel_path: &str, base_code: &str) -> io::Result<Self> {
        let temp_dir = TempDir::new()?;
        let root = temp_dir.path().canonicalize().unwrap_or_else(|_| temp_dir.path().to_path_buf());
        let file_path = root.join(rel_path);

        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&file_path, base_code)?;

        let target_dir = root.join("target/ridx");
        fs::create_dir_all(&target_dir)?;
        let index_path = target_dir.join(format!("{}.ridx", crate_name));

        let uri = format!("file://{}", file_path.display());

        let mut fixture =
            Self { _temp_dir: temp_dir, crate_name: crate_name.to_string(), file_path, index_path, uri, version: 0, disk_content: base_code.to_string(), overlay_content: None };

        fixture.rebuild_index_with_defs(base_code, DefKind::Fn)?;
        Ok(fixture)
    }

    /// Simulates LSP `textDocument/didOpen`, initializing in-memory buffer at specified version.
    pub fn did_open(&mut self, version: i32, text: Option<&str>) {
        self.version = version;
        self.overlay_content = Some(text.unwrap_or(&self.disk_content).to_string());
    }

    /// Simulates LSP full-content `textDocument/didChange`.
    pub fn did_change_full(&mut self, new_version: i32, new_text: &str) {
        assert!(new_version > self.version, "LSP version must monotonically increase");
        self.version = new_version;
        self.overlay_content = Some(new_text.to_string());
    }

    /// Simulates LSP incremental `textDocument/didChange` using JSON TextEdit array.
    pub fn did_change_incremental(&mut self, new_version: i32, edits: &[Value]) {
        assert!(new_version > self.version, "LSP version must monotonically increase");
        self.version = new_version;
        let current = self.current_text();
        let updated = apply_text_edits(current, edits);
        self.overlay_content = Some(updated);
    }

    /// Simulates LSP `textDocument/didSave`, writing overlay contents to disk and clearing dirty flag.
    pub fn did_save(&mut self) -> io::Result<()> {
        if let Some(ref text) = self.overlay_content {
            fs::write(&self.file_path, text)?;
            self.disk_content = text.clone();
        }
        Ok(())
    }

    /// Simulates LSP `textDocument/didClose`, dropping in-memory overlay buffer.
    pub fn did_close(&mut self) {
        self.overlay_content = None;
        self.version = 0;
    }

    /// Returns whether the in-memory buffer currently diverges from disk content.
    pub fn is_dirty(&self) -> bool {
        match &self.overlay_content {
            Some(text) => text != &self.disk_content,
            None => false,
        }
    }

    /// Current document version (0 if closed).
    pub fn version(&self) -> i32 {
        self.version
    }

    /// Document URI string.
    pub fn uri(&self) -> &str {
        &self.uri
    }

    /// Absolute path to the source file.
    pub fn file_path(&self) -> &Path {
        &self.file_path
    }

    /// Absolute path to the `.ridx` index shard file.
    pub fn index_path(&self) -> &Path {
        &self.index_path
    }

    /// Current visible text (overlay buffer if open, disk content otherwise).
    pub fn current_text(&self) -> &str {
        self.overlay_content.as_deref().unwrap_or(&self.disk_content)
    }

    /// Current on-disk text.
    pub fn disk_text(&self) -> &str {
        &self.disk_content
    }

    /// Rebuilds the on-disk `.ridx` index shard using definitions parsed from `annotated_code`.
    /// Synchronizes both disk content and in-memory overlay buffer if active, preserving clean state.
    pub fn rebuild_index_with_defs(&mut self, annotated_code: &str, kind: DefKind) -> io::Result<Vec<u32>> {
        let mut builder = FixtureIndexBuilder::new(&self.crate_name, "0.1.0");
        let abs_str = self.file_path.to_str().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Non-UTF8 file path"))?;

        let (clean_code, def_ids) = builder.add_defs_with_ids(annotated_code, abs_str, &self.crate_name, kind);

        fs::write(&self.file_path, &clean_code)?;
        if self.overlay_content.is_some() {
            self.overlay_content = Some(clean_code.clone());
        }
        self.disk_content = clean_code;
        builder.write_to_file(&self.index_path);

        Ok(def_ids)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ridx_format::IndexReader;
    use serde_json::json;

    #[test]
    fn test_overlay_lifecycle_coordinates_and_shifts() {
        // Base content with Unicode: "/* 🦀 */ pub fn «basis»() {}\n"
        // 1-based start col: "/* " (3) + "🦀" (1) + " */ pub fn " (11) = 15 chars before marker -> col 16.
        let base = "/* 🦀 */ pub fn «basis»() {}\n";
        let mut fixture = OverlayLifecycleFixture::new("overlay_pkg", "src/lib.rs", base).unwrap();

        assert!(!fixture.is_dirty());
        assert_eq!(fixture.version(), 0);

        // Verify base index on disk
        let reader = IndexReader::open(fixture.index_path()).unwrap();
        let defs = reader.definitions();
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].span.start_col(), 16);
        assert_eq!(defs[0].span.end_col(), 21); // "basis" has 5 chars -> 16 + 5 = 21

        // 1. didOpen
        fixture.did_open(1, None);
        assert_eq!(fixture.version(), 1);
        assert!(!fixture.is_dirty());

        // 2. didChange incremental: prepend "/* 🌟 */ " to line 0
        // Length of "/* 🌟 */ " is 3 + 1 + 4 = 8 chars.
        // Old col 16 shifted by 8 chars -> new col 24.
        let edit = json!({
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": { "line": 0, "character": 0 }
            },
            "newText": "/* 🌟 */ "
        });
        fixture.did_change_incremental(2, &[edit]);
        assert_eq!(fixture.version(), 2);
        assert!(fixture.is_dirty());
        assert_eq!(fixture.current_text(), "/* 🌟 */ /* 🦀 */ pub fn basis() {}\n");

        // 3. didSave -> persists modified buffer to disk
        fixture.did_save().unwrap();
        assert!(!fixture.is_dirty());
        assert_eq!(fixture.disk_text(), "/* 🌟 */ /* 🦀 */ pub fn basis() {}\n");

        // 4. Rebuild index for saved content with shifted coordinates
        let new_annotated = "/* 🌟 */ /* 🦀 */ pub fn «basis»() {}\n";
        let new_ids = fixture.rebuild_index_with_defs(new_annotated, DefKind::Fn).unwrap();
        assert_eq!(new_ids.len(), 1);

        let reader2 = IndexReader::open(fixture.index_path()).unwrap();
        let defs2 = reader2.definitions();
        assert_eq!(defs2.len(), 1);
        // "/* 🌟 */ " (8 chars) + "/* 🦀 */ pub fn " (15 chars) = 23 chars before marker -> 1-based col 24!
        assert_eq!(defs2[0].span.start_col(), 24);
        assert_eq!(defs2[0].span.end_col(), 29);

        // 5. didClose
        fixture.did_close();
        assert_eq!(fixture.version(), 0);
        assert!(!fixture.is_dirty());
    }
}
