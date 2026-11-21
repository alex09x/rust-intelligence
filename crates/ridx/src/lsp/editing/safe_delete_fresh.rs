/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Whether the index answering `ridx/safeDelete` has every use the workspace's sources hold now, told by the sources'
//! content alone: every file the index read is as it read it, and no file it did not read may use the item. No
//! notification, timestamp or record of a build certifies an index (review of 9effe92d: an old mtime, a root opened
//! by another path, a reindex not loaded, a root registered again, an index published from outside).

use crate::overlay::OverlayManager;
use ridx_format::WorkspaceIndex;
use ridx_syntax::TokenKind as T;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// More Rust files than this under the workspace are not read on a request: the answer is then `staleIndex`.
const MAX_FILES: usize = 50_000;

/// The directories of sources no build changes: Cargo's registry and git checkouts, rustup's toolchains and the
/// sysroot, where they are on this machine (a workspace's own `.rustup/caller.rs` is no toolchain's: review of
/// 4009c2c4).
fn cache_dirs() -> &'static [PathBuf] {
    static DIRS: std::sync::OnceLock<Vec<PathBuf>> = std::sync::OnceLock::new();
    DIRS.get_or_init(|| {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let cargo = std::env::var_os("CARGO_HOME").map(PathBuf::from).or_else(|| home.as_ref().map(|h| h.join(".cargo")));
        let rustup = std::env::var_os("RUSTUP_HOME").map(PathBuf::from).or_else(|| home.as_ref().map(|h| h.join(".rustup")));
        let sysroot = std::process::Command::new("rustc")
            .args(["--print", "sysroot"])
            .stderr(std::process::Stdio::null())
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()));
        let mut dirs: Vec<PathBuf> = Vec::new();
        dirs.extend(cargo.iter().flat_map(|c| [c.join("registry"), c.join("git")]));
        dirs.extend(rustup.map(|r| r.join("toolchains")));
        dirs.extend(sysroot.filter(|s| !s.as_os_str().is_empty()));
        dirs.into_iter().map(|d| ridx_format::canonicalize_best_effort(&d)).collect()
    })
}

pub(crate) fn cached(p: &Path) -> bool {
    let p = ridx_format::canonicalize_best_effort(p);
    cache_dirs().iter().any(|d| p.starts_with(d))
}

/// The file `raw` of reader `idx` as the workspace resolves it (an absolute path, or one relative to the reader's
/// source root or the workspace's, overlapping components joined once: review of 4009c2c4); `None` when no root is
/// known.
pub(crate) fn resolve(ws: &WorkspaceIndex, idx: usize, raw: &str) -> Option<PathBuf> {
    let rel = Path::new(raw);
    if rel.is_absolute() {
        return Some(rel.to_path_buf());
    }
    let root = ws.source_root_at(idx).or_else(|| (!ws.is_reader_dependency(idx)).then(|| ws.workspace_root()).flatten())?;
    Some(ridx_format::format::join_resolving_overlap(root, rel))
}

/// Whether the text of `path` the server sees begins with a byte order mark.
pub(crate) fn starts_with_bom(path: &Path, ov: &OverlayManager) -> bool {
    match ov.get(&path.to_string_lossy()) {
        Some(o) => o.text.starts_with('\u{feff}'),
        None => std::fs::read(path).is_ok_and(|b| b.starts_with("\u{feff}".as_bytes())),
    }
}

/// The text of `path` as the server sees it: its open buffer, else the disk.
fn text_of(path: &Path, ov: &OverlayManager) -> Option<String> {
    match ov.get(&path.to_string_lossy()) {
        Some(o) => Some(o.text.clone()),
        None => std::fs::read(path).ok().map(|b| String::from_utf8_lossy(&b).into_owned()),
    }
}

