//! Multi-crate workspace index support for Rust Intelligence.
//!
//! Provides unified zero-copy queries across all crates in a cargo workspace.

use crate::format::{DefKind, Span};
use crate::reader::{IndexReader, ResolvedDefinition};
use memmap2::Mmap;
use std::collections::{HashMap, HashSet};
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
    pub source_root: Option<&'a Path>,
    pub unit_key: Option<&'a str>,
}

impl<'a> WorkspaceSymbol<'a> {
    pub fn resolved_path(&self) -> PathBuf {
        let p = Path::new(self.file);
        if p.is_absolute() {
            p.to_path_buf()
        } else if let Some(root) = self.source_root {
            root.join(p)
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
            root.join(p)
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
            root.join(p)
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
    pub version: String,
    pub unit_key: String,
    pub object_path: String,
    #[serde(default)]
    pub source_root: Option<String>,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub extern_units: HashMap<String, String>,
}

fn default_format_version() -> u32 {
    crate::format::CURRENT_FORMAT_VERSION
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
    pub is_dependency: bool,
}

/// Unified multi-crate index for a cargo workspace or multi-index directory.
pub struct WorkspaceIndex {
    readers: Vec<IndexReader<Mmap>>,
    paths: Vec<PathBuf>,
    mtimes: Vec<Option<std::time::SystemTime>>,
    source_roots: Vec<Option<PathBuf>>,
    unit_keys: Vec<Option<String>>,
    reader_dependencies: Vec<Vec<String>>,
    reader_extern_units: Vec<HashMap<String, String>>,
    last_reloaded_files: HashSet<String>,
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
            let (source_root, unit_key, dependencies, extern_units) = if let Some(parent) = p.parent() {
                Self::load_file_metadata(p, parent, reader.crate_name())
            } else {
                (None, None, Vec::new(), HashMap::new())
            };
            return Ok(Self {
                readers: vec![reader],
                paths: vec![p.to_path_buf()],
                mtimes: vec![mtime],
                source_roots: vec![source_root],
                unit_keys: vec![unit_key],
                reader_dependencies: vec![dependencies],
                reader_extern_units: vec![extern_units],
                last_reloaded_files: HashSet::new(),
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
            let content = fs::read(&manifest_file)?;
            let manifest = serde_json::from_slice::<VariantManifest>(&content).map_err(|e| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Failed to parse variant manifest {:?}: {e}", manifest_file),
                )
            })?;

            let mtime = fs::metadata(&manifest_file).and_then(|m| m.modified()).ok();
            if !manifest.active_variant_id.is_empty() {
                let expected_variant_dir = format!("variants/{}", manifest.active_variant_id);
                let vdir_str = manifest.variant_dir.trim();
                let vpath = Path::new(vdir_str);
                if vdir_str.is_empty()
                    || vpath.is_absolute()
                    || vpath.components().any(|c| c == std::path::Component::ParentDir)
                    || vpath == Path::new(".")
                    || vdir_str != expected_variant_dir
                {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "Active variant manifest {:?} specifies variant_dir {:?} which does not match active_variant_id {:?} (expected {:?})",
                            manifest_file, manifest.variant_dir, manifest.active_variant_id, expected_variant_dir
                        ),
                    ));
                }

                let vdir = search_dir.join(&expected_variant_dir);
                if vdir == search_dir || !vdir.is_dir() {
                    return Err(io::Error::new(
                        io::ErrorKind::NotFound,
                        format!(
                            "Active variant directory {:?} specified in {:?} does not exist. Run 'ridx index' to build this variant.",
                            vdir, manifest_file
                        ),
                    ));
                }

