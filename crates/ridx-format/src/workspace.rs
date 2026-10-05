//! Multi-crate workspace index support for Rust Intelligence.
//!
//! Provides unified zero-copy queries across all crates in a cargo workspace.

use crate::format::{DefKind, Span};
use crate::reader::{IndexReader, ResolvedDefinition};
use memmap2::Mmap;
use std::fs;
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
    pub references: Vec<WorkspaceReference<'a>>,
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

/// Active variant metadata stored in `target/ridx/ridx_manifest.json`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VariantManifest {
    #[serde(default = "default_manifest_version")]
    pub version: String,
    #[serde(default)]
    pub active_variant_id: String,
    #[serde(default)]
    pub variant_dir: String,
    #[serde(default)]
    pub workspace_root: String,
    #[serde(default)]
    pub manifest_path: String,
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
}

fn default_manifest_version() -> String {
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
}

/// Unified multi-crate index for a cargo workspace or multi-index directory.
pub struct WorkspaceIndex {
    readers: Vec<IndexReader<Mmap>>,
    paths: Vec<PathBuf>,
    mtimes: Vec<Option<std::time::SystemTime>>,
    root_dir: Option<PathBuf>,
    manifest_path: Option<PathBuf>,
    manifest_mtime: Option<std::time::SystemTime>,
    active_variant_id: Option<String>,
}

impl WorkspaceIndex {
    /// Opens a single .ridx file or scans a directory containing .ridx files.
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let p = path.as_ref();

        if p.is_file() {
            let mtime = fs::metadata(p).and_then(|m| m.modified()).ok();
            let reader = IndexReader::open(p)?;
            return Ok(Self {
                readers: vec![reader],
                paths: vec![p.to_path_buf()],
                mtimes: vec![mtime],
                root_dir: None,
                manifest_path: None,
                manifest_mtime: None,
                active_variant_id: None,
            });
        }

        // If directory, check target/ridx subfolder first if it exists
        let search_dir = if p.is_dir() && p.join("target/ridx").is_dir() {
            p.join("target/ridx")
        } else {
            p.to_path_buf()
        };

        // Check if an active build variant manifest exists
        let manifest_file = search_dir.join("ridx_manifest.json");
        let (scan_dir, manifest_info) = if manifest_file.is_file() {
            if let Ok(content) = fs::read(&manifest_file) {
                if let Ok(manifest) = serde_json::from_slice::<VariantManifest>(&content) {
                    let vdir = search_dir.join(&manifest.variant_dir);
                    let mtime = fs::metadata(&manifest_file).and_then(|m| m.modified()).ok();
                    if vdir.is_dir() {
                        (vdir, Some((manifest_file, mtime, manifest.active_variant_id)))
                    } else {
                        (search_dir.clone(), Some((manifest_file, mtime, manifest.active_variant_id)))
                    }
                } else {
                    (search_dir.clone(), None)
                }
            } else {
                (search_dir.clone(), None)
            }
        } else {
            (search_dir.clone(), None)
        };

        let mut ridx_files = Vec::new();
        if scan_dir.is_dir() {
            collect_ridx_files(&scan_dir, &mut ridx_files)?;
        }

