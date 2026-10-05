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
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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
