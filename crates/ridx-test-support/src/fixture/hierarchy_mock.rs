/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Hierarchy and macro fixture modeling traits, impls, calls, and macro expansions.

use crate::cursor_dsl::parse_fixture;
use crate::fixture::FixtureIndexBuilder;
use ridx_format::DefKind;
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// Hermetic fixture for testing Call Hierarchy, Type Hierarchy (supertypes/subtypes),
/// and macro-expanded definitions/references.
pub struct MacroAndTraitHierarchyFixture {
    _temp_dir: TempDir,
    crate_name: String,
    file_path: PathBuf,
    index_path: PathBuf,
    uri: String,
    builder: FixtureIndexBuilder,
    source_code: String,
    def_spans: HashMap<u32, (u32, u32, u32, u32)>,
}

impl MacroAndTraitHierarchyFixture {
    /// Creates a new hierarchy and macro fixture in a fresh temporary directory.
    pub fn new(crate_name: &str, rel_path: &str) -> io::Result<Self> {
        let temp_dir = TempDir::new()?;
        let root = temp_dir.path().canonicalize().unwrap_or_else(|_| temp_dir.path().to_path_buf());
        let file_path = root.join(rel_path);

        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let target_dir = root.join("target/ridx");
        fs::create_dir_all(&target_dir)?;
        let index_path = target_dir.join(format!("{}.ridx", crate_name));

        let uri = format!("file://{}", file_path.display());

        Ok(Self {
            _temp_dir: temp_dir,
            crate_name: crate_name.to_string(),
            file_path,
            index_path,
            uri,
            builder: FixtureIndexBuilder::new(crate_name, "0.1.0"),
            source_code: String::new(),
            def_spans: HashMap::new(),
        })
    }

    /// Sets and writes the raw Rust source code to disk.
    pub fn set_source(&mut self, code: &str) -> io::Result<()> {
        self.source_code = code.to_string();
        fs::write(&self.file_path, code)
    }

    fn check_coords(&self, name: &str, line: u32, start_col: u32, end_col: u32) {
        if !self.source_code.is_empty() {
            let l_idx = (line.saturating_sub(1)) as usize;
            if let Some(l) = self.source_code.lines().nth(l_idx) {
                let chars: Vec<char> = l.chars().collect();
                let sc = (start_col.saturating_sub(1)) as usize;
                let ec = (end_col.saturating_sub(1)) as usize;
                if sc <= chars.len() && ec <= chars.len() && sc <= ec {
                    let slice: String = chars[sc..ec].iter().collect();
                    assert_eq!(
                        slice, name,
                        "Coordinate mismatch at line {}: expected '{}', found '{}'",
                        line, name, slice
                    );
                }
            }
        }
    }

    /// Adds a trait definition with 1-based char coordinates.
    pub fn add_trait(&mut self, name: &str, line: u32, start_col: u32, end_col: u32) -> u32 {
        self.check_coords(name, line, start_col, end_col);
        let path = format!("{}::{}", self.crate_name, name);
        let abs_str = self.file_path.to_str().unwrap();
        let id = self.builder.add_def(name, &path, DefKind::Trait, abs_str, line, start_col, line, end_col);
        self.def_spans.insert(id, (line, start_col, line, end_col));
        id
    }

    /// Adds a struct or enum type definition with 1-based char coordinates.
    pub fn add_type(&mut self, name: &str, kind: DefKind, line: u32, start_col: u32, end_col: u32) -> u32 {
        self.check_coords(name, line, start_col, end_col);
        let path = format!("{}::{}", self.crate_name, name);
        let abs_str = self.file_path.to_str().unwrap();
        let id = self.builder.add_def(name, &path, kind, abs_str, line, start_col, line, end_col);
        self.def_spans.insert(id, (line, start_col, line, end_col));
        id
    }

    /// Adds a trait implementation linking a target type to a trait (`<Target as Trait>`).
    pub fn add_impl_block(&mut self, target_name: &str, trait_name: Option<&str>, line: u32, start_col: u32, end_col: u32) -> u32 {
        let impl_path = match trait_name {
            Some(tr) => format!("<{}::{} as {}::{}>", self.crate_name, target_name, self.crate_name, tr),
            None => format!("<{}::{}>", self.crate_name, target_name),
        };
        let def_name = match trait_name {
            Some(tr) => format!("impl {} for {}", tr, target_name),
            None => format!("impl {}", target_name),
        };
        let abs_str = self.file_path.to_str().unwrap();
        let id = self.builder.add_def(&def_name, &impl_path, DefKind::Impl, abs_str, line, start_col, line, end_col);
        self.def_spans.insert(id, (line, start_col, line, end_col));
        id
    }

    /// Adds a function definition for call hierarchy testing.
    pub fn add_fn(&mut self, name: &str, line: u32, start_col: u32, end_col: u32) -> u32 {
        self.check_coords(name, line, start_col, end_col);
        let path = format!("{}::{}", self.crate_name, name);
        let abs_str = self.file_path.to_str().unwrap();
        let id = self.builder.add_def(name, &path, DefKind::Fn, abs_str, line, start_col, line, end_col);
        self.def_spans.insert(id, (line, start_col, line, end_col));
        id
    }

