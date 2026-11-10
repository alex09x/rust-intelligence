/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Corpus source loader (`CorpusSourceDb`), UTF-8 `(line, char_col)` <-> byte offset converter
//! (`LineIndex`), and `.deps/*.json` compilation-unit metadata loader for `v5` parity runs.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use ridx_format::WorkspaceIndex;
use ridx_syntax::name_res::{join_norm_into, join_norm_path, Edition, FileId, SourceDb};

/// Fast `(1-based line, 1-based char_col)` <-> UTF-8 byte offset converter for one source file.
pub struct LineIndex {
    line_starts: Vec<u32>,
    is_ascii: bool,
}

impl LineIndex {
    pub fn new(text: &str) -> Self {
        let bytes = text.as_bytes();
        let is_ascii = text.is_ascii();
        let mut line_starts = Vec::with_capacity(bytes.len() / 32 + 2);
        line_starts.push(0);
        for (i, &b) in bytes.iter().enumerate() {
            if b == b'\n' {
                line_starts.push((i + 1) as u32);
            }
        }
        Self { line_starts, is_ascii }
    }

    /// Converts 1-based `(line, char_col)` (matching `rustc` `SourceMap::lookup_char_pos`) to UTF-8 byte offset.
    #[inline]
    pub fn to_byte(&self, text: &str, line_1: u32, col_1: u32) -> Option<u32> {
        let li = line_1.checked_sub(1)? as usize;
        let line_start = *self.line_starts.get(li)? as usize;
        let char_off = col_1.checked_sub(1)? as usize;
        if self.is_ascii {
            let off = line_start.checked_add(char_off)?;
            return (off <= text.len()).then_some(off as u32);
        }
        let slice = text.get(line_start..)?;
        if char_off == 0 {
            return Some(line_start as u32);
        }
        let mut seen = 0usize;
        for (byte_idx, _) in slice.char_indices() {
            if seen == char_off {
                return Some((line_start + byte_idx) as u32);
            }
            seen += 1;
        }
        (seen == char_off).then_some((line_start + slice.len()) as u32)
    }

    /// Converts UTF-8 byte offset to 1-based `(line, char_col)`.
    pub fn to_line_col(&self, text: &str, byte_off: u32) -> (u32, u32) {
        let b = (byte_off as usize).min(text.len());
        let li = self.line_starts.partition_point(|&s| (s as usize) <= b).saturating_sub(1);
        let ls = self.line_starts[li] as usize;
        let col = if self.is_ascii { b - ls + 1 } else { text[ls..b].chars().count() + 1 };
        ((li + 1) as u32, col as u32)
    }
}

pub struct CorpusFile {
    pub rel_path: String,
    pub text: String,
    pub lines: LineIndex,
}

/// Workspace-wide `SourceDb` holding all `.rs` files under a corpus checkout.
pub struct CorpusSourceDb {
    pub ws_root: PathBuf,
    pub canon_root: PathBuf,
    pub files: Vec<CorpusFile>,
    pub(crate) by_rel: HashMap<String, FileId>,
    pub(crate) by_abs: HashMap<PathBuf, FileId>,
}

impl CorpusSourceDb {
    pub fn load(ws_root: &Path) -> Self {
        let canon_root = ws_root.canonicalize().unwrap_or_else(|_| ws_root.to_path_buf());
        let mut raw_paths = Vec::new();
        let mut ancestors = Vec::new();
        collect_rs_files(ws_root, &mut ancestors, &mut raw_paths);
        raw_paths.sort();

        let mut files = Vec::with_capacity(raw_paths.len());
        let mut by_rel = HashMap::with_capacity(raw_paths.len() * 2);
        let mut by_abs = HashMap::with_capacity(raw_paths.len() * 2);

        for abs in raw_paths {
            let Ok(text) = fs::read_to_string(&abs) else {
                continue;
            };
            let rel_raw = abs.strip_prefix(ws_root).unwrap_or(&abs).to_string_lossy();
            let rel_path = join_norm_path("", &rel_raw);
            let id = FileId(files.len() as u32);
            let lines = LineIndex::new(&text);
            by_rel.insert(rel_path.clone(), id);
            by_abs.insert(abs.clone(), id);
            if let Ok(ca) = abs.canonicalize() {
                by_abs.entry(ca).or_insert(id);
            }
            files.push(CorpusFile { rel_path, text, lines });
        }

        Self { ws_root: ws_root.to_path_buf(), canon_root, files, by_rel, by_abs }
    }