                if manifest.format_version != crate::format::CURRENT_FORMAT_VERSION {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "Format version mismatch in manifest {:?}: format_version {} != current {}",
                            manifest_file, manifest.format_version, crate::format::CURRENT_FORMAT_VERSION
                        ),
                    ));
                }

                match manifest.extractor_version.as_deref() {
                    Some(ver) if ver == crate::format::EXTRACTOR_VERSION => {}
                    Some(ver) => {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!(
                                "Extractor version mismatch in manifest {:?}: extractor_version {} != current {}",
                                manifest_file, ver, crate::format::EXTRACTOR_VERSION
                            ),
                        ));
                    }
                    None => {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!(
                                "Extractor version missing in manifest {:?}",
                                manifest_file
                            ),
                        ));
                    }
                }

                (vdir, Some((manifest_file, mtime, manifest.active_variant_id, manifest.dependency_shards)))
            } else {
                // Legacy manifest without active_variant_id: scan search_dir directly
                (search_dir.clone(), None)
            }
        } else if search_dir.join("variant_manifest.json").is_file() {
            let v_manifest_file = search_dir.join("variant_manifest.json");
            let content = fs::read(&v_manifest_file)?;
            let manifest = serde_json::from_slice::<VariantManifest>(&content).map_err(|e| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Failed to parse variant manifest {:?}: {e}", v_manifest_file),
                )
            })?;
            let mtime = fs::metadata(&v_manifest_file).and_then(|m| m.modified()).ok();
            if manifest.format_version != crate::format::CURRENT_FORMAT_VERSION {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "Format version mismatch in manifest {:?}: format_version {} != current {}",
                        v_manifest_file, manifest.format_version, crate::format::CURRENT_FORMAT_VERSION
                    ),
                ));
            }
            match manifest.extractor_version.as_deref() {
                Some(ver) if ver == crate::format::EXTRACTOR_VERSION => {}
                Some(ver) => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "Extractor version mismatch in manifest {:?}: extractor_version {} != current {}",
                            v_manifest_file, ver, crate::format::EXTRACTOR_VERSION
                        ),
                    ));
                }
                None => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "Extractor version missing in manifest {:?}",
                            v_manifest_file
                        ),
                    ));
                }
            }
            (search_dir.clone(), Some((v_manifest_file, mtime, manifest.active_variant_id, manifest.dependency_shards)))
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
        let mut source_roots = Vec::with_capacity(ridx_files.len());
        let mut unit_keys = Vec::with_capacity(ridx_files.len());
        let mut reader_dependencies = Vec::with_capacity(ridx_files.len());
        let mut reader_extern_units = Vec::with_capacity(ridx_files.len());

        for file_path in ridx_files {
            match IndexReader::open(&file_path) {
                Ok(reader) => {
                    let mtime = fs::metadata(&file_path).and_then(|m| m.modified()).ok();
                    let (sroot, ukey, deps, ext_units) =
                        Self::load_file_metadata(&file_path, &scan_dir, reader.crate_name());
                    readers.push(reader);
                    paths.push(file_path);
                    mtimes.push(mtime);
                    source_roots.push(sroot);
                    unit_keys.push(ukey);
                    reader_dependencies.push(deps);
                    reader_extern_units.push(ext_units);
                }
                Err(e) => {
                    eprintln!("Warning: skipping invalid index {:?}: {e}", file_path);
                }
            }
        }

        // Load external dependency shards from global cache if specified in manifest
        if let Some((_, _, _, ref dep_shards)) = manifest_info {
            let cache_dir = crate::format::resolve_cache_dir();
            for dep in dep_shards {
                let obj_path = PathBuf::from(&dep.object_path);
                let cand = if obj_path.is_file() {
                    Some(obj_path)
                } else {
                    let fallback = crate::format::cache_object_path(&cache_dir, &dep.unit_key);
                    if fallback.is_file() {
                        Some(fallback)
                    } else {
                        None
                    }
                };

                if let Some(dep_file) = cand {
                    if let Ok(reader) = IndexReader::open(&dep_file) {
                        let mtime = fs::metadata(&dep_file).and_then(|m| m.modified()).ok();
                        readers.push(reader);
                        paths.push(dep_file);
                        mtimes.push(mtime);
                        source_roots.push(dep.source_root.as_ref().map(PathBuf::from));
                        unit_keys.push(Some(dep.unit_key.clone()));
                        reader_dependencies.push(dep.dependencies.clone());
                        reader_extern_units.push(dep.extern_units.clone());
                    }
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
            Some((mp, mm, avid, _)) => (Some(mp), mm, Some(avid)),
            None => (None, None, None),
        };

        Ok(Self {
            readers,
            paths,
            mtimes,
            source_roots,
            unit_keys,
            reader_dependencies,
            reader_extern_units,
            last_reloaded_files: HashSet::new(),
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
                is_dependency: self.source_roots.get(i).and_then(|r| r.as_ref()).is_some(),
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
        for (i, reader) in self.readers.iter().enumerate() {
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
                        source_root: self.source_roots.get(i).and_then(|r| r.as_deref()),
                        unit_key: self.unit_keys.get(i).and_then(|u| u.as_deref()),
                    });
                }
            }
        }

        // General search across all crates
        for (i, reader) in self.readers.iter().enumerate() {
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
                    source_root: self.source_roots.get(i).and_then(|r| r.as_deref()),
                    unit_key: self.unit_keys.get(i).and_then(|u| u.as_deref()),
                });
            }
        }

        None
    }

    /// Returns the source root path for a loaded crate, if it is an external dependency.
    pub fn crate_source_root(&self, crate_name: &str) -> Option<&Path> {
        for (i, reader) in self.readers.iter().enumerate() {
            if reader.crate_name() == crate_name {
                if let Some(Some(root)) = self.source_roots.get(i) {
                    return Some(root.as_path());
                }
            }
        }
        None
    }

    /// Resolves a file path for a given crate.
    /// If the path is relative and the crate is an external dependency with a source root,
    /// joins the source root with the relative path.
    pub fn resolve_path_for_crate(&self, crate_name: &str, file: &str) -> PathBuf {
        let p = Path::new(file);
        if p.is_absolute() {
            return p.to_path_buf();
        }
        if let Some(root) = self.crate_source_root(crate_name) {
            return root.join(file);
        }
        p.to_path_buf()
    }

    /// Finds a definition at a cursor location, seamlessly resolving cross-crate references.
    pub fn find_definition_at<'a>(
        &'a self,
        file: &str,
        line: u32,
        col: u32,
    ) -> Option<ResolvedDefinition<'a>> {
        let p = Path::new(file);
        let is_abs = p.is_absolute();

        for (i, reader) in self.readers.iter().enumerate() {
            let target_file = if let Some(Some(root)) = self.source_roots.get(i) {
                if is_abs {
                    if let Ok(rel) = p.strip_prefix(root) {
                        rel.to_str().unwrap_or(file)
                    } else {
                        // The queried file is absolute and outside this dependency's source_root.
                        // Skip this reader to prevent suffix collision.
                        continue;
                    }
                } else {
                    file
                }
            } else {
                if is_abs {
                    // Workspace member reader: skip if file belongs to any external dependency's source_root
                    let belongs_to_dep = self.source_roots.iter().any(|r| {
                        if let Some(r) = r.as_ref() {
                            p.starts_with(r)
                        } else {
                            false
                        }
                    });
                    if belongs_to_dep {
                        continue;
                    }
                }
                file
            };

            if let Some(mut res) = reader.find_definition_at(target_file, line, col) {
                res.source_root = self.source_roots.get(i).and_then(|r| r.as_deref());
                res.unit_key = self.unit_keys.get(i).and_then(|u| u.as_deref());
                let crate_prefix = format!("{}::", reader.crate_name());
                let is_local = res.path.starts_with(&crate_prefix) || res.path == reader.crate_name();
                if is_local && res.file != "external" {
                    return Some(res);
                }
                // If it resolves to a cross-crate target, attempt resolution in
                // workspace peer crates / dependencies to retrieve the true indexed definition and signature
                if let Some(cross_def) = self.find_cross_definition(i, &res) {
                    return Some(cross_def);
                }
                return Some(res);
            }
        }

        None
    }

    /// Resolves a cross-crate definition target with compilation-unit and path/signature awareness.
    fn find_cross_definition<'a>(
        &'a self,
        referrer_idx: usize,
        res: &ResolvedDefinition<'_>,
    ) -> Option<ResolvedDefinition<'a>> {
        let target_path = res.path;
        let extern_alias = target_path.split("::").next().unwrap_or(target_path);

        // 1. Determine target unit key for this extern alias if known
        let mut target_unit: Option<&str> = self
            .reader_extern_units
            .get(referrer_idx)
            .and_then(|m| m.get(extern_alias).map(|s| s.as_str()));

        // If not found in extern_units, check if referrer_dependencies has a unit key matching a reader
        if target_unit.is_none() {
            if let Some(deps) = self.reader_dependencies.get(referrer_idx) {
                for (j, r) in self.readers.iter().enumerate() {
                    if let Some(Some(uk)) = self.unit_keys.get(j).map(|u| u.as_deref()) {
                        if deps.iter().any(|d| d == uk)
                            && (r.crate_name() == extern_alias
                                || target_path.starts_with(&format!("{}::", r.crate_name())))
                        {
                            target_unit = Some(uk);
                            break;
                        }
                    }
                }
            }
        }

        // 2. If target unit is known: ONLY search readers matching that exact unit key!
        if let Some(target_uk) = target_unit {
            for (j, r) in self.readers.iter().enumerate() {
                if self.unit_keys.get(j).and_then(|u| u.as_deref()) == Some(target_uk) {
                    // Try looking up with original target_path (e.g. a::foo)
                    if let Some(def) = r.find_definition_by_path(target_path) {
                        let def_span = def.span();
                        return Some(ResolvedDefinition {
                            name: r.get_str(def.name_offset()),
                            path: r.get_str(def.path_offset()),
                            kind: DefKind::from(def.kind),
                            signature: if def.sig_offset() != 0 {
                                Some(r.get_str(def.sig_offset()))
                            } else {
                                None
                            },
                            file: r.get_file(def_span.file_id()),
                            start_line: def_span.start_line(),
                            start_col: def_span.start_col(),
                            end_line: def_span.end_line(),
                            end_col: def_span.end_col(),
                            is_macro_expansion: def.is_macro_expansion(),
                            source_root: self.source_roots.get(j).and_then(|r| r.as_deref()),
                            unit_key: Some(target_uk),
                        });
                    }
                    // If target_path started with an alias (e.g. a1::foo), try with reader's real crate name
                    if target_path.starts_with(&format!("{extern_alias}::")) {
                        let mapped_path = format!("{}::{}", r.crate_name(), &target_path[extern_alias.len() + 2..]);
                        if let Some(def) = r.find_definition_by_path(&mapped_path) {
                            let def_span = def.span();
                            return Some(ResolvedDefinition {
                                name: r.get_str(def.name_offset()),
                                path: r.get_str(def.path_offset()),
                                kind: DefKind::from(def.kind),
                                signature: if def.sig_offset() != 0 {
                                    Some(r.get_str(def.sig_offset()))
                                } else {
                                    None
                                },
                                file: r.get_file(def_span.file_id()),
                                start_line: def_span.start_line(),
                                start_col: def_span.start_col(),
                                end_line: def_span.end_line(),
                                end_col: def_span.end_col(),
                                is_macro_expansion: def.is_macro_expansion(),
                                source_root: self.source_roots.get(j).and_then(|r| r.as_deref()),
                                unit_key: Some(target_uk),
                            });
                        }
                    }
                }
            }
            // If target unit is known but definition wasn't found in that unit, do NOT fall back to another unit!
            return None;
        }

        // 3. Target unit is not known. Search readers matching target_crate
        let target_crate = extern_alias;
        let mut candidates = Vec::new();
        for (j, r) in self.readers.iter().enumerate() {
            if r.crate_name() == target_crate {
                if let Some(def) = r.find_definition_by_path(target_path) {
                    let def_span = def.span();
                    candidates.push(ResolvedDefinition {
                        name: r.get_str(def.name_offset()),
                        path: r.get_str(def.path_offset()),
                        kind: DefKind::from(def.kind),
                        signature: if def.sig_offset() != 0 {
                            Some(r.get_str(def.sig_offset()))
                        } else {
                            None
                        },
                        file: r.get_file(def_span.file_id()),
                        start_line: def_span.start_line(),
                        start_col: def_span.start_col(),
                        end_line: def_span.end_line(),
                        end_col: def_span.end_col(),
                        is_macro_expansion: def.is_macro_expansion(),
                        source_root: self.source_roots.get(j).and_then(|r| r.as_deref()),
                        unit_key: self.unit_keys.get(j).and_then(|u| u.as_deref()),
                    });
                }
            }
        }

        if candidates.is_empty() {
            return None;
        }
        if candidates.len() == 1 {
            return Some(candidates.remove(0));
        }

        // 4. Disambiguate among multiple candidates:
        // Match by start_line and/or file if available from the reference
        let mut best_candidate = None;
        for c_def in &candidates {
            let line_matches = res.start_line != 0 && c_def.start_line == res.start_line;
            let file_matches = res.file != "external"
                && !res.file.is_empty()
                && (c_def.file == res.file || c_def.resolved_path().to_string_lossy().ends_with(res.file));
            if line_matches && file_matches {
                return Some(c_def.clone());
            }
            if (line_matches || file_matches) && best_candidate.is_none() {
                best_candidate = Some(c_def.clone());
            }
        }

        if let Some(cand) = best_candidate {
            return Some(cand);
        }

        // Ambiguous candidates with no exact unit or file/line match: do not guess!
        None
    }

    /// Finds all references to a symbol, optionally scoped to a target dependency unit.
    pub fn find_references_for_unit<'a>(
        &'a self,
        target_path: &str,
        target_unit_key: Option<&str>,
    ) -> Vec<WorkspaceReference<'a>> {
        let mut all_refs = Vec::new();
        let target_crate = target_path.split("::").next().unwrap_or(target_path);

        for (i, reader) in self.readers.iter().enumerate() {
            let reader_unit = self.unit_keys.get(i).and_then(|u| u.as_deref());
            let reader_crate = reader.crate_name();

            // 1. If reader is the target crate itself:
            if reader_crate == target_crate {
                if let (Some(target_unit), Some(ru)) = (target_unit_key, reader_unit) {
                    if ru != target_unit {
                        continue;
                    }
                }
            } else {
                // 2. The reader is an external/referring crate.
                // If target_unit_key is given:
                if let Some(target_unit) = target_unit_key {
                    let has_deps = self.reader_dependencies.get(i).map(|d| !d.is_empty()).unwrap_or(false);
                    let has_extern_units = self.reader_extern_units.get(i).map(|m| !m.is_empty()).unwrap_or(false);

                    if has_deps || has_extern_units {
                        let in_deps = self.reader_dependencies.get(i).map(|d| d.iter().any(|dep| dep == target_unit)).unwrap_or(false);
                        let in_extern_units = self.reader_extern_units.get(i).map(|m| m.values().any(|uk| uk == target_unit)).unwrap_or(false);
                        if !in_deps && !in_extern_units {
                            // Referring crate has dependency info but does NOT depend on target_unit. Exclude!
                            continue;
                        }
                    } else {
                        // Legacy reader without dependency info: exclude if multiple versions of target_crate exist in workspace
                        let target_crate_versions = self.readers.iter().filter(|r| r.crate_name() == target_crate).count();
                        if target_crate_versions > 1 {
                            continue;
                        }
                    }
                }
            }

            // References with canonical target_path
            let refs = reader.find_references(target_path);
            let source_root = self.source_roots.get(i).and_then(|r| r.as_deref());

            for r in refs {
                let span = r.span();
                let file = reader.get_file(span.file_id());
                all_refs.push(WorkspaceReference {
                    crate_name: reader_crate,
                    file,
                    span,
                    kind: DefKind::from(r.kind),
                    is_write: r.is_write != 0,
                    is_macro_expansion: r.is_macro_expansion(),
                    source_root,
                    unit_key: reader_unit,
                });
            }

            // Also check if reader refers to target_path via an extern alias (e.g. `a1::foo` instead of `a::foo`)
            if let Some(extern_map) = self.reader_extern_units.get(i) {
                for (alias, uk) in extern_map {
                    if alias != target_crate && target_path.starts_with(&format!("{target_crate}::")) {
                        if let Some(target_unit) = target_unit_key {
                            if uk != target_unit {
                                continue;
                            }
                        }
                        let aliased_path = format!("{alias}::{}", &target_path[target_crate.len() + 2..]);
                        for r in reader.find_references(&aliased_path) {
                            let span = r.span();
                            let file = reader.get_file(span.file_id());
                            all_refs.push(WorkspaceReference {
                                crate_name: reader_crate,
                                file,
                                span,
                                kind: DefKind::from(r.kind),
                                is_write: r.is_write != 0,
                                is_macro_expansion: r.is_macro_expansion(),
                                source_root,
                                unit_key: reader_unit,
                            });
                        }
                    }
                }
            }
        }

        all_refs
    }

    /// Finds all references to a symbol across every crate in the workspace.
    pub fn find_references<'a>(&'a self, target_path: &str) -> Vec<WorkspaceReference<'a>> {
        self.find_references_for_unit(target_path, None)
    }

    /// Inferred expression type at cursor location.
    pub fn type_at(&self, file: &str, line: u32, col: u32) -> Option<&str> {
        let p = Path::new(file);
        let is_abs = p.is_absolute();

        for (i, reader) in self.readers.iter().enumerate() {
            let target_file = if let Some(Some(root)) = self.source_roots.get(i) {
                if is_abs {
                    if let Ok(rel) = p.strip_prefix(root) {
                        rel.to_str().unwrap_or(file)
                    } else {
                        continue;
                    }
                } else {
                    file
                }
            } else {
                if is_abs {
                    let belongs_to_dep = self.source_roots.iter().any(|r| {
                        if let Some(r) = r.as_ref() {
                            p.starts_with(r)
                        } else {
                            false
                        }
                    });
                    if belongs_to_dep {
                        continue;
                    }
                }
                file
            };
            if let Some(ty) = reader.type_at(target_file, line, col) {
                return Some(ty);
            }
        }
        None
    }

    /// Substring symbol search across all definitions in the workspace.
    pub fn search_symbols<'a>(&'a self, query: &str) -> Vec<WorkspaceSymbol<'a>> {
        let mut matches = Vec::new();

        for (i, reader) in self.readers.iter().enumerate() {
            let crate_name = reader.crate_name();
            let defs = reader.search_symbols(query);
            let source_root = self.source_roots.get(i).and_then(|r| r.as_deref());
            let unit_key = self.unit_keys.get(i).and_then(|u| u.as_deref());

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
                    source_root,
                    unit_key,
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
        let mut refs = self.find_references_for_unit(def.path, def.unit_key);
        // For local bindings whose references are stored by local name, try by short name if needed
        if refs.is_empty() && def.kind == DefKind::Local && def.name != def.path {
            refs = self.find_references_for_unit(def.name, def.unit_key);
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
            target_source_root: def.source_root,
            target_unit_key: def.unit_key,
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
        self.last_reloaded_files.clear();

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
                        if manifest.format_version != crate::format::CURRENT_FORMAT_VERSION {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                format!(
                                    "Format version mismatch in reloaded manifest {:?}: format_version {} != current {}",
                                    mf, manifest.format_version, crate::format::CURRENT_FORMAT_VERSION
                                ),
                            ));
                        }
                        if manifest.extractor_version.as_deref() != Some(crate::format::EXTRACTOR_VERSION) {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                format!(
                                    "Extractor version mismatch in reloaded manifest {:?}: extractor_version {:?} != current {:?}",
                                    mf, manifest.extractor_version, crate::format::EXTRACTOR_VERSION
                                ),
                            ));
                        }

                        let base_dir = mf.parent().unwrap_or_else(|| Path::new("."));
                        if Some(&manifest.active_variant_id) != self.active_variant_id.as_ref() {
                            let expected_variant_dir = format!("variants/{}", manifest.active_variant_id);
                            let vdir_str = manifest.variant_dir.trim();
                            if !manifest.active_variant_id.is_empty() && vdir_str == expected_variant_dir {
                                let new_vdir = base_dir.join(&expected_variant_dir);
                                if new_vdir != base_dir && new_vdir.is_dir() {
                                    // Active variant changed! Atomically switch reader set
                                    let mut new_files = Vec::new();
                                    collect_ridx_files(&new_vdir, &mut new_files)?;
                                    new_files.sort();

                                    let mut new_readers = Vec::with_capacity(new_files.len());
                                    let mut new_paths = Vec::with_capacity(new_files.len());
                                    let mut new_mtimes = Vec::with_capacity(new_files.len());
                                    let mut new_source_roots = Vec::with_capacity(new_files.len());
                                    let mut new_unit_keys = Vec::with_capacity(new_files.len());
                                    let mut new_reader_dependencies = Vec::with_capacity(new_files.len());
                                    let mut new_reader_extern_units = Vec::with_capacity(new_files.len());

                                    for fp in new_files {
                                        if let Ok(r) = IndexReader::open(&fp) {
                                            let mt = fs::metadata(&fp).and_then(|m| m.modified()).ok();
                                            let (sroot, ukey, deps, ext_units) =
                                                Self::load_file_metadata(&fp, &new_vdir, r.crate_name());
                                            new_readers.push(r);
                                            new_paths.push(fp);
                                            new_mtimes.push(mt);
                                            new_source_roots.push(sroot);
                                            new_unit_keys.push(ukey);
                                            new_reader_dependencies.push(deps);
                                            new_reader_extern_units.push(ext_units);
                                        }
                                    }

                                    // Load dependency shards for new variant if any
                                    let cache_dir = crate::format::resolve_cache_dir();
                                    for dep in &manifest.dependency_shards {
                                        let obj_path = PathBuf::from(&dep.object_path);
                                        let cand = if obj_path.is_file() {
                                            Some(obj_path)
                                        } else {
                                            let fallback = crate::format::cache_object_path(&cache_dir, &dep.unit_key);
                                            if fallback.is_file() {
                                                Some(fallback)
                                            } else {
                                                None
                                            }
                                        };
                                        if let Some(dep_file) = cand {
                                            if let Ok(r) = IndexReader::open(&dep_file) {
                                                let mt = fs::metadata(&dep_file).and_then(|m| m.modified()).ok();
                                                new_readers.push(r);
                                                new_paths.push(dep_file);
                                                new_mtimes.push(mt);
                                                new_source_roots.push(dep.source_root.as_ref().map(PathBuf::from));
                                                new_unit_keys.push(Some(dep.unit_key.clone()));
                                                new_reader_dependencies.push(dep.dependencies.clone());
                                                new_reader_extern_units.push(dep.extern_units.clone());
                                            }
                                        }
                                    }

                                    if !new_readers.is_empty() {
                                        self.readers = new_readers;
                                        self.paths = new_paths;
                                        self.mtimes = new_mtimes;
                                        self.source_roots = new_source_roots;
                                        self.unit_keys = new_unit_keys;
                                        self.reader_dependencies = new_reader_dependencies;
                                        self.reader_extern_units = new_reader_extern_units;
                                        self.root_dir = Some(new_vdir);
                                        self.active_variant_id = Some(manifest.active_variant_id);
                                        for idx in 0..self.readers.len() {
                                            self.record_reloaded_reader_files(idx);
                                        }
                                        return Ok(true);
                                    }
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
            let p = self.paths[i].clone();
            if !p.exists() {
                self.paths.remove(i);
                self.readers.remove(i);
                self.mtimes.remove(i);
                if i < self.source_roots.len() {
                    self.source_roots.remove(i);
                }
                if i < self.unit_keys.len() {
                    self.unit_keys.remove(i);
                }
                if i < self.reader_dependencies.len() {
                    self.reader_dependencies.remove(i);
                }
                if i < self.reader_extern_units.len() {
                    self.reader_extern_units.remove(i);
                }
                changed = true;
                continue;
            }

            let current_mtime = fs::metadata(&p).and_then(|m| m.modified()).ok();
            if current_mtime != self.mtimes[i] {
                if let Ok(new_reader) = IndexReader::open(&p) {
                    self.readers[i] = new_reader;
                    self.mtimes[i] = current_mtime;
                    if let Some(dir) = self.root_dir.clone() {
                        let (sroot, ukey, deps, ext_units) =
                            Self::load_file_metadata(&p, &dir, self.readers[i].crate_name());
                        if let Some(sr) = self.source_roots.get_mut(i) {
                            *sr = sroot;
                        }
                        if let Some(uk) = self.unit_keys.get_mut(i) {
                            *uk = ukey;
                        }
                        if let Some(dp) = self.reader_dependencies.get_mut(i) {
                            *dp = deps;
                        }
                        if let Some(eu) = self.reader_extern_units.get_mut(i) {
                            *eu = ext_units;
                        }
                    }
                    self.record_reloaded_reader_files(i);
                    changed = true;
                }
            }
            i += 1;
        }

        // 2. If loaded from directory, check for new .ridx files
        if let Some(dir) = self.root_dir.clone() {
            let mut existing_set = std::collections::HashSet::new();
            for p in &self.paths {
                existing_set.insert(p.clone());
            }

            let mut all_files = Vec::new();
            if dir.is_dir() {
                collect_ridx_files(&dir, &mut all_files)?;
            }

            for file_path in all_files {
                if !existing_set.contains(&file_path) {
                    if let Ok(reader) = IndexReader::open(&file_path) {
                        let mtime = fs::metadata(&file_path).and_then(|m| m.modified()).ok();
                        let (sroot, ukey, deps, ext_units) =
                            Self::load_file_metadata(&file_path, &dir, reader.crate_name());
                        self.readers.push(reader);
                        self.paths.push(file_path);
                        self.mtimes.push(mtime);
                        self.source_roots.push(sroot);
                        self.unit_keys.push(ukey);
                        self.reader_dependencies.push(deps);
                        self.reader_extern_units.push(ext_units);
                        let new_idx = self.readers.len() - 1;
                        self.record_reloaded_reader_files(new_idx);
                        changed = true;
                    }
                }
            }
        }

        Ok(changed)
    }

    /// Checks if a file path belongs to any shard that was newly reloaded or published in the last reload.
    pub fn is_file_reloaded(&self, file_path: &str) -> bool {
        if self.last_reloaded_files.contains(file_path) {
            return true;
        }
        let p = Path::new(file_path);
        if let Ok(canon) = p.canonicalize() {
            if self.last_reloaded_files.contains(&canon.to_string_lossy().to_string()) {
                return true;
            }
        }
        self.last_reloaded_files
            .iter()
            .any(|rf| rf == file_path || rf.ends_with(file_path) || file_path.ends_with(rf))
    }

    /// Returns the file modification timestamp of the index shard containing the given source file.
    pub fn shard_mtime_for_file(&self, file_path: &str) -> Option<std::time::SystemTime> {
        let p = Path::new(file_path);
        let is_abs = p.is_absolute();

        for (i, reader) in self.readers.iter().enumerate() {
            let target_file = if let Some(Some(root)) = self.source_roots.get(i) {
                if is_abs {
                    if let Ok(rel) = p.strip_prefix(root) {
                        rel.to_str().unwrap_or(file_path)
                    } else {
                        continue;
                    }
                } else {
                    file_path
                }
            } else {
                if is_abs {
                    let belongs_to_dep = self.source_roots.iter().any(|r| {
                        if let Some(r) = r.as_ref() {
                            p.starts_with(r)
                        } else {
                            false
                        }
                    });
                    if belongs_to_dep {
                        continue;
                    }
                }
                file_path
            };

            if reader.find_file_id(target_file).is_some() {
                return self.mtimes.get(i).copied().flatten();
            }
        }
        None
    }

    fn record_reloaded_reader_files(&mut self, idx: usize) {
        if let Some(reader) = self.readers.get(idx) {
            let source_root = self.source_roots.get(idx).and_then(|r| r.as_ref());
            let header = reader.header();
            for fid in 0..header.files_count {
                let f = reader.get_file(fid);
                if !f.is_empty() {
                    let full = if let Some(root) = source_root {
                        root.join(f)
                    } else {
                        PathBuf::from(f)
                    };
                    self.last_reloaded_files.insert(full.to_string_lossy().to_string());
                    if let Ok(canon) = full.canonicalize() {
                        self.last_reloaded_files.insert(canon.to_string_lossy().to_string());
                    }
                    self.last_reloaded_files.insert(f.to_string());
                }
            }
        }
    }

    fn load_file_metadata(
        file_path: &Path,
        base_dir: &Path,
        crate_name: &str,
    ) -> (Option<PathBuf>, Option<String>, Vec<String>, HashMap<String, String>) {
        let stem = file_path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        let stem_without_lib = stem.strip_prefix("lib").unwrap_or(stem);
        let deps_dir = base_dir.join(".deps");
        let cand_json = [
            deps_dir.join(format!("{stem_without_lib}.json")),
            deps_dir.join(format!("{stem}.json")),
            deps_dir.join(format!("{crate_name}.json")),
        ];
        for c in &cand_json {
            if c.is_file() {
                if let Ok(content) = fs::read(c) {
                    if let Ok(info) = serde_json::from_slice::<DependencyShardInfo>(&content) {
                        return (
                            info.source_root.as_ref().map(PathBuf::from),
                            Some(info.unit_key),
                            info.dependencies,
                            info.extern_units,
                        );
                    }
                }
            }
        }
        (None, None, Vec::new(), HashMap::new())
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
