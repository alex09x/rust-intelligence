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

/// The registry's, git checkouts' and toolchains' sources, which do not change under a build.
fn cached(p: &Path) -> bool {
    let s = p.to_string_lossy();
    s.contains("/.cargo/registry/") || s.contains("/.cargo/git/") || s.contains("/.rustup/") || s.contains("/rustlib/src/")
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
            let f = Path::new(&f);
            let path = if f.is_absolute() {
                f.to_path_buf()
            } else if let Some(root) = ws.source_root_at(idx) {
                root.join(f)
            } else {
                continue;
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

/// The directories whose Rust files may belong to the indexed crates: for each source, the outermost ancestor with a
/// `Cargo.toml` inside its repository, or its own directory; none inside another.
fn roots(indexed: &HashSet<PathBuf>) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    for f in indexed {
        let Some(dir) = f.parent() else { continue };
        let mut root = dir.to_path_buf();
        for a in dir.ancestors() {
            if a.join("Cargo.toml").is_file() {
                root = a.to_path_buf();
            }
            if a.join(".git").exists() {
                break;
            }
        }
        if !roots.iter().any(|r| root.starts_with(r)) {
            roots.retain(|r| !r.starts_with(&root));
            roots.push(root);
        }
    }
    roots
}

/// Every Rust file under `roots`, but in hidden directories and Cargo's build directories (which hold a
/// `CACHEDIR.TAG`: a source directory named `target` is a source directory); `None` past [`MAX_FILES`].
fn rust_files(roots: &[PathBuf]) -> Option<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut stack: Vec<PathBuf> = roots.to_vec();
    while let Some(dir) = stack.pop() {
        if dir.join("CACHEDIR.TAG").is_file() {
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
    let Some(mut files) = rust_files(&roots(&indexed)) else { return false };
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