    pub fn load_out_dirs(&mut self, metas: &[ReaderMeta]) {
        let mut seen_dirs = HashSet::new();
        for m in metas {
            if m.is_dependency {
                continue;
            }
            let Some(out_dir_str) = m.env.get("OUT_DIR") else {
                continue;
            };
            let out_dir = PathBuf::from(out_dir_str);
            if !seen_dirs.insert(out_dir.clone()) || !out_dir.is_dir() {
                continue;
            }
            let mut extra = Vec::new();
            let mut ancestors = Vec::new();
            collect_rs_files(&out_dir, &mut ancestors, &mut extra);
            extra.sort();
            for abs in extra {
                if self.by_abs.contains_key(&abs) {
                    continue;
                }
                let Ok(text) = fs::read_to_string(&abs) else {
                    continue;
                };
                let norm_abs = join_norm_path("", &abs.to_string_lossy());
                let id = FileId(self.files.len() as u32);
                let lines = LineIndex::new(&text);
                self.by_rel.entry(norm_abs.clone()).or_insert(id);
                self.by_abs.insert(abs.clone(), id);
                if let Ok(ca) = abs.canonicalize() {
                    let norm_ca = join_norm_path("", &ca.to_string_lossy());
                    self.by_rel.entry(norm_ca).or_insert(id);
                    self.by_abs.entry(ca).or_insert(id);
                }
                self.files.push(CorpusFile { rel_path: norm_abs, text, lines });
            }
        }
    }

    pub fn lookup_abs_or_rel(&self, source_root: Option<&Path>, raw_file: &str) -> Option<FileId> {
        let p = Path::new(raw_file);
        if p.is_absolute() {
            if let Some(&fid) = self.by_abs.get(p) {
                return Some(fid);
            }
            if let Ok(rel) = p.strip_prefix(&self.ws_root).or_else(|_| p.strip_prefix(&self.canon_root)) {
                let norm = join_norm_path("", &rel.to_string_lossy());
                if let Some(&fid) = self.by_rel.get(&norm) {
                    return Some(fid);
                }
            }
            return None;
        }
        if let Some(sr) = source_root {
            let joined = sr.join(p);
            if let Some(&fid) = self.by_abs.get(&joined) {
                return Some(fid);
            }
            if let Ok(rel) = joined.strip_prefix(&self.ws_root).or_else(|_| joined.strip_prefix(&self.canon_root)) {
                let norm = join_norm_path("", &rel.to_string_lossy());
                if let Some(&fid) = self.by_rel.get(&norm) {
                    return Some(fid);
                }
            }
        }
        let norm = join_norm_path("", raw_file);
        self.by_rel.get(&norm).copied()
    }
}

impl SourceDb for CorpusSourceDb {
    #[inline]
    fn resolve_path(&self, normalized_path: &str) -> Option<FileId> {
        if let Some(&fid) = self.by_rel.get(normalized_path) {
            return Some(fid);
        }
        if normalized_path.starts_with('/') {
            return self.lookup_abs_or_rel(None, normalized_path);
        }
        None
    }

    #[inline]
    fn file_path(&self, file: FileId) -> &str {
        self.files.get(file.0 as usize).map_or("", |f| f.rel_path.as_str())
    }

    #[inline]
    fn file_text(&self, file: FileId) -> &str {
        self.files.get(file.0 as usize).map_or("", |f| f.text.as_str())
    }
}

pub(crate) fn collect_rs_files(dir: &Path, ancestors: &mut Vec<PathBuf>, out: &mut Vec<PathBuf>) {
    if dir == Path::new("/") || ancestors.len() >= 32 {
        return;
    }
    let Ok(canon_dir) = dir.canonicalize() else {
        return;
    };
    if ancestors.contains(&canon_dir) {
        return;
    }
    let Ok(rd) = fs::read_dir(dir) else {
        return;
    };
    ancestors.push(canon_dir);
    for entry in rd.flatten() {
        let p = entry.path();
        let Ok(md) = fs::metadata(&p) else {
            continue;
        };
        if md.is_dir() {
            let name = entry.file_name();
            if matches!(name.to_str(), Some("target" | ".git" | "node_modules")) {
                continue;
            }
            collect_rs_files(&p, ancestors, out);
        } else if md.is_file() && p.extension().and_then(|s| s.to_str()) == Some("rs") {
            out.push(p);
        }
    }
    ancestors.pop();
}

/// Extracted sidecar metadata for one `IndexReader` slot in `WorkspaceIndex`.
pub struct ReaderMeta {
    pub crate_name: String,
    pub edition: Edition,
    pub is_dependency: bool,
    pub is_sysroot: bool,
    pub target_kind: String,
    pub is_test_harness: bool,
    pub source_root: Option<PathBuf>,
    pub root_rs_abs: Option<PathBuf>,
    pub extern_units: BTreeMap<String, String>,
    pub env: BTreeMap<String, String>,
}