        if ridx_files.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("No .ridx index files found in {:?}", p),
            ));
        }

        // Sort files for deterministic order
        ridx_files.sort();

        let mut readers = Vec::with_capacity(ridx_files.len());
        let mut paths = Vec::with_capacity(ridx_files.len());
        let mut mtimes = Vec::with_capacity(ridx_files.len());

        for file_path in ridx_files {
            match IndexReader::open(&file_path) {
                Ok(reader) => {
                    let mtime = fs::metadata(&file_path).and_then(|m| m.modified()).ok();
                    readers.push(reader);
                    paths.push(file_path);
                    mtimes.push(mtime);
                }
                Err(e) => {
                    eprintln!("Warning: skipping invalid index {:?}: {e}", file_path);
                }
            }
        }

        if readers.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Failed to open any valid .ridx index files in {:?}", p),
            ));
        }

        let (manifest_path, manifest_mtime, active_variant_id) = match manifest_info {
            Some((mp, mm, avid)) => (Some(mp), mm, Some(avid)),
            None => (None, None, None),
        };

        Ok(Self {
            readers,
            paths,
            mtimes,
            root_dir: Some(scan_dir),
            manifest_path,
            manifest_mtime,
            active_variant_id,
        })
    }

    /// Auto-discovers .ridx indices starting from current directory up to root or target/ridx.
    pub fn auto_discover() -> io::Result<Self> {
        if let Ok(dir) = std::env::var("RIDX_INDEX_DIR") {
            let p = PathBuf::from(dir);
            if p.exists() {
                return Self::open(p);
            }
        }

        let cur = std::env::current_dir()?;
        let candidates = [
            cur.join("target/ridx"),
            cur.clone(),
            cur.join("../target/ridx"),
        ];

        for cand in candidates {
            if cand.is_dir() {
                if let Ok(ws) = Self::open(&cand) {
                    return Ok(ws);
                }
            }
        }

        Err(io::Error::new(
            io::ErrorKind::NotFound,
            "Could not auto-discover target/ridx directory. Specify index path explicitly.",
        ))
    }

    /// Number of crates loaded in this workspace index.
    pub fn crate_count(&self) -> usize {
        self.readers.len()
    }

    /// Slice of crate index readers.
    pub fn readers(&self) -> &[IndexReader<Mmap>] {
        &self.readers
    }

    /// Paths to the loaded .ridx index files.
    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    /// Aggregated statistics across all crates in the workspace.
    pub fn stats(&self) -> WorkspaceStats {
        let mut stats = WorkspaceStats {
            total_crates: self.readers.len(),
            active_variant: self.active_variant_id.clone(),
            ..Default::default()
        };

        for (i, reader) in self.readers.iter().enumerate() {
            let h = reader.header();
            let size = fs::metadata(&self.paths[i]).map(|m| m.len()).unwrap_or(0);

            stats.total_defs += h.defs_count as usize;
            stats.total_refs += h.refs_count as usize;
            stats.total_types += h.types_count as usize;
            stats.total_files += h.files_count as usize;
            stats.total_bytes += size;

            stats.crate_summaries.push(CrateSummary {
                name: reader.crate_name().to_string(),
                version: reader.crate_version().to_string(),
                path: self.paths[i].clone(),
                file_size: size,
                defs_count: h.defs_count,
                refs_count: h.refs_count,
                types_count: h.types_count,
                files_count: h.files_count,
            });
        }

        stats
    }

    /// Currently active build variant ID, if loaded from a variant manifest.
    pub fn active_variant_id(&self) -> Option<&str> {
        self.active_variant_id.as_deref()
    }

    /// Finds a definition by full or relative canonical path across the workspace.
    pub fn find_definition_by_path<'a>(&'a self, path: &str) -> Option<ResolvedDefinition<'a>> {
        // Fast path: if path starts with `<crate_name>::`, check that crate first
        for reader in &self.readers {
            let prefix = format!("{}::", reader.crate_name());
            if path.starts_with(&prefix) {
                if let Some(def) = reader.find_definition_by_path(path) {
                    let span = def.span();
                    return Some(ResolvedDefinition {
                        name: reader.get_str(def.name_offset()),
                        path: reader.get_str(def.path_offset()),
                        kind: DefKind::from(def.kind),
                        signature: if def.sig_offset() != 0 {
                            Some(reader.get_str(def.sig_offset()))
                        } else {
                            None
                        },
                        file: reader.get_file(span.file_id()),
                        start_line: span.start_line(),
                        start_col: span.start_col(),
                        end_line: span.end_line(),
                        end_col: span.end_col(),
                        is_macro_expansion: def.is_macro_expansion(),
                    });
                }
            }
        }

        // General search across all crates
        for reader in &self.readers {
            if let Some(def) = reader.find_definition_by_path(path) {
                let span = def.span();
                return Some(ResolvedDefinition {
                    name: reader.get_str(def.name_offset()),
                    path: reader.get_str(def.path_offset()),
                    kind: DefKind::from(def.kind),
                    signature: if def.sig_offset() != 0 {
                        Some(reader.get_str(def.sig_offset()))
                    } else {
                        None
                    },
                    file: reader.get_file(span.file_id()),
                    start_line: span.start_line(),
                    start_col: span.start_col(),
                    end_line: span.end_line(),
                    end_col: span.end_col(),
                    is_macro_expansion: def.is_macro_expansion(),
                });
            }
        }

        None
    }

    /// Finds a definition at a cursor location, seamlessly resolving cross-crate references.
    pub fn find_definition_at<'a>(
        &'a self,
        file: &str,
        line: u32,
        col: u32,
    ) -> Option<ResolvedDefinition<'a>> {
        for reader in &self.readers {
            if let Some(res) = reader.find_definition_at(file, line, col) {
                // If it resolves to a cross-crate target, attempt resolution in
                // workspace peer crates to retrieve the true indexed definition and signature
                if let Some(cross_def) = self.find_definition_by_path(res.path) {
                    return Some(cross_def);
                }
                return Some(res);
            }
        }

        None
    }

    /// Finds all references to a symbol across every crate in the workspace.
    pub fn find_references<'a>(&'a self, target_path: &str) -> Vec<WorkspaceReference<'a>> {
        let mut all_refs = Vec::new();

        for reader in &self.readers {
            let refs = reader.find_references(target_path);
            let crate_name = reader.crate_name();

            for r in refs {
                let span = r.span();
                let file = reader.get_file(span.file_id());
                all_refs.push(WorkspaceReference {
                    crate_name,
                    file,
                    span,
                    kind: DefKind::from(r.kind),
                    is_write: r.is_write != 0,
                    is_macro_expansion: r.is_macro_expansion(),
                });
            }
        }

        all_refs
    }

    /// Inferred expression type at cursor location.
    pub fn type_at(&self, file: &str, line: u32, col: u32) -> Option<&str> {
        for reader in &self.readers {
            if let Some(ty) = reader.type_at(file, line, col) {
                return Some(ty);
            }
        }
        None
    }

    /// Substring symbol search across all definitions in the workspace.
    pub fn search_symbols<'a>(&'a self, query: &str) -> Vec<WorkspaceSymbol<'a>> {
        let mut matches = Vec::new();

        for reader in &self.readers {
            let crate_name = reader.crate_name();
            let defs = reader.search_symbols(query);

            for def in defs {
                let name = reader.get_str(def.name_offset());
                let path = reader.get_str(def.path_offset());
                let span = def.span();
                let file = reader.get_file(span.file_id());
                let signature = if def.sig_offset() != 0 {
                    Some(reader.get_str(def.sig_offset()))
                } else {
                    None
                };

                matches.push(WorkspaceSymbol {
                    crate_name,
                    name,
                    path,
                    kind: DefKind::from(def.kind),
                    file,
                    span,
                    signature,
                    is_macro_expansion: def.is_macro_expansion(),
                });
            }
        }

        matches
    }

    /// Finds all references to the symbol at cursor location across the workspace.
    pub fn find_references_at<'a>(
        &'a self,
        file: &str,
        line: u32,
        col: u32,
    ) -> Option<LocationReferences<'a>> {
        let def = self.find_definition_at(file, line, col)?;
        let mut refs = self.find_references(def.path);
        // For local bindings whose references are stored by local name, try by short name if needed
        if refs.is_empty() && def.kind == DefKind::Local && def.name != def.path {
            refs = self.find_references(def.name);
        }
        Some(LocationReferences {
            target_symbol: def.name,
            target_kind: def.kind,
            target_path: def.path,
            target_file: def.file,
            target_line: def.start_line,
            target_col: def.start_col,
            target_end_line: def.end_line,
            target_end_col: def.end_col,
            references: refs,
        })
    }

    /// Complete hover information (definition + signature + inferred type) at cursor location.
    pub fn hover_at<'a>(&'a self, file: &str, line: u32, col: u32) -> Option<WorkspaceHover<'a>> {
        let def = self.find_definition_at(file, line, col);
        let ty = self.type_at(file, line, col);
        if def.is_none() && ty.is_none() {
            return None;
        }
        Some(WorkspaceHover {
            definition: def,
            inferred_type: ty,
        })
    }

    /// Checks timestamps of indexed files and reloads any that have changed.
    /// Also scans directory for newly generated .ridx files and removes deleted ones.
    /// If an active variant manifest exists and was switched, atomically reloads the new variant's reader set.
    /// Returns true if any index was updated, added, removed, or switched.
    pub fn reload_if_modified(&mut self) -> io::Result<bool> {
        let mut changed = false;

        // 0. Check if active variant manifest was updated or switched
        if self.manifest_path.is_none() {
            if let Some(ref dir) = self.root_dir {
                let cand = dir.join("ridx_manifest.json");
                if cand.is_file() {
                    self.manifest_path = Some(cand);
                }
            }
        }

        if let Some(ref mf) = self.manifest_path {
            let current_manifest_mtime = fs::metadata(mf).and_then(|m| m.modified()).ok();
            if current_manifest_mtime != self.manifest_mtime {
                self.manifest_mtime = current_manifest_mtime;
                if let Ok(content) = fs::read(mf) {
                    if let Ok(manifest) = serde_json::from_slice::<VariantManifest>(&content) {
                        let base_dir = mf.parent().unwrap_or_else(|| Path::new("."));
                        let new_vdir = base_dir.join(&manifest.variant_dir);
                        if Some(&manifest.active_variant_id) != self.active_variant_id.as_ref() {
                            // Active variant changed! Atomically switch reader set
                            if new_vdir.is_dir() {
                                let mut new_files = Vec::new();
                                collect_ridx_files(&new_vdir, &mut new_files)?;
                                new_files.sort();

                                let mut new_readers = Vec::with_capacity(new_files.len());
                                let mut new_paths = Vec::with_capacity(new_files.len());
                                let mut new_mtimes = Vec::with_capacity(new_files.len());

                                for fp in new_files {
                                    if let Ok(r) = IndexReader::open(&fp) {
                                        let mt = fs::metadata(&fp).and_then(|m| m.modified()).ok();
                                        new_readers.push(r);
                                        new_paths.push(fp);
                                        new_mtimes.push(mt);
                                    }
                                }

                                if !new_readers.is_empty() {
                                    self.readers = new_readers;
                                    self.paths = new_paths;
                                    self.mtimes = new_mtimes;
                                    self.root_dir = Some(new_vdir);
                                    self.active_variant_id = Some(manifest.active_variant_id);
                                    return Ok(true);
                                }
                            }
                        }
                    }
                }
            }
        }

        // 1. Reconcile existing files: remove deleted, reload modified
        let mut i = 0;
        while i < self.paths.len() {
            let p = &self.paths[i];
            if !p.exists() {
                self.paths.remove(i);
                self.readers.remove(i);
                self.mtimes.remove(i);
                changed = true;
                continue;
            }

            let current_mtime = fs::metadata(p).and_then(|m| m.modified()).ok();
            if current_mtime != self.mtimes[i] {
                if let Ok(new_reader) = IndexReader::open(p) {
                    self.readers[i] = new_reader;
                    self.mtimes[i] = current_mtime;
                    changed = true;
                }
            }
            i += 1;
        }

        // 2. If loaded from directory, check for new .ridx files
        if let Some(ref dir) = self.root_dir {
            let mut existing_set = std::collections::HashSet::new();
            for p in &self.paths {
                existing_set.insert(p.clone());
            }

            let mut all_files = Vec::new();
            if dir.is_dir() {
                collect_ridx_files(dir, &mut all_files)?;
            }

            for file_path in all_files {
                if !existing_set.contains(&file_path) {
                    if let Ok(reader) = IndexReader::open(&file_path) {
                        let mtime = fs::metadata(&file_path).and_then(|m| m.modified()).ok();
                        self.readers.push(reader);
                        self.paths.push(file_path);
                        self.mtimes.push(mtime);
                        changed = true;
                    }
                }
            }
        }

        Ok(changed)
    }
}

fn collect_ridx_files(dir: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let p = entry.path();
        if p.is_file() {
            if let Some(ext) = p.extension() {
                if ext == "ridx" {
                    files.push(p);
                }
            }
        } else if p.is_dir() {
            // Do not descend into "variants" subdirectory if scanning root target/ridx directly
            if p.file_name().and_then(|s| s.to_str()) != Some("variants") {
                let _ = collect_ridx_files(&p, files);
            }
        }
    }
    Ok(())
}