/// The workspace's sources the index read (canonical), when every one is as it read it: on disk, or in its open
/// buffer (a closed caller changed with nothing telling the server holds uses the index does not have: review of
/// f4f8b604).
fn indexed_sources(ws: &WorkspaceIndex, ov: &OverlayManager) -> Option<HashSet<PathBuf>> {
    let mut out = HashSet::new();
    for (idx, reader) in ws.readers().iter().enumerate() {
        // Every file the compiler read for the shard, not only those holding a definition or a reference.
        for (f, hash) in reader.embedded_file_hashes() {
            // A file of the workspace whose place is not known is not known to be as it was read.
            let path = match resolve(ws, idx, &f) {
                Some(p) => p,
                None if ws.is_reader_dependency(idx) => continue,
                None => return None,
            };
            if hash == 0 || cached(&path) {
                continue;
            }
            let bytes = match ov.get(&path.to_string_lossy()) {
                Some(o) => Some(o.text.clone().into_bytes()),
                None => std::fs::read(&path).ok(),
            };
            if bytes.is_none_or(|t| ridx_format::normalized_source_hash(&t) != hash) {
                return None;
            }
            out.insert(ridx_format::canonicalize_best_effort(&path));
        }
    }
    Some(out)
}

/// The directories whose Rust files may belong to the indexed crates: for each source, the root of its Cargo workspace
/// as Cargo resolves it (not a repository or a workspace it is nested in), or its own directory with no manifest above
/// it; none inside another. With them, the workspaces' build directories, which hold no source (Cargo's, and ridx's
/// copies of the texts it indexed). `None` where Cargo gives no answer for a manifest.
fn roots(indexed: &HashSet<PathBuf>) -> Option<(Vec<PathBuf>, Vec<PathBuf>)> {
    let (mut roots, mut builds): (Vec<PathBuf>, Vec<PathBuf>) = (Vec::new(), Vec::new());
    let mut by_dir: std::collections::HashMap<PathBuf, PathBuf> = std::collections::HashMap::new();
    for f in indexed {
        let Some(dir) = f.parent() else { continue };
        let root = match by_dir.get(dir) {
            Some(r) => r.clone(),
            None => {
                let r = match super::cargo_meta::workspace_root(f).ok()? {
                    Some((root, build)) => {
                        builds.push(build);
                        root
                    }
                    None => dir.to_path_buf(),
                };
                by_dir.insert(dir.to_path_buf(), r.clone());
                r
            }
        };
        if !roots.iter().any(|r| root.starts_with(r)) {
            roots.retain(|r| !r.starts_with(&root));
            roots.push(root);
        }
    }
    Some((roots, builds))
}

/// Every Rust file under `roots`, but in hidden directories, Cargo's build directories (which hold a `CACHEDIR.TAG`:
/// a source directory named `target` is a source directory) and workspaces of their own nested in a root (a fixture
/// with its `[workspace]`), and the `builds` directories; `None` past [`MAX_FILES`].
fn rust_files(roots: &[PathBuf], builds: &[PathBuf]) -> Option<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut stack: Vec<PathBuf> = roots.to_vec();
    let own_workspace = |dir: &Path| std::fs::read_to_string(dir.join("Cargo.toml")).is_ok_and(|m| m.lines().any(|l| l.trim() == "[workspace]"));
    while let Some(dir) = stack.pop() {
        let build = builds.iter().any(|b| ridx_format::canonicalize_best_effort(&dir) == *b);
        if build || dir.join("CACHEDIR.TAG").is_file() || (!roots.contains(&dir) && own_workspace(&dir)) {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let (path, name) = (e.path(), e.file_name().to_string_lossy().into_owned());
            let Ok(kind) = e.file_type() else { continue };
            if kind.is_dir() && !name.starts_with('.') {
                stack.push(path);
            } else if kind.is_file() && name.ends_with(".rs") {
                out.push(path);
                if out.len() > MAX_FILES {
                    return None;
                }
            }
        }
    }
    Some(out)
}

