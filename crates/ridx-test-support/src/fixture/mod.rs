/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Index FixtureBuilder for creating test `.ridx` shards with driver-matching coordinates.

use crate::cursor_dsl::{parse_fixture, parse_spans, ParsedFixture};
use ridx_format::{DefKind, IndexWriter, RawDefinition, RawReference};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Builder to construct `.ridx` index files in memory or on disk for tests.
pub struct FixtureIndexBuilder {
    crate_name: String,
    crate_version: String,
    writer: IndexWriter,
    next_def_id: u32,
    defs: HashMap<u32, RawDefinition>,
}

impl FixtureIndexBuilder {
    pub fn new(crate_name: &str, crate_version: &str) -> Self {
        Self {
            crate_name: crate_name.to_string(),
            crate_version: crate_version.to_string(),
            writer: IndexWriter::new(crate_name, crate_version),
            next_def_id: 1,
            defs: HashMap::new(),
        }
    }

    pub fn crate_name(&self) -> &str {
        &self.crate_name
    }

    pub fn crate_version(&self) -> &str {
        &self.crate_version
    }

    /// Adds a raw definition with 1-based char coordinates (matching ridx-driver).
    #[allow(clippy::too_many_arguments)]
    pub fn add_def(
        &mut self,
        name: &str,
        path: &str,
        kind: DefKind,
        file: &str,
        start_line: u32,
        start_col: u32,
        end_line: u32,
        end_col: u32,
    ) -> u32 {
        let def_id = self.next_def_id;
        self.next_def_id += 1;

        let raw = RawDefinition {
            def_id,
            kind,
            visibility: 0,
            is_macro_expansion: false,
            file: file.to_string(),
            name: name.to_string(),
            start_line,
            start_col,
            end_line,
            end_col,
            path: path.to_string(),
            signature: Some(format!("pub {} {}", kind.as_str(), name)),
        };
        self.defs.insert(def_id, raw.clone());
        self.writer.add_definition(raw);

        def_id
    }

    /// Adds definitions extracted from a source text with `«name»` markers.
    ///
    /// Coordinates are recorded in 1-based Unicode scalar (char) columns,
    /// precisely matching `ridx-driver`'s `resolve_span_coords`.
    pub fn add_defs_from_fixture(
        &mut self,
        annotated_code: &str,
        file: &str,
        path_prefix: &str,
        kind: DefKind,
    ) -> String {
        let (clean_code, _) = self.add_defs_with_ids(annotated_code, file, path_prefix, kind);
        clean_code
    }

    /// Adds definitions from a fixture, returning both clean code and definition IDs.
    pub fn add_defs_with_ids(
        &mut self,
        annotated_code: &str,
        file: &str,
        path_prefix: &str,
        kind: DefKind,
    ) -> (String, Vec<u32>) {
        let (clean_code, spans) = parse_spans(annotated_code);
        let mut ids = Vec::new();
        for span in spans {
            let full_path = format!("{}::{}", path_prefix, span.text);
            let id = self.add_def(
                &span.text,
                &full_path,
                kind,
                file,
                span.start_line + 1,     // 1-based line
                span.start_col_char + 1, // 1-based Unicode scalar char column (driver format)
                span.end_line + 1,
                span.end_col_char + 1,
            );
            ids.push(id);
        }
        (clean_code, ids)
    }

    /// Adds definitions and self-references targeting them from a fixture with `«name»` and optional `$0`.
    pub fn add_defs_and_self_refs_fixture(
        &mut self,
        fixture: &str,
        file: &str,
        path_prefix: &str,
        kind: DefKind,
    ) -> (ParsedFixture, Vec<u32>) {
        let parsed = parse_fixture(fixture);
        let mut ids = Vec::new();
        for span in &parsed.spans {
            let full_path = format!("{}::{}", path_prefix, span.text);
            let start_line = span.start_line + 1;
            let start_col = span.start_col_char + 1;
            let end_line = span.end_line + 1;
            let end_col = span.end_col_char + 1;
            let id = self.add_def(
                &span.text, &full_path, kind, file, start_line, start_col, end_line, end_col,
            );
            self.add_ref(
                &full_path,
                file,
                start_line,
                start_col,
                end_line,
                end_col,
                Some(id),
                Some(file),
                start_line,
                start_col,
                kind,
            );
            ids.push(id);
        }
        (parsed, ids)
    }

