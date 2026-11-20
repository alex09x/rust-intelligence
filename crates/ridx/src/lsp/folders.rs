/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! The client's workspace folders and the Cargo workspace each one stands for (a member opened as a folder is its
//! workspace: `cargo_root`). The roots of the session are these, whether an index was ever built for them or not
//! (review of 2045537b: with indexing off, a first index published by hand was never found), and a root leaves when the
//! last folder standing for it is removed (removing a member left its workspace loaded).

use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Default)]
pub struct Folders {
    /// Each client folder (canonical) with its root (canonical).
    folders: Vec<(PathBuf, PathBuf)>,
}

fn canonical(p: &Path) -> PathBuf {
    ridx_format::canonicalize_best_effort(p)
}

impl Folders {
    /// Registers a client folder; returns its root.
    pub fn add(&mut self, folder: &Path) -> PathBuf {
        let folder = canonical(folder);
        let root = super::cargo_root::workspace_roots(vec![folder.clone()]).pop().map_or_else(|| folder.clone(), |r| canonical(&r));
        if !self.folders.iter().any(|(f, _)| *f == folder) {
            self.folders.push((folder, root.clone()));
        }
        root
    }

    /// Removes a client folder; returns its root when no other folder stands for it any more.
    pub fn remove(&mut self, folder: &Path) -> Option<PathBuf> {
        let folder = canonical(folder);
        let at = self.folders.iter().position(|(f, _)| *f == folder)?;
        let (_, root) = self.folders.remove(at);
        (!self.folders.iter().any(|(_, r)| *r == root)).then_some(root)
    }

    /// The roots of the session, each once.
    pub fn roots(&self) -> Vec<PathBuf> {
        let mut roots: Vec<PathBuf> = self.folders.iter().map(|(_, r)| r.clone()).collect();
        roots.sort();
        roots.dedup();
        roots
    }

    /// Registers the folders of an `initialize` request (its `workspaceFolders`, else `rootUri` or `rootPath`).
    pub fn add_initialize(&mut self, msg: &Value) {
        let params = &msg["params"];
        let uris: Vec<String> = match params["workspaceFolders"].as_array().filter(|a| !a.is_empty()) {
            Some(folders) => folders.iter().filter_map(|f| f["uri"].as_str()).map(super::protocol::uri_to_path).collect(),
            None => params["rootUri"].as_str().map(super::protocol::uri_to_path).or_else(|| params["rootPath"].as_str().map(str::to_string)).into_iter().collect(),
        };
        for path in uris {
            self.add(Path::new(&path));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_a_root_leaves_with_the_last_folder_standing_for_it() {
        let dir = tempfile::tempdir().unwrap();
        let ws = canonical(dir.path());
        std::fs::write(ws.join("Cargo.toml"), "[workspace]\nmembers = [\"a\", \"b\"]\n").unwrap();
        for m in ["a", "b"] {
            std::fs::create_dir_all(ws.join(m).join("src")).unwrap();
            std::fs::write(ws.join(m).join("Cargo.toml"), format!("[package]\nname = \"{m}\"\nversion = \"0.1.0\"\n")).unwrap();
            std::fs::write(ws.join(m).join("src/lib.rs"), "").unwrap();
        }
        let mut f = Folders::default();
        assert_eq!(f.add(&ws.join("a")), ws, "a member stands for its workspace");
        f.add(&ws.join("b"));
        assert_eq!(f.roots(), vec![ws.clone()]);
        assert_eq!(f.remove(&ws.join("a")), None, "b still stands for it");
        assert_eq!(f.remove(&ws.join("b")), Some(ws), "the last one gone: the root goes");
        assert!(f.roots().is_empty());
    }
}