/// What one file says about uses of `names`: whether it names one (or any identifier beyond ASCII, which may be one
/// in another normal form), the macros it calls, and the `macro_rules!` it defines: each with whether its body names
/// one and the macros its body calls.
struct Scan {
    names_one: bool,
    calls: Vec<String>,
    defines: Vec<(String, bool, Vec<String>)>,
}

fn scan(text: &str, names: &[&str]) -> Scan {
    let toks: Vec<_> = ridx_syntax::lex(text).into_iter().filter(|t| !t.is_trivia()).collect();
    let ident = |i: usize| toks.get(i).filter(|t| matches!(t.kind, T::Ident | T::RawIdent)).map(|t| ridx_syntax::sema_fn::norm_ident(t.text(text)));
    let txt = |i: usize| toks.get(i).map_or("", |t| t.text(text));
    let names_word = |w: &str| names.contains(&w) || !w.is_ascii();
    let mut s = Scan { names_one: false, calls: Vec::new(), defines: Vec::new() };
    for i in 0..toks.len() {
        let Some(w) = ident(i) else { continue };
        s.names_one |= names_word(w);
        if txt(i + 1) != "!" {
            continue;
        }
        if w == "macro_rules" {
            // `macro_rules! m { .. }`: its body up to the delimiter closing the first one.
            let Some(m) = ident(i + 2) else { continue };
            let (mut depth, mut naming, mut calls) = (0i32, false, Vec::new());
            for j in i + 3..toks.len() {
                match txt(j) {
                    "(" | "[" | "{" => depth += 1,
                    ")" | "]" | "}" => depth -= 1,
                    _ => {
                        naming |= ident(j).is_some_and(names_word);
                        if txt(j + 1) == "!" {
                            calls.extend(ident(j).map(str::to_string));
                        }
                    }
                }
                if depth <= 0 {
                    break;
                }
            }
            s.defines.push((m.to_string(), naming, calls));
        } else if matches!(txt(i + 2), "(" | "[" | "{") {
            s.calls.push(w.to_string());
        }
    }
    s
}

/// Whether the index has every use of `names` the workspace's sources hold now. Not when a source it read differs
/// from what it read, or a Rust file it did not read names one, or calls a macro the workspace defines with a body
/// naming one (a file added since, a `#[path]` module, a module of a crate the index lacks).
pub(crate) fn uses_covered(ws: &WorkspaceIndex, ov: &OverlayManager, names: &[&str]) -> bool {
    let Some(indexed) = indexed_sources(ws, ov) else { return false };
    let Some(mut files) = roots(&indexed).and_then(|(r, b)| rust_files(&r, &b)) else { return false };
    // A buffer of a file not saved yet is a source too.
    files.extend(ov.open_file_paths().into_iter().map(PathBuf::from).filter(|p| p.extension().is_some_and(|e| e == "rs")));
    let mut seen = HashSet::new();
    let mut unindexed_calls: Vec<String> = Vec::new();
    let mut defines: Vec<(String, bool, Vec<String>)> = Vec::new();
    for f in files {
        let canonical = ridx_format::canonicalize_best_effort(&f);
        if !seen.insert(canonical.clone()) || cached(&canonical) {
            continue;
        }
        let Some(text) = text_of(&f, ov) else { continue };
        let s = scan(&text, names);
        defines.extend(s.defines);
        if !indexed.contains(&canonical) {
            if s.names_one {
                return false;
            }
            unindexed_calls.extend(s.calls);
        }
    }
    // A macro names one when its body does, or calls a macro that does.
    let mut naming: HashSet<String> = defines.iter().filter(|d| d.1).map(|d| d.0.clone()).collect();
    loop {
        let more: Vec<String> = defines.iter().filter(|d| !naming.contains(&d.0) && d.2.iter().any(|c| naming.contains(c))).map(|d| d.0.clone()).collect();
        if more.is_empty() {
            break;
        }
        naming.extend(more);
    }
    !unindexed_calls.iter().any(|c| naming.contains(c))
}
