/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Manifest, symbols, statistics and configuration types for multi-crate workspace index.

use crate::format::{join_resolving_overlap, DefKind, Span, CURRENT_FORMAT_VERSION};
use crate::reader::ResolvedDefinition;
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

/// A symbol match within a workspace, including its crate provenance.
#[derive(Debug, Clone)]
pub struct WorkspaceSymbol<'a> {
    pub crate_name: &'a str,
    pub name: &'a str,
    pub path: &'a str,
    pub kind: DefKind,
    pub file: &'a str,
    pub span: Span,
    pub signature: Option<&'a str>,
    pub is_macro_expansion: bool,
    pub source_root: Option<&'a Path>,
    pub unit_key: Option<&'a str>,
}

impl<'a> WorkspaceSymbol<'a> {
    pub fn resolved_path(&self) -> PathBuf {
        let p = Path::new(self.file);
        if p.is_absolute() {
            p.to_path_buf()
        } else if let Some(root) = self.source_root {
            join_resolving_overlap(root, p)
        } else {
            p.to_path_buf()
        }
    }
}

/// A reference match within a workspace, including its crate provenance.
#[derive(Debug, Clone)]
pub struct WorkspaceReference<'a> {
    pub crate_name: &'a str,
    pub file: &'a str,
    pub span: Span,
    pub kind: DefKind,
    pub is_write: bool,
    pub is_macro_expansion: bool,
    pub source_root: Option<&'a Path>,
    pub unit_key: Option<&'a str>,
}

impl<'a> WorkspaceReference<'a> {
    pub fn resolved_path(&self) -> PathBuf {
        let p = Path::new(self.file);
        if p.is_absolute() {
            p.to_path_buf()
        } else if let Some(root) = self.source_root {
            join_resolving_overlap(root, p)
        } else {
            p.to_path_buf()
        }
    }
}

/// Result of finding references at a cursor location.
#[derive(Debug, Clone)]
pub struct LocationReferences<'a> {
    pub target_symbol: &'a str,
    pub target_kind: DefKind,
    pub target_path: &'a str,
    pub target_file: &'a str,
    pub target_line: u32,
    pub target_col: u32,
    pub target_end_line: u32,
    pub target_end_col: u32,
    pub target_source_root: Option<&'a Path>,
    pub target_unit_key: Option<&'a str>,
    pub references: Vec<WorkspaceReference<'a>>,
}

impl<'a> LocationReferences<'a> {
    pub fn target_resolved_path(&self) -> PathBuf {
        let p = Path::new(self.target_file);
        if p.is_absolute() {
            p.to_path_buf()
        } else if let Some(root) = self.target_source_root {
            join_resolving_overlap(root, p)
        } else {
            p.to_path_buf()
        }
    }
}

/// Detailed hover information for a source location.
#[derive(Debug, Clone)]
pub struct WorkspaceHover<'a> {
    pub definition: Option<ResolvedDefinition<'a>>,
    pub inferred_type: Option<&'a str>,
}

/// Workspace-level index statistics.
#[derive(Debug, Clone, Default)]
pub struct WorkspaceStats {
    pub total_crates: usize,
    pub total_defs: usize,
    pub total_refs: usize,
    pub total_types: usize,
    pub total_files: usize,
    pub total_bytes: u64,
    pub crate_summaries: Vec<CrateSummary>,
    pub active_variant: Option<String>,
}

/// Metadata describing an immutable external dependency shard in the global cache.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct DependencyShardInfo {
    pub name: String,
    #[serde(default)]
    pub version: String,
    pub unit_key: String,
    #[serde(default)]
    pub object_path: String,
    #[serde(default)]
    pub source_root: Option<String>,
    #[serde(default)]
    pub is_dependency: bool,
    #[serde(default)]
    pub generation: Option<String>,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub extern_units: HashMap<String, String>,
    #[serde(default)]
    pub file_hashes: HashMap<String, u64>,
    #[serde(default)]
    pub stable_crate_id: Option<String>,
    #[serde(default)]
    pub caught_panics: Option<usize>,
}

pub(crate) fn default_format_version() -> u32 {
    CURRENT_FORMAT_VERSION
}

/// Helper to identify if an I/O error was caused by a RIDX index format version mismatch.
pub fn is_version_mismatch(err: &io::Error) -> bool {
    if err.kind() == io::ErrorKind::InvalidData {
        let msg = err.to_string();
        msg.contains("Unsupported RIDX version")
            || msg.contains("Format version mismatch")
            || msg.contains("Extractor version mismatch")
            || msg.contains("Extractor version missing")
    } else {
        false
    }
}

/// Active variant metadata stored in `target/ridx/ridx_manifest.json`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VariantManifest {
    #[serde(default = "default_manifest_version")]
    pub version: String,
    #[serde(default = "default_format_version")]
    pub format_version: u32,
    #[serde(default)]
    pub generation: u64,
    #[serde(default)]
    pub extractor_version: Option<String>,
    #[serde(default)]
    pub active_variant_id: String,
    #[serde(default)]
    pub variant_dir: String,
    #[serde(default)]
    pub workspace_root: String,
    #[serde(default)]
    pub manifest_path: String,
    #[serde(default)]
    pub base_out_dir: Option<String>,
    #[serde(default)]
    pub profile: String,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub features: Option<String>,
    #[serde(default)]
    pub no_default_features: bool,
    #[serde(default)]
    pub all_features: bool,
    #[serde(default)]
    pub package_filter: Option<String>,
    #[serde(default)]
    pub timestamp_epoch_secs: u64,
    #[serde(default)]
    pub total_crates: usize,
    #[serde(default)]
    pub total_defs: usize,
    #[serde(default)]
    pub total_refs: usize,
    #[serde(default)]
    pub total_types: usize,
    #[serde(default)]
    pub dependency_shards: Vec<DependencyShardInfo>,
}

