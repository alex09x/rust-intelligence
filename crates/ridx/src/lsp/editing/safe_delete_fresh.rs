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
    // A relative source root is the workspace's (`crates/app`), not the server's working directory (review of
    // b0b16e39).
    let workspace = ws.workspace_root();
    let root = match ws.source_root_at(idx) {
        Some(r) if r.is_absolute() => r.to_path_buf(),
        Some(r) => ridx_format::format::join_resolving_overlap(workspace?, r),
        None if !ws.is_reader_dependency(idx) => workspace?.to_path_buf(),
        None => return None,
    };
    Some(ridx_format::format::join_resolving_overlap(&root, rel))
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
        // Every file the compiler read for the shard, not only those holding a definition or a reference: the loaded
        // source snapshot's too, a non-Rust `include!` input among them (review of c5dee55b).
        let files: Vec<(String, u64)> = reader.embedded_file_hashes().into_iter().chain(ws.reader_source_snapshot(idx).map(|(f, h)| (f.to_string(), h))).collect();
        for (f, hash) in files {
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

/// Where the sources of the indexed crates may be, as Cargo resolves them: each workspace's root, every member's
/// directory (outside the root too) and every target's (a declared hidden one too: review of c5dee55b), or a source's
/// own directory with no manifest above it.
struct Inventory {
    roots: Vec<PathBuf>,
    /// Directories that hold no source: Cargo's build directories and ridx's own (its shards and the copies of the
    /// texts it indexed), wherever `CARGO_TARGET_DIR` puts the first.
    skip: Vec<PathBuf>,
    /// Member directories, never taken for a workspace of their own.
    members: HashSet<PathBuf>,
    /// Every directory Cargo names (roots, members, targets'), read even when hidden.
    named: HashSet<PathBuf>,
}

/// The inventory of the indexed sources' workspaces; `None` where Cargo gives no answer for a manifest.
fn inventory(indexed: &HashSet<PathBuf>) -> Option<Inventory> {
    let mut inv = Inventory { roots: Vec::new(), skip: Vec::new(), members: HashSet::new(), named: HashSet::new() };
    let mut dirs: HashSet<PathBuf> = HashSet::new();
    let mut found: Vec<PathBuf> = Vec::new();
    for f in indexed {
        let Some(dir) = f.parent() else { continue };
        if !dirs.insert(dir.to_path_buf()) {
            continue;
        }
        match super::cargo_meta::layout(f).ok()? {
            Some(l) => {
                inv.skip.push(l.target_dir.clone());
                inv.skip.push(l.root.join("target/ridx"));
                inv.members.extend(l.packages.iter().cloned());
                found.push(l.root);
                found.extend(l.packages);
                found.extend(l.targets);
            }
            None => found.push(dir.to_path_buf()),
        }
    }
    // Every directory Cargo names and every one holding a source the index read is a starting point of its own, so
    // that no hidden directory above it hides it (`.cases/deep/caller.rs`, `#[path = ".private/helper.rs"]`: review of
    // 53031a6e); the walk visits each once. A shard's directory is not skipped: a source may sit beside a manual index.
    found.extend(dirs);
    found.sort();
    found.dedup();
    inv.named.extend(found.iter().cloned());
    inv.roots = found;
    Some(inv)
}

/// Every Rust file under the inventory's roots, links followed (once each), but in hidden directories other than a
/// root, the `skip` directories, any with a `CACHEDIR.TAG`, and a workspace of its own nested in a root (not a
/// member); `None` past [`MAX_FILES`] or where a directory or an entry cannot be read (review of c5dee55b: what cannot be
/// inspected is not known to hold no use).
fn rust_files(inv: &Inventory) -> Option<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut stack: Vec<PathBuf> = inv.roots.clone();
    let mut visited: HashSet<PathBuf> = HashSet::new();
    let own_workspace = |dir: &Path| std::fs::read_to_string(dir.join("Cargo.toml")).is_ok_and(|m| m.lines().any(|l| l.trim() == "[workspace]"));
    while let Some(dir) = stack.pop() {
        let canonical = ridx_format::canonicalize_best_effort(&dir);
        let root = inv.roots.contains(&canonical);
        if !visited.insert(canonical.clone())
            || inv.skip.contains(&canonical)
            || dir.join("CACHEDIR.TAG").is_file()
            || (!root && !inv.members.contains(&canonical) && own_workspace(&dir))
        {
            continue;
        }
        for e in std::fs::read_dir(&dir).ok()? {
            let path = e.ok()?.path();
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            // A dangling link points at no source; any other entry that cannot be read, a link whose target cannot be
            // searched among them, is not known to be none (review of 53031a6e).
            let meta = match std::fs::metadata(&path) {
                Ok(m) => m,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound && std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) => continue,
                Err(_) => return None,
            };
            if meta.is_dir() && (!name.starts_with('.') || inv.named.contains(&ridx_format::canonicalize_best_effort(&path))) {
                stack.push(path);
            } else if meta.is_file() && name.ends_with(".rs") {
                out.push(path);
                if out.len() > MAX_FILES {
                    return None;
                }
            }
        }
    }
    Some(out)
}