    /// Adds definitions from a fixture with `«name»`, also recording self-references.
    pub fn add_defs_and_self_refs(
        &mut self,
        annotated_code: &str,
        file: &str,
        path_prefix: &str,
        kind: DefKind,
    ) -> (String, Vec<u32>) {
        let (parsed, ids) =
            self.add_defs_and_self_refs_fixture(annotated_code, file, path_prefix, kind);
        (parsed.clean_text, ids)
    }

    /// Adds references targeting an existing definition from a fixture with `«name»` and optional `$0`.
    pub fn add_refs_to_def_fixture(
        &mut self,
        fixture: &str,
        file: &str,
        def_id: u32,
    ) -> ParsedFixture {
        let def = self
            .defs
            .get(&def_id)
            .unwrap_or_else(|| panic!("FixtureIndexBuilder: def_id {} not found", def_id))
            .clone();
        let parsed = parse_fixture(fixture);
        for span in &parsed.spans {
            self.add_ref(
                &def.path,
                file,
                span.start_line + 1,     // 1-based line
                span.start_col_char + 1, // 1-based char column
                span.end_line + 1,
                span.end_col_char + 1,
                Some(def.def_id),
                Some(&def.file),
                def.start_line,
                def.start_col,
                def.kind,
            );
        }
        parsed
    }

    /// Adds references targeting an existing definition, automatically resolving
    /// its coordinates, target file, and target path from the recorded definition.
    pub fn add_refs_to_def(&mut self, annotated_code: &str, file: &str, def_id: u32) -> String {
        let parsed = self.add_refs_to_def_fixture(annotated_code, file, def_id);
        parsed.clean_text
    }

    /// Adds a raw reference targeting a definition.
    #[allow(clippy::too_many_arguments)]
    pub fn add_ref(
        &mut self,
        target_path: &str,
        file: &str,
        start_line: u32,
        start_col: u32,
        end_line: u32,
        end_col: u32,
        target_def_id: Option<u32>,
        target_file: Option<&str>,
        target_line: u32,
        target_col: u32,
        kind: DefKind,
    ) {
        self.writer.add_reference(RawReference {
            file: file.to_string(),
            start_line,
            start_col,
            end_line,
            end_col,
            target_def_id,
            target_path: target_path.to_string(),
            target_file: target_file.map(|s| s.to_string()),
            target_unit: None,
            target_line,
            target_col,
            kind,
            is_write: false,
            is_macro_expansion: false,
        });
    }

    /// Adds references extracted from source text with `«name»` markers.
    #[allow(clippy::too_many_arguments)]
    pub fn add_refs_from_fixture(
        &mut self,
        annotated_code: &str,
        file: &str,
        target_path: &str,
        target_def_id: Option<u32>,
        target_file: Option<&str>,
        target_line: u32,
        target_col: u32,
        kind: DefKind,
    ) -> String {
        let (clean_code, spans) = parse_spans(annotated_code);
        for span in spans {
            self.add_ref(
                target_path,
                file,
                span.start_line + 1,     // 1-based line
                span.start_col_char + 1, // 1-based char column
                span.end_line + 1,
                span.end_col_char + 1,
                target_def_id,
                target_file,
                target_line,
                target_col,
                kind,
            );
        }
        clean_code
    }

    /// Writes the `.ridx` binary file to disk.
    pub fn write_to_file(mut self, out_path: &Path) {
        if let Some(parent) = out_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let mut file = fs::File::create(out_path).expect("Failed to create fixture file");
        self.writer
            .write_to(&mut file)
            .expect("Failed to write fixture .ridx file");
    }
}

#[cfg(test)]
mod tests;
