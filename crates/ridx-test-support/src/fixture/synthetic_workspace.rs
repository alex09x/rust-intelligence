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
use ridx_format::DefKind;
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// Internal state tracking for a single crate within the synthetic workspace.
struct CrateState {
    crate_name: String,
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
        Ok(Self {
            _temp_dir: temp_dir,
            workspace_root,
            crates: HashMap::new(),
        })
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
        let manifest = format!(
            "[workspace]\nmembers = [\n{}\n]\nresolver = \"2\"\n",
            members_toml.join("\n")
        );
        let manifest_path = self.workspace_root.join("Cargo.toml");
        fs::write(&manifest_path, manifest)?;
        Ok(manifest_path)
    }

    /// Adds a crate to the workspace, creating its directory and `Cargo.toml`.
    pub fn add_crate(&mut self, crate_name: &str, version: &str) -> io::Result<PathBuf> {
        let crate_dir = self.workspace_root.join(crate_name);
        fs::create_dir_all(crate_dir.join("src"))?;

        let cargo_toml = format!(
            "[package]\nname = \"{}\"\nversion = \"{}\"\nedition = \"2021\"\n",
            crate_name, version
        );
        fs::write(crate_dir.join("Cargo.toml"), cargo_toml)?;

        let state = CrateState {
            crate_name: crate_name.to_string(),
            version: version.to_string(),
            crate_dir: crate_dir.clone(),
            builder: FixtureIndexBuilder::new(crate_name, version),
            files: HashMap::new(),
            file_uris: HashMap::new(),
        };
        self.crates.insert(crate_name.to_string(), state);
        Ok(crate_dir)
    }

    /// Adds an unindexed plain source file to a crate.
    pub fn add_source_file(
        &mut self,
        crate_name: &str,
        rel_path: &str,
        content: &str,
    ) -> io::Result<PathBuf> {
        let crate_state = self
            .crates
            .get_mut(crate_name)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("Crate {} not found", crate_name)))?;

        let file_path = crate_state.crate_dir.join(rel_path);
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&file_path, content)?;

        let uri = format!("file://{}", file_path.display());
        crate_state.files.insert(rel_path.to_string(), content.to_string());
        crate_state.file_uris.insert(rel_path.to_string(), uri);
        Ok(file_path)
    }

    /// Adds a source file with `«name»` definition annotations, indexing them via `FixtureIndexBuilder`.
    pub fn add_source_file_with_defs(
        &mut self,
        crate_name: &str,
        rel_path: &str,
        annotated_code: &str,
        kind: DefKind,
    ) -> io::Result<(String, Vec<u32>)> {
        let crate_state = self
            .crates
            .get_mut(crate_name)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("Crate {} not found", crate_name)))?;

        let file_path = crate_state.crate_dir.join(rel_path);
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let abs_str = file_path.to_str().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "Non-UTF8 file path")
        })?;

        let (clean_code, def_ids) = crate_state.builder.add_defs_with_ids(
            annotated_code,
            abs_str,
            &crate_state.crate_name,
            kind,
        );
        fs::write(&file_path, &clean_code)?;

        let uri = format!("file://{}", file_path.display());
        crate_state.files.insert(rel_path.to_string(), clean_code.clone());
        crate_state.file_uris.insert(rel_path.to_string(), uri);

        Ok((clean_code, def_ids))
    }

    /// Adds references in a source file targeting an existing definition ID.
    pub fn add_refs_to_def(
        &mut self,
        crate_name: &str,
        rel_path: &str,
        annotated_code: &str,
        target_def_id: u32,
    ) -> io::Result<String> {
        let crate_state = self
            .crates
            .get_mut(crate_name)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("Crate {} not found", crate_name)))?;

        let file_path = crate_state.crate_dir.join(rel_path);
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let abs_str = file_path.to_str().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "Non-UTF8 file path")
        })?;

        let clean_code = crate_state
            .builder
            .add_refs_to_def(annotated_code, abs_str, target_def_id);
        fs::write(&file_path, &clean_code)?;

        let uri = format!("file://{}", file_path.display());
        crate_state.files.insert(rel_path.to_string(), clean_code.clone());
        crate_state.file_uris.insert(rel_path.to_string(), uri);

        Ok(clean_code)
    }

    /// Adds an inferred type mapping for a cursor range in a crate's file.
    pub fn add_type_at(
        &mut self,
        crate_name: &str,
        rel_path: &str,
        start_line: u32,
        start_col: u32,
        end_line: u32,
        end_col: u32,
        type_name: &str,
    ) -> io::Result<()> {
        let crate_state = self.crates.get_mut(crate_name).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("Crate {} not found", crate_name),
            )
        })?;
        let file_path = crate_state.crate_dir.join(rel_path);
        let abs_str = file_path
            .to_str()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Non-UTF8 file path"))?;
        crate_state
            .builder
            .add_type(abs_str, start_line, start_col, end_line, end_col, type_name);
        Ok(())
    }

    /// Adds references with `DefKind::Impl` targeting an existing trait or struct definition.
    pub fn add_source_file_with_impl_refs(
        &mut self,
        crate_name: &str,
        rel_path: &str,
        annotated_code: &str,
        target_crate: &str,
        target_def_id: u32,
    ) -> io::Result<String> {
        let target_def = {
            let target_state = self.crates.get(target_crate).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("Target crate {} not found", target_crate),
                )
            })?;
            target_state
                .builder
                .get_def(target_def_id)
                .cloned()
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::NotFound,
                        format!("Def {} not found in crate {}", target_def_id, target_crate),
                    )
                })?
        };

        let crate_state = self.crates.get_mut(crate_name).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("Crate {} not found", crate_name),
            )
        })?;

        let file_path = crate_state.crate_dir.join(rel_path);
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let abs_str = file_path
            .to_str()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Non-UTF8 file path"))?;

        let (clean_code, spans) = parse_spans(annotated_code);
        for span in spans {
            crate_state.builder.add_ref(
                &target_def.path,
                abs_str,
                span.start_line + 1,
                span.start_col_char + 1,
                span.end_line + 1,
                span.end_col_char + 1,
                Some(target_def_id),
                Some(&target_def.file),
                target_def.start_line,
                target_def.start_col,
                DefKind::Impl,
            );
        }
        fs::write(&file_path, &clean_code)?;

        let uri = format!("file://{}", file_path.display());
        crate_state
            .files
            .insert(rel_path.to_string(), clean_code.clone());
        crate_state.file_uris.insert(rel_path.to_string(), uri);

        Ok(clean_code)
    }

    /// Writes the `.ridx` index shard for a specific crate to `target/ridx/<crate_name>.ridx`.
    /// State is preserved so subsequent calls reflect the full accumulated definitions.
    pub fn build_crate_index(&self, crate_name: &str) -> io::Result<PathBuf> {
        let crate_state = self
            .crates
            .get(crate_name)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("Crate {} not found", crate_name)))?;

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
        self.crates
            .get(crate_name)
            .and_then(|c| c.file_uris.get(rel_path).cloned())
    }

    /// Retrieves the clean text content of a file in a given crate.
    pub fn file_content(&self, crate_name: &str, rel_path: &str) -> Option<&str> {
        self.crates
            .get(crate_name)
            .and_then(|c| c.files.get(rel_path).map(|s| s.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ridx_format::IndexReader;

    #[test]
    fn test_synthetic_workspace_coordinates() {
        let mut ws = SyntheticWorkspaceFixture::new().unwrap();
        ws.add_crate("core_pkg", "0.1.0").unwrap();

        // "/* 🦀 */ pub fn «grüßen»() {}"
        // "/* " (3) + "🦀" (1) + " */ pub fn " (11) = 15 chars before marker.
        // 1-based column for driver = 15 + 1 = 16.
        // "grüßen" has 6 chars -> end col = 16 + 6 = 22.
        let code = "/* 🦀 */ pub fn «grüßen»() {}\n";
        let (clean, def_ids) = ws
            .add_source_file_with_defs("core_pkg", "src/lib.rs", code, DefKind::Fn)
            .unwrap();

        assert_eq!(clean, "/* 🦀 */ pub fn grüßen() {}\n");
        assert_eq!(def_ids.len(), 1);

        assert_eq!(ws.crate_version("core_pkg"), Some("0.1.0"));

        let ridx_path = ws.build_crate_index("core_pkg").unwrap();
        assert!(ridx_path.is_file());

        let reader = IndexReader::open(&ridx_path).unwrap();
        let defs = reader.definitions();
        assert_eq!(defs.len(), 1);

        let def = &defs[0];
        let name = reader.get_str(def.name_offset);
        assert_eq!(name, "grüßen");
        assert_eq!(def.span.start_line(), 1);
        assert_eq!(def.span.start_col(), 16);
        assert_eq!(def.span.end_col(), 22);

        // Verify state is preserved across repeated builds
        let ridx_path2 = ws.build_crate_index("core_pkg").unwrap();
        let reader2 = IndexReader::open(&ridx_path2).unwrap();
        assert_eq!(reader2.definitions().len(), 1);

        // Verify sorted build_all_indices and write_workspace_manifest
        ws.add_crate("extra_pkg", "0.2.0").unwrap();
        let manifest = ws.write_workspace_manifest().unwrap();
        assert!(manifest.is_file());
        let manifest_text = fs::read_to_string(&manifest).unwrap();
        assert!(manifest_text.contains("\"core_pkg\""));
        assert!(manifest_text.contains("\"extra_pkg\""));

        let all_shards = ws.build_all_indices().unwrap();
        assert_eq!(all_shards.len(), 2);
    }
}
