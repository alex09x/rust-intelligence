/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Synthetic multi-crate workspace fixture for hermetic integration tests.

use crate::cursor_dsl::parse_spans;
use crate::fixture::FixtureIndexBuilder;
use ridx_format::{DefKind, RawDefinition};
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// Internal state tracking for a single crate within the synthetic workspace.
struct CrateState {
    version: String,
    crate_dir: PathBuf,
    builder: FixtureIndexBuilder,
    files: HashMap<String, String>,
    file_uris: HashMap<String, String>,
}

/// Hermetic multi-crate workspace fixture managing file layouts, `.ridx` index shards,
/// and cross-crate references in a self-cleaning temporary directory.
pub struct SyntheticWorkspaceFixture {
    _temp_dir: TempDir,
    workspace_root: PathBuf,
    crates: HashMap<String, CrateState>,
}

impl SyntheticWorkspaceFixture {
    /// Creates a new hermetic synthetic workspace in a fresh temporary directory.
    pub fn new() -> io::Result<Self> {
        let temp_dir = TempDir::new()?;
        let workspace_root = temp_dir.path().canonicalize().unwrap_or_else(|_| temp_dir.path().to_path_buf());
        Ok(Self { _temp_dir: temp_dir, workspace_root, crates: HashMap::new() })
    }

    /// Absolute path to the workspace root directory.
    pub fn root_path(&self) -> &Path {
        &self.workspace_root
    }

    /// LSP URI for the workspace root directory.
    pub fn root_uri(&self) -> String {
        format!("file://{}", self.workspace_root.display())
    }

    /// Writes the root `Cargo.toml` with `[workspace] members = [...]` sorted alphabetically.
    pub fn write_workspace_manifest(&self) -> io::Result<PathBuf> {
        let mut members: Vec<String> = self.crates.keys().cloned().collect();
        members.sort();
        let members_toml: Vec<String> = members.iter().map(|m| format!("    \"{}\",", m)).collect();
        let manifest = format!("[workspace]\nmembers = [\n{}\n]\nresolver = \"2\"\n", members_toml.join("\n"));
        let manifest_path = self.workspace_root.join("Cargo.toml");
        fs::write(&manifest_path, manifest)?;
        Ok(manifest_path)
    }

    /// Adds a crate to the workspace, creating its directory and `Cargo.toml`.
    pub fn add_crate(&mut self, crate_name: &str, version: &str) -> io::Result<PathBuf> {
        let crate_dir = self.workspace_root.join(crate_name);
        fs::create_dir_all(crate_dir.join("src"))?;

        let cargo_toml = format!("[package]\nname = \"{}\"\nversion = \"{}\"\nedition = \"2021\"\n", crate_name, version);
        fs::write(crate_dir.join("Cargo.toml"), cargo_toml)?;

        let state = CrateState {
            version: version.to_string(),
            crate_dir: crate_dir.clone(),
            builder: FixtureIndexBuilder::new(crate_name, version),
            files: HashMap::new(),
            file_uris: HashMap::new(),
        };
        self.crates.insert(crate_name.to_string(), state);
        Ok(crate_dir)
    }

