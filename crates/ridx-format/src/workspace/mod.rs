//! Multi-crate workspace index support for Rust Intelligence.
//!
//! Provides unified zero-copy queries across all crates in a cargo workspace.

pub mod manifest;
pub mod navigation;
pub mod provenance;
pub mod reload;

pub use manifest::*;

use crate::reader::IndexReader;
use memmap2::Mmap;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Unified multi-crate index for a cargo workspace or multi-index directory.
pub struct WorkspaceIndex {
    pub(crate) readers: Vec<IndexReader<Mmap>>,
    pub(crate) paths: Vec<PathBuf>,
    pub(crate) mtimes: Vec<Option<std::time::SystemTime>>,
    pub(crate) source_roots: Vec<Option<PathBuf>>,
    pub(crate) unit_keys: Vec<Option<String>>,
    pub(crate) reader_generations: Vec<Option<String>>,
    pub(crate) reader_dependencies: Vec<Vec<String>>,
    pub(crate) reader_extern_units: Vec<HashMap<String, String>>,
    pub(crate) reader_file_hashes: Vec<HashMap<String, u64>>,
    pub(crate) reader_is_dependency: Vec<bool>,
    pub(crate) meta_paths: Vec<Option<PathBuf>>,
    pub(crate) meta_mtimes: Vec<Option<std::time::SystemTime>>,
    pub(crate) last_reloaded_files: HashSet<String>,
    pub(crate) root_dir: Option<PathBuf>,
    pub(crate) workspace_root: Option<PathBuf>,
    pub(crate) manifest_path: Option<PathBuf>,
    pub(crate) manifest_mtime: Option<std::time::SystemTime>,
    pub(crate) active_variant_id: Option<String>,
}

impl WorkspaceIndex {
    /// Opens a single .ridx file or scans a directory containing .ridx files.
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let p = path.as_ref();

