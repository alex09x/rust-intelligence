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
}

/// A reference match within a workspace, including its crate provenance.
#[derive(Debug, Clone)]
pub struct WorkspaceReference<'a> {
    pub crate_name: &'a str,
    pub file: &'a str,
    pub span: Span,
    pub kind: DefKind,
    pub is_write: bool,
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
}

impl WorkspaceIndex {
    /// Opens a single .ridx file or scans a directory containing .ridx files.
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let p = path.as_ref();

        if p.is_file() {
            let reader = IndexReader::open(p)?;
            return Ok(Self {
                readers: vec![reader],
                paths: vec![p.to_path_buf()],
            });
        }

        // If directory, check target/ridx subfolder first if it exists
        let search_dir = if p.is_dir() && p.join("target/ridx").is_dir() {
            p.join("target/ridx")
        } else {
            p.to_path_buf()
        };

        let mut ridx_files = Vec::new();
        if search_dir.is_dir() {
            collect_ridx_files(&search_dir, &mut ridx_files)?;
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

        for file_path in ridx_files {
            match IndexReader::open(&file_path) {
                Ok(reader) => {
                    readers.push(reader);
                    paths.push(file_path);
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

        Ok(Self { readers, paths })
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
                // If it resolves to an external file or path, try resolving in workspace crates
                if res.file == "external" || res.file.is_empty() {
                    if let Some(cross_def) = self.find_definition_by_path(res.path) {
                        return Some(cross_def);
                    }
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
                });
            }
        }

        matches
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
            // Recurse into subdirectories (e.g. target/ridx/deps)
            let _ = collect_ridx_files(&p, files);
        }
    }
    Ok(())
}