    fn prepare_source_file(&self, crate_name: &str, rel_path: &str) -> io::Result<(PathBuf, String)> {
        let crate_state = self.crates.get(crate_name).ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("Crate {} not found", crate_name)))?;
        let file_path = crate_state.crate_dir.join(rel_path);
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let abs_str = file_path.to_str().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Non-UTF8 file path"))?.to_string();
        Ok((file_path, abs_str))
    }

    fn record_source_file(&mut self, crate_name: &str, rel_path: &str, file_path: &Path, content: &str) {
        if let Some(crate_state) = self.crates.get_mut(crate_name) {
            let uri = format!("file://{}", file_path.display());
            crate_state.files.insert(rel_path.to_string(), content.to_string());
            crate_state.file_uris.insert(rel_path.to_string(), uri);
        }
    }

    /// Adds an unindexed plain source file to a crate.
    pub fn add_source_file(&mut self, crate_name: &str, rel_path: &str, content: &str) -> io::Result<PathBuf> {
        let (file_path, _) = self.prepare_source_file(crate_name, rel_path)?;
        fs::write(&file_path, content)?;
        self.record_source_file(crate_name, rel_path, &file_path, content);
        Ok(file_path)
    }

    /// Adds a source file with `«name»` definition annotations and a custom path prefix.
    pub fn add_source_file_with_prefix_defs(&mut self, crate_name: &str, rel_path: &str, annotated_code: &str, path_prefix: &str, kind: DefKind) -> io::Result<(String, Vec<u32>)> {
        let (file_path, abs_str) = self.prepare_source_file(crate_name, rel_path)?;
        let crate_state = self.crates.get_mut(crate_name).expect("crate exists");
        let (clean_code, def_ids) = crate_state.builder.add_defs_with_ids(annotated_code, &abs_str, path_prefix, kind);
        fs::write(&file_path, &clean_code)?;
        self.record_source_file(crate_name, rel_path, &file_path, &clean_code);
        Ok((clean_code, def_ids))
    }

    /// Adds a source file with `«name»` definition annotations, indexing them via `FixtureIndexBuilder`.
    pub fn add_source_file_with_defs(&mut self, crate_name: &str, rel_path: &str, annotated_code: &str, kind: DefKind) -> io::Result<(String, Vec<u32>)> {
        self.add_source_file_with_prefix_defs(crate_name, rel_path, annotated_code, crate_name, kind)
    }

    /// Adds references in a source file targeting an existing definition ID.
    pub fn add_refs_to_def(&mut self, crate_name: &str, rel_path: &str, annotated_code: &str, target_def_id: u32) -> io::Result<String> {
        let (file_path, abs_str) = self.prepare_source_file(crate_name, rel_path)?;
        let crate_state = self.crates.get_mut(crate_name).expect("crate exists");
        let clean_code = crate_state.builder.add_refs_to_def(annotated_code, &abs_str, target_def_id);
        fs::write(&file_path, &clean_code)?;
        self.record_source_file(crate_name, rel_path, &file_path, &clean_code);
        Ok(clean_code)
    }

    /// Adds an inferred type mapping for a cursor range in a crate's file.
    #[allow(clippy::too_many_arguments)]
    pub fn add_type_at(&mut self, crate_name: &str, rel_path: &str, start_line: u32, start_col: u32, end_line: u32, end_col: u32, type_name: &str) -> io::Result<()> {
        let (_, abs_str) = self.prepare_source_file(crate_name, rel_path)?;
        let crate_state = self.crates.get_mut(crate_name).expect("crate exists");
        crate_state.builder.add_type(&abs_str, start_line, start_col, end_line, end_col, type_name);
        Ok(())
    }

    fn get_foreign_def(&self, target_crate: &str, target_def_id: u32) -> io::Result<RawDefinition> {
        let target_state = self.crates.get(target_crate).ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("Target crate {} not found", target_crate)))?;
        target_state
            .builder
            .get_def(target_def_id)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("Def {} not found in crate {}", target_def_id, target_crate)))
    }

    /// Adds references with `DefKind::Impl` targeting an existing trait or struct definition.
    pub fn add_source_file_with_impl_refs(&mut self, crate_name: &str, rel_path: &str, annotated_code: &str, target_crate: &str, target_def_id: u32) -> io::Result<String> {
        let target_def = self.get_foreign_def(target_crate, target_def_id)?;
        self.add_foreign_refs_internal(
            crate_name,
            rel_path,
            annotated_code,
            target_def_id,
            &target_def.path,
            &target_def.file,
            target_def.start_line,
            target_def.start_col,
            DefKind::Impl,
        )
    }

    /// Adds references in a source file targeting an existing definition in another crate.
    pub fn add_refs_to_foreign_def(&mut self, crate_name: &str, rel_path: &str, annotated_code: &str, target_crate: &str, target_def_id: u32) -> io::Result<String> {
        let target_def = self.get_foreign_def(target_crate, target_def_id)?;
        self.add_foreign_refs_internal(
            crate_name,
            rel_path,
            annotated_code,
            target_def_id,
            &target_def.path,
            &target_def.file,
            target_def.start_line,
            target_def.start_col,
            target_def.kind,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn add_foreign_refs_internal(
        &mut self,
        crate_name: &str,
        rel_path: &str,
        annotated_code: &str,
        target_def_id: u32,
        target_path: &str,
        target_file: &str,
        target_line: u32,
        target_col: u32,
        kind: DefKind,
    ) -> io::Result<String> {
        let (file_path, abs_str) = self.prepare_source_file(crate_name, rel_path)?;
        let (clean_code, spans) = parse_spans(annotated_code);
        let crate_state = self.crates.get_mut(crate_name).expect("crate exists");
        for span in spans {
            crate_state.builder.add_ref(
                target_path,
                &abs_str,
                span.start_line + 1,
                span.start_col_char + 1,
                span.end_line + 1,
                span.end_col_char + 1,
                Some(target_def_id),
                Some(target_file),
                target_line,
                target_col,
                kind,
            );
        }
        fs::write(&file_path, &clean_code)?;
        self.record_source_file(crate_name, rel_path, &file_path, &clean_code);
        Ok(clean_code)
    }

    /// Writes the `.ridx` index shard for a specific crate to `target/ridx/<crate_name>.ridx`.
    /// State is preserved so subsequent calls reflect the full accumulated definitions.
    pub fn build_crate_index(&self, crate_name: &str) -> io::Result<PathBuf> {
        let crate_state = self.crates.get(crate_name).ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("Crate {} not found", crate_name)))?;

        let target_dir = self.workspace_root.join("target/ridx");
        fs::create_dir_all(&target_dir)?;
        let ridx_path = target_dir.join(format!("{}.ridx", crate_name));

        crate_state.builder.write_to_file(&ridx_path);
        Ok(ridx_path)
    }

    /// Builds `.ridx` index shards for all registered crates in the workspace in sorted order.
    pub fn build_all_indices(&self) -> io::Result<Vec<PathBuf>> {
        let mut crate_names: Vec<String> = self.crates.keys().cloned().collect();
        crate_names.sort();
        let mut paths = Vec::new();
        for name in crate_names {
            paths.push(self.build_crate_index(&name)?);
        }
        Ok(paths)
    }

    /// Retrieves the crate version string.
    pub fn crate_version(&self, crate_name: &str) -> Option<&str> {
        self.crates.get(crate_name).map(|c| c.version.as_str())
    }

    /// Retrieves the absolute path to a file in a given crate.
    pub fn file_path(&self, crate_name: &str, rel_path: &str) -> Option<PathBuf> {
        self.crates.get(crate_name).map(|c| c.crate_dir.join(rel_path))
    }

    /// Retrieves the LSP URI for a file in a given crate.
    pub fn file_uri(&self, crate_name: &str, rel_path: &str) -> Option<String> {
        self.crates.get(crate_name).and_then(|c| c.file_uris.get(rel_path).cloned())
    }

    /// Retrieves the clean text content of a file in a given crate.
    pub fn file_content(&self, crate_name: &str, rel_path: &str) -> Option<&str> {
        self.crates.get(crate_name).and_then(|c| c.files.get(rel_path).map(|s| s.as_str()))
    }
}