    /// Records a function call reference targeting a known callee definition ID.
    pub fn add_call(&mut self, callee_path: &str, callee_def_id: u32, call_line: u32, call_start_col: u32, call_end_col: u32) {
        let (target_line, target_col, _, _) = *self.def_spans.get(&callee_def_id).unwrap_or_else(|| {
            panic!("MacroAndTraitHierarchyFixture::add_call: callee_def_id {} not registered", callee_def_id)
        });
        let abs_str = self.file_path.to_str().unwrap();
        self.builder.add_ref(
            callee_path, abs_str, call_line, call_start_col, call_line, call_end_col,
            Some(callee_def_id), Some(abs_str), target_line, target_col, DefKind::Fn,
        );
    }

    /// Adds a macro definition.
    pub fn add_macro(&mut self, name: &str, line: u32, start_col: u32, end_col: u32) -> u32 {
        self.check_coords(name, line, start_col, end_col);
        let path = format!("{}::{}", self.crate_name, name);
        let abs_str = self.file_path.to_str().unwrap();
        let id = self.builder.add_def(name, &path, DefKind::Macro, abs_str, line, start_col, line, end_col);
        self.def_spans.insert(id, (line, start_col, line, end_col));
        id
    }

    /// Adds definitions from an annotated fixture string containing `«name»` spans.
    pub fn add_defs_from_fixture(&mut self, annotated_code: &str, kind: DefKind) -> (String, Vec<u32>) {
        let parsed = parse_fixture(annotated_code);
        let abs_str = self.file_path.to_str().unwrap();
        let mut ids = Vec::new();
        for span in &parsed.spans {
            let full_path = format!("{}::{}", self.crate_name, span.text);
            let s_line = span.start_line + 1;
            let s_col = span.start_col_char + 1;
            let e_line = span.end_line + 1;
            let e_col = span.end_col_char + 1;
            let id = self.builder.add_def(&span.text, &full_path, kind, abs_str, s_line, s_col, e_line, e_col);
            self.def_spans.insert(id, (s_line, s_col, e_line, e_col));
            ids.push(id);
        }
        self.source_code = parsed.clean_text.clone();
        let _ = fs::write(&self.file_path, &parsed.clean_text);
        (parsed.clean_text, ids)
    }

    /// Writes the `.ridx` index shard to disk and returns its path without consuming the fixture.
    pub fn build_index(&self) -> io::Result<PathBuf> {
        self.builder.write_to_file(&self.index_path);
        Ok(self.index_path.clone())
    }

    pub fn uri(&self) -> &str { &self.uri }
    pub fn file_path(&self) -> &Path { &self.file_path }
    pub fn index_path(&self) -> &Path { &self.index_path }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ridx_format::IndexReader;

    #[test]
    fn test_hierarchy_and_macro_coordinates() {
        let mut fixture = MacroAndTraitHierarchyFixture::new("hier_pkg", "src/lib.rs").unwrap();

        // 1. Trait definition with Unicode comment:
        // "/* 🦀 */ trait «Zeigbar» {}\n" -> 1-based start col 15, end col 22.
        let code = "/* 🦀 */ trait «Zeigbar» {}\n";
        let (clean, def_ids) = fixture.add_defs_from_fixture(code, DefKind::Trait);
        assert_eq!(clean, "/* 🦀 */ trait Zeigbar {}\n");
        assert_eq!(def_ids.len(), 1);

        // 2. Struct implementing the trait
        let struct_id = fixture.add_type("Widget", DefKind::Struct, 2, 8, 14);

        // 3. Impl block linking Widget to Zeigbar
        let impl_id = fixture.add_impl_block("Widget", Some("Zeigbar"), 3, 1, 30);

        // 4. Caller & callee functions for call hierarchy
        let callee_id = fixture.add_fn("compute", 5, 4, 11);
        fixture.add_call("hier_pkg::compute", callee_id, 8, 5, 12);

        // 5. Macro definition
        let macro_id = fixture.add_macro("make_widget", 10, 1, 12);

        let index_path = fixture.build_index().unwrap();
        assert!(index_path.is_file());

        let reader = IndexReader::open(&index_path).unwrap();
        let defs = reader.definitions();
        assert_eq!(defs.len(), 5);

        // Verify trait coordinates
        let trait_def = &defs[0];
        assert_eq!(reader.get_str(trait_def.name_offset), "Zeigbar");
        assert_eq!(trait_def.kind, DefKind::Trait as u8);
        assert_eq!(trait_def.span.start_line(), 1);
        assert_eq!(trait_def.span.start_col(), 15);
        assert_eq!(trait_def.span.end_col(), 22);

        // Verify struct and impl
        assert_eq!(defs[1].def_id, struct_id);
        assert_eq!(defs[1].kind, DefKind::Struct as u8);
        assert_eq!(defs[2].def_id, impl_id);
        assert_eq!(defs[2].kind, DefKind::Impl as u8);
        assert_eq!(reader.get_str(defs[2].path_offset), "<hier_pkg::Widget as hier_pkg::Zeigbar>");

        // Verify call reference coordinates match callee definition (not 1:1!)
        let refs = reader.references();
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].target_def_id, callee_id);
        assert_eq!(refs[0].target_line, 5);
        assert_eq!(refs[0].target_col, 4);
        assert_eq!(reader.get_str(refs[0].target_path_offset), "hier_pkg::compute");

        // Verify macro
        assert_eq!(defs[4].def_id, macro_id);
        assert_eq!(defs[4].kind, DefKind::Macro as u8);
    }

    #[test]
    #[should_panic(expected = "callee_def_id 9999 not registered")]
    fn test_add_call_unregistered_callee_panics() {
        let mut fixture = MacroAndTraitHierarchyFixture::new("hier_pkg", "src/lib.rs").unwrap();
        fixture.add_call("hier_pkg::missing", 9999, 1, 1, 10);
    }
}