pub fn load_reader_metas(ws: &WorkspaceIndex) -> Vec<ReaderMeta> {
    let has_core_sysroot = ws.readers().iter().any(|r| r.crate_name() == "core");
    let mut out = Vec::with_capacity(ws.crate_count());
    for idx in 0..ws.crate_count() {
        let reader = &ws.readers()[idx];
        let mut crate_name = reader.crate_name().replace('-', "_");
        let mut edition = Edition::Edition2021;
        let mut is_dependency = ws.is_reader_dependency(idx);
        let is_sysroot = has_core_sysroot && ws.meta_path_at(idx).is_none();
        let mut target_kind = String::new();
        let mut is_test_harness = false;
        let mut source_root = ws.source_root_at(idx).map(Path::to_path_buf);
        let mut root_rs_abs: Option<PathBuf> = None;
        let mut extern_units: BTreeMap<String, String> = BTreeMap::new();
        let mut env: BTreeMap<String, String> = BTreeMap::new();

        if let Some(meta_path) = ws.meta_path_at(idx) {
            if let Ok(raw) = fs::read_to_string(meta_path) {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw) {
                    if let Some(n) = val.get("name").and_then(|v| v.as_str()) {
                        crate_name = n.replace('-', "_");
                    }
                    if let Some(dep) = val.get("is_dependency").and_then(|v| v.as_bool()) {
                        is_dependency = dep;
                    }
                    if let Some(tk) = val.get("target_kind").and_then(|v| v.as_str()) {
                        target_kind = tk.to_string();
                    }
                    if let Some(sr) = val.get("source_root").and_then(|v| v.as_str()) {
                        source_root = Some(PathBuf::from(sr));
                    }
                    if let Some(obj) = val.get("extern_units").and_then(|v| v.as_object()) {
                        for (k, v) in obj {
                            if !k.contains('-') {
                                if let Some(uk) = v.as_str() {
                                    extern_units.insert(k.clone(), uk.to_string());
                                }
                            }
                        }
                        for (k, v) in obj {
                            let norm_k = k.split('-').next().unwrap_or(k).replace('-', "_");
                            if let Some(uk) = v.as_str() {
                                extern_units.entry(norm_k).or_insert_with(|| uk.to_string());
                            }
                        }
                    }
                    if let Some(obj) = val.get("env").and_then(|v| v.as_object()) {
                        for (k, v) in obj {
                            if let Some(ev) = v.as_str() {
                                env.insert(k.clone(), ev.to_string());
                            }
                        }
                    }
                    if let Some(args) = val.get("compiler_args").and_then(|v| v.as_array()) {
                        let strs: Vec<&str> = args.iter().filter_map(|x| x.as_str()).collect();
                        parse_compiler_args(&strs, source_root.as_deref(), &mut edition, &mut is_test_harness, &mut root_rs_abs);
                    }
                }
            }
        }

        out.push(ReaderMeta { crate_name, edition, is_dependency, is_sysroot, target_kind, is_test_harness, source_root, root_rs_abs, extern_units, env });
    }
    out
}

#[inline]
pub fn resolve_include_in_corpus<'w>(src: &'w CorpusSourceDb, from_file: FileId, rel_or_abs: &str) -> Option<(FileId, &'w str)> {
    let base_dir = if from_file.is_some() {
        let fp = src.file_path(from_file);
        match fp.rfind('/') {
            Some(0) => "/",
            Some(idx) => &fp[..idx],
            None => "",
        }
    } else {
        ""
    };
    let mut norm = String::with_capacity(base_dir.len() + rel_or_abs.len() + 1);
    join_norm_into(&mut norm, base_dir, rel_or_abs, "");
    let fid = src.resolve_path(&norm)?;
    Some((fid, src.file_text(fid)))
}

pub fn include_bytes_len_in_corpus(src: &CorpusSourceDb, from_file: FileId, rel_or_abs: &str) -> Option<u64> {
    if let Some((_, text)) = resolve_include_in_corpus(src, from_file, rel_or_abs) {
        return Some(text.len() as u64);
    }
    let base_dir = if from_file.is_some() {
        let fp = src.file_path(from_file);
        match fp.rfind('/') {
            Some(0) => "/",
            Some(idx) => &fp[..idx],
            None => "",
        }
    } else {
        ""
    };
    let mut norm = String::with_capacity(base_dir.len() + rel_or_abs.len() + 1);
    join_norm_into(&mut norm, base_dir, rel_or_abs, "");
    let fs_path = if norm.starts_with('/') { PathBuf::from(&norm) } else { src.ws_root.join(&norm) };
    fs::metadata(&fs_path).ok().map(|m| m.len())
}

fn parse_compiler_args(args: &[&str], source_root: Option<&Path>, edition: &mut Edition, is_test: &mut bool, root_rs: &mut Option<PathBuf>) {
    let mut i = 1usize;
    while i < args.len() {
        let a = args[i];
        if a == "--test" {
            *is_test = true;
        } else if a == "--edition" && i + 1 < args.len() {
            *edition = parse_edition(args[i + 1]);
            i += 1;
        } else if let Some(ed) = a.strip_prefix("--edition=") {
            *edition = parse_edition(ed);
        } else if !a.starts_with('-') && a.ends_with(".rs") && root_rs.is_none() {
            let p = PathBuf::from(a);
            *root_rs = Some(if p.is_absolute() {
                p
            } else if let Some(sr) = source_root {
                sr.join(p)
            } else {
                p
            });
        }
        i += 1;
    }
}

fn parse_edition(s: &str) -> Edition {
    match s {
        "2015" => Edition::Edition2015,
        "2018" => Edition::Edition2018,
        "2024" => Edition::Edition2024,
        _ => Edition::Edition2021,
    }
}