/// Whether the index has every use of `names` the workspace's sources hold now. Not when a source it read differs
/// from what it read, a mention it read is under `cfg` (another configuration compiles it), or a file it did not read
/// (a file added since, a `#[path]` module, an included input, a module of a crate the index lacks) names one or calls
/// a macro whose body names one, under its own name or a rename. `own`: the item's file and bytes, whose own
/// mentions are no use.
pub(crate) fn uses_covered(ws: &WorkspaceIndex, ov: &OverlayManager, names: &[&str], own: (&Path, std::ops::Range<usize>)) -> bool {
    let Some(indexed) = indexed_sources(ws, ov) else { return false };
    let Some(mut files) = inventory(&indexed).and_then(|inv| rust_files(&inv)) else { return false };
    // A buffer of a file not saved yet is a source too, and so is every source the index read (its gated mentions).
    files.extend(ov.open_file_paths().into_iter().map(PathBuf::from).filter(|p| p.extension().is_some_and(|e| e == "rs")));
    files.extend(indexed.iter().cloned());
    let mut seen = HashSet::new();
    // Macros called from a file the index did not read, and those called under `cfg` from one it did.
    let mut unseen_calls: Vec<String> = Vec::new();
    let mut all_calls: Vec<String> = Vec::new();
    let mut defines: Vec<(String, bool, bool, Vec<String>)> = Vec::new();
    let mut aliases: Vec<(String, String)> = Vec::new();
    while let Some(f) = files.pop() {
        let canonical = ridx_format::canonicalize_best_effort(&f);
        if !seen.insert(canonical.clone()) || cached(&canonical) {
            continue;
        }
        let Some(text) = text_of(&f, ov) else { return false };
        let is_indexed = indexed.contains(&canonical);
        let own_bytes = (canonical == own.0).then_some(&own.1);
        let s = super::safe_delete_scan::scan(&text, names, is_indexed, own_bytes);
        // What it includes is read as a source of its own; an include whose path is not known is a source not known
        // (review of 53031a6e).
        if s.unresolved_include {
            return false;
        }
        files.extend(s.includes.iter().filter_map(|inc| f.parent().map(|d| d.join(inc))));
        if is_indexed && s.gated_mention {
            return false;
        }
        if !is_indexed && s.names_one {
            return false;
        }
        for (call, gated) in s.calls {
            if !is_indexed || gated {
                unseen_calls.push(call.clone());
            }
            all_calls.push(call);
        }
        defines.extend(s.defines);
        aliases.extend(s.aliases);
    }
    // A macro names one when its body does, or calls a macro that does, or renames one that does.
    let closure = |seed: HashSet<String>| {
        let mut set = seed;
        loop {
            let more: Vec<String> = defines
                .iter()
                .filter(|d| !set.contains(&d.0) && d.3.iter().any(|c| set.contains(c)))
                .map(|d| d.0.clone())
                .chain(aliases.iter().filter(|(from, to)| set.contains(from) && !set.contains(to)).map(|(_, to)| to.clone()))
                .collect();
            if more.is_empty() {
                return set;
            }
            set.extend(more);
        }
    };
    let naming = closure(defines.iter().filter(|d| d.1).map(|d| d.0.clone()).collect());
    // A naming macro whose expansion depends on the configuration (a `cfg` in its body, or one it calls) may use the
    // item in another configuration from any call (review of 53031a6e).
    let gated_naming: HashSet<String> = closure(defines.iter().filter(|d| d.2).map(|d| d.0.clone()).collect()).intersection(&naming).cloned().collect();
    !unseen_calls.iter().any(|c| naming.contains(c)) && !all_calls.iter().any(|c| gated_naming.contains(c))
}