        if p.is_file() {
            let mtime = fs::metadata(p).and_then(|m| m.modified()).ok();
            let reader = IndexReader::open(p)?;
            let embedded = reader.embedded_file_hashes();
            let (
                source_root,
                unit_key,
                gen,
                dependencies,
                extern_units,
                file_hashes,
                mpath,
                mmtime,
            ) = if let Some(parent) = p.parent() {
                Self::load_file_metadata(
                    p,
                    parent,
                    reader.crate_name(),
                    reader.unit_key(),
                    reader.generation(),
                    &embedded,
                )
            } else {
                (
                    None,
                    None,
                    None,
                    Vec::new(),
                    HashMap::new(),
                    embedded,
                    None,
                    None,
                )
            };
            let is_dep = source_root.is_some();
            let ws_root = p.parent().and_then(|parent| {
                if parent.ends_with("target/ridx") {
                    parent
                        .parent()
                        .and_then(|p| p.parent())
                        .map(|p| p.to_path_buf())
                } else {
                    None
                }
            });
            return Ok(Self {
                readers: vec![reader],
                paths: vec![p.to_path_buf()],
                mtimes: vec![mtime],
                source_roots: vec![source_root],
                unit_keys: vec![unit_key],
                reader_generations: vec![gen],
                reader_dependencies: vec![dependencies],
                reader_extern_units: vec![extern_units],
                reader_file_hashes: vec![file_hashes],
                reader_is_dependency: vec![is_dep],
                meta_paths: vec![mpath],
                meta_mtimes: vec![mmtime],
                last_reloaded_files: HashSet::new(),
                root_dir: None,
                workspace_root: ws_root,
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
                    || vpath
                        .components()
                        .any(|c| c == std::path::Component::ParentDir)
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
                            format!("Extractor version missing in manifest {:?}", manifest_file),
                        ));
                    }
                }

                (
                    vdir,
                    Some((
                        manifest_file,
                        mtime,
                        manifest.active_variant_id,
                        manifest.dependency_shards,
                        if !manifest.workspace_root.is_empty() {
                            Some(PathBuf::from(manifest.workspace_root))
                        } else {
                            None
                        },
                    )),
                )
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
                    format!(
                        "Failed to parse variant manifest {:?}: {e}",
                        v_manifest_file
                    ),
                )
            })?;
            let mtime = fs::metadata(&v_manifest_file)
                .and_then(|m| m.modified())
                .ok();
            if manifest.format_version != crate::format::CURRENT_FORMAT_VERSION {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "Format version mismatch in manifest {:?}: format_version {} != current {}",
                        v_manifest_file,
                        manifest.format_version,
                        crate::format::CURRENT_FORMAT_VERSION
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
            (
                search_dir.clone(),
                Some((
                    v_manifest_file,
                    mtime,
                    manifest.active_variant_id,
                    manifest.dependency_shards,
                    if !manifest.workspace_root.is_empty() {
                        Some(PathBuf::from(manifest.workspace_root))
                    } else {
                        None
                    },
                )),
            )
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
        let mut reader_generations = Vec::with_capacity(ridx_files.len());
        let mut reader_dependencies = Vec::with_capacity(ridx_files.len());
        let mut reader_extern_units = Vec::with_capacity(ridx_files.len());
        let mut reader_file_hashes = Vec::with_capacity(ridx_files.len());
        let mut reader_is_dependency = Vec::with_capacity(ridx_files.len());
        let mut meta_paths = Vec::with_capacity(ridx_files.len());
        let mut meta_mtimes = Vec::with_capacity(ridx_files.len());

        for file_path in ridx_files {
            match IndexReader::open(&file_path) {
                Ok(reader) => {
                    let mtime = fs::metadata(&file_path).and_then(|m| m.modified()).ok();
                    let embedded = reader.embedded_file_hashes();
                    let (sroot, ukey, gen, deps, ext_units, file_hashes, mpath, mmtime) =
                        Self::load_file_metadata(
                            &file_path,
                            &scan_dir,
                            reader.crate_name(),
                            reader.unit_key(),
                            reader.generation(),
                            &embedded,
                        );
                    let is_dep = sroot.is_some();
                    readers.push(reader);
                    paths.push(file_path);
                    mtimes.push(mtime);
                    source_roots.push(sroot);
                    unit_keys.push(ukey);
                    reader_generations.push(gen);
                    reader_dependencies.push(deps);
                    reader_extern_units.push(ext_units);
                    reader_file_hashes.push(file_hashes);
                    reader_is_dependency.push(is_dep);
                    meta_paths.push(mpath);
                    meta_mtimes.push(mmtime);
                }
                Err(e) => {
                    eprintln!("Warning: skipping invalid index {:?}: {e}", file_path);
                }
            }
        }

        // Load external dependency shards from global cache if specified in manifest
        if let Some((_, _, _, ref dep_shards, _)) = manifest_info {
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
                    let canon_cand =
                        fs::canonicalize(&dep_file).unwrap_or_else(|_| dep_file.clone());
                    let already_loaded = paths.iter().any(|existing| {
                        fs::canonicalize(existing).unwrap_or_else(|_| existing.clone())
                            == canon_cand
                    }) || unit_keys
                        .iter()
                        .any(|existing_uk| existing_uk.as_deref() == Some(&dep.unit_key));
                    if already_loaded {
                        continue;
                    }

                    if let Ok(reader) = IndexReader::open(&dep_file) {
                        if !dep.unit_key.is_empty()
                            && !reader.unit_key().is_empty()
                            && dep.unit_key != reader.unit_key()
                        {
                            continue;
                        }
                        let embedded = reader.embedded_file_hashes();
                        let reader_gen = reader.generation();
                        let mut gen_valid = true;
                        if !reader_gen.is_empty() {
                            let dep_gen = dep.generation.as_deref().unwrap_or("");
                            if !dep_gen.is_empty() {
                                if dep_gen != reader_gen {
                                    gen_valid = false;
                                }
                            } else if !dep.file_hashes.is_empty() {
                                let computed_digest =
                                    crate::format::compute_source_snapshot_digest(&dep.file_hashes);
                                let computed_gen = format!("{:016x}", computed_digest);
                                if computed_gen != reader_gen {
                                    gen_valid = false;
                                }
                            }
                        }
                        let gen = if !reader_gen.is_empty() {
                            Some(reader_gen.to_string())
                        } else if gen_valid {
                            dep.generation.clone()
                        } else {
                            None
                        };
                        let mtime = fs::metadata(&dep_file).and_then(|m| m.modified()).ok();
                        readers.push(reader);
                        paths.push(dep_file);
                        mtimes.push(mtime);
                        source_roots.push(dep.source_root.as_ref().map(PathBuf::from));
                        unit_keys.push(Some(dep.unit_key.clone()));
                        reader_generations.push(gen);
                        let deps = if gen_valid {
                            dep.dependencies.clone()
                        } else {
                            Vec::new()
                        };
                        let ext_units = if gen_valid {
                            dep.extern_units.clone()
                        } else {
                            HashMap::new()
                        };
                        reader_dependencies.push(deps);
                        reader_extern_units.push(ext_units);
                        let mut combined_hashes = if gen_valid {
                            dep.file_hashes.clone()
                        } else {
                            HashMap::new()
                        };
                        let sroot_path = dep.source_root.as_deref().map(Path::new);
                        crate::format::merge_embedded_hashes_authoritative(
                            &mut combined_hashes,
                            &embedded,
                            sroot_path,
                        );
                        reader_file_hashes.push(combined_hashes);
                        reader_is_dependency.push(true);
                        meta_paths.push(None);
                        meta_mtimes.push(None);
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

        let (manifest_path, manifest_mtime, active_variant_id, manifest_ws_root) =
            match manifest_info {
                Some((mp, mm, avid, _, ws_r)) => (Some(mp), mm, Some(avid), ws_r),
                None => (None, None, None, None),
            };
        let workspace_root = manifest_ws_root.or_else(|| {
            if scan_dir.ends_with("target/ridx") {
                scan_dir
                    .parent()
                    .and_then(|p| p.parent())
                    .map(|p| p.to_path_buf())
            } else if p.is_dir() {
                Some(p.to_path_buf())
            } else {
                None
            }
        });

        Ok(Self {
            readers,
            paths,
            mtimes,
            source_roots,
            unit_keys,
            reader_generations,
            reader_dependencies,
            reader_extern_units,
            reader_file_hashes,
            reader_is_dependency,
            meta_paths,
            meta_mtimes,
            last_reloaded_files: HashSet::new(),
            root_dir: Some(scan_dir),
            workspace_root,
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

    /// Returns the snapshot generation for reader at index i.
    pub fn generation_at(&self, idx: usize) -> Option<&str> {
        self.reader_generations.get(idx).and_then(|g| g.as_deref())
    }

    /// Returns the sidecar metadata path for reader at index i.
    pub fn meta_path_at(&self, idx: usize) -> Option<&Path> {
        self.meta_paths.get(idx).and_then(|p| p.as_deref())
    }

    /// Returns the sidecar metadata modification time for reader at index i.
    pub fn meta_mtime_at(&self, idx: usize) -> Option<std::time::SystemTime> {
        self.meta_mtimes.get(idx).copied().flatten()
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
                is_dependency: self.reader_is_dependency.get(i).copied().unwrap_or(false),
            });
        }

        stats
    }

    /// Currently active build variant ID, if loaded from a variant manifest.
    pub fn active_variant_id(&self) -> Option<&str> {
        self.active_variant_id.as_deref()
    }

    /// Root directory of the workspace if known.
    pub fn workspace_root(&self) -> Option<&Path> {
        self.workspace_root.as_deref()
    }
}

pub(crate) fn collect_ridx_files(dir: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
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