pub(crate) fn default_manifest_version() -> String {
    "2.0".to_string()
}

/// Individual crate summary inside a workspace.
#[derive(Debug, Clone)]
pub struct CrateSummary {
    pub name: String,
    pub version: String,
    pub path: PathBuf,
    pub file_size: u64,
    pub defs_count: u32,
    pub refs_count: u32,
    pub types_count: u32,
    pub files_count: u32,
    pub is_dependency: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::{DefKind, Span};

    #[test]
    fn test_manifest_resolved_paths() {
        let abs_file = "/path/to/abs/file.rs";
        let rel_file = "src/lib.rs";
        let root = Path::new("/workspace/root");

        let sym_abs = WorkspaceSymbol {
            crate_name: "foo",
            name: "bar",
            path: "foo::bar",
            kind: DefKind::Fn,
            file: abs_file,
            span: Span::new(0, 1, 1, 2, 1),
            signature: None,
            is_macro_expansion: false,
            source_root: Some(root),
            unit_key: None,
        };
        assert_eq!(sym_abs.resolved_path(), PathBuf::from(abs_file));

        let sym_rel = WorkspaceSymbol {
            crate_name: "foo",
            name: "bar",
            path: "foo::bar",
            kind: DefKind::Fn,
            file: rel_file,
            span: Span::new(0, 1, 1, 2, 1),
            signature: None,
            is_macro_expansion: false,
            source_root: Some(root),
            unit_key: None,
        };
        assert_eq!(sym_rel.resolved_path(), root.join(rel_file));

        let sym_no_root = WorkspaceSymbol {
            source_root: None,
            ..sym_rel
        };
        assert_eq!(sym_no_root.resolved_path(), PathBuf::from(rel_file));

        let r_abs = WorkspaceReference {
            crate_name: "foo",
            file: abs_file,
            span: Span::new(0, 1, 1, 2, 1),
            kind: DefKind::Fn,
            is_write: false,
            is_macro_expansion: false,
            source_root: Some(root),
            unit_key: None,
        };
        assert_eq!(r_abs.resolved_path(), PathBuf::from(abs_file));

        let r_rel = WorkspaceReference {
            file: rel_file,
            source_root: Some(root),
            ..r_abs
        };
        assert_eq!(r_rel.resolved_path(), root.join(rel_file));

        let r_no_root = WorkspaceReference {
            source_root: None,
            ..r_rel
        };
        assert_eq!(r_no_root.resolved_path(), PathBuf::from(rel_file));

        let loc_refs = LocationReferences {
            target_symbol: "bar",
            target_kind: DefKind::Fn,
            target_path: "foo::bar",
            target_file: rel_file,
            target_line: 1,
            target_col: 1,
            target_end_line: 2,
            target_end_col: 1,
            target_source_root: Some(root),
            target_unit_key: None,
            references: vec![r_abs],
        };
        assert_eq!(loc_refs.target_resolved_path(), root.join(rel_file));

        let loc_refs_abs = LocationReferences {
            target_file: abs_file,
            target_source_root: None,
            ..loc_refs
        };
        assert_eq!(loc_refs_abs.target_resolved_path(), PathBuf::from(abs_file));
    }

    #[test]
    fn test_version_helpers_and_mismatch() {
        assert_eq!(default_format_version(), CURRENT_FORMAT_VERSION);
        assert_eq!(default_manifest_version(), "2.0");

        let err_unsupported =
            io::Error::new(io::ErrorKind::InvalidData, "Unsupported RIDX version 99");
        assert!(is_version_mismatch(&err_unsupported));

        let err_format =
            io::Error::new(io::ErrorKind::InvalidData, "Format version mismatch: got 1");
        assert!(is_version_mismatch(&err_format));

        let err_extractor =
            io::Error::new(io::ErrorKind::InvalidData, "Extractor version mismatch");
        assert!(is_version_mismatch(&err_extractor));

        let err_missing = io::Error::new(io::ErrorKind::InvalidData, "Extractor version missing");
        assert!(is_version_mismatch(&err_missing));

        let err_other_data = io::Error::new(io::ErrorKind::InvalidData, "Corrupted index");
        assert!(!is_version_mismatch(&err_other_data));

        let err_not_found = io::Error::new(io::ErrorKind::NotFound, "File not found");
        assert!(!is_version_mismatch(&err_not_found));
    }

    #[test]
    fn test_variant_manifest_serialization() {
        let json = r#"{"active_variant_id": "test_var", "workspace_root": "/test"}"#;
        let manifest: VariantManifest = serde_json::from_str(json).unwrap();
        assert_eq!(manifest.version, "2.0");
        assert_eq!(manifest.format_version, CURRENT_FORMAT_VERSION);
        assert_eq!(manifest.active_variant_id, "test_var");
        assert_eq!(manifest.workspace_root, "/test");
    }
}
