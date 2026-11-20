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

/// A client folder: as the client names it, canonical when it was registered, and the root it stands for.
struct Folder {
    named: String,
    canonical: PathBuf,
    root: PathBuf,
}

#[derive(Default)]
pub struct Folders {
    folders: Vec<Folder>,
}

fn canonical(p: &Path) -> PathBuf {
    ridx_format::canonicalize_best_effort(p)
}

impl Folders {
    /// Registers a client folder, given as the client names it; returns its root.
    pub fn add(&mut self, named: &str) -> PathBuf {
        let canonical_folder = canonical(Path::new(named));
        let root = super::cargo_root::workspace_roots(vec![canonical_folder.clone()]).pop().map_or_else(|| canonical_folder.clone(), |r| canonical(&r));
        if !self.folders.iter().any(|f| f.named == named) {
            self.folders.push(Folder { named: named.to_string(), canonical: canonical_folder, root: root.clone() });
        }
        root
    }

    /// Registers a root the server took without a client folder (the current directory's workspace).
    pub fn add_root(&mut self, root: &Path) {
        let named = root.display().to_string();
        if !self.folders.iter().any(|f| f.named == named) {
            self.folders.push(Folder { named, canonical: root.to_path_buf(), root: root.to_path_buf() });
        }
    }

    /// Removes a client folder, named as when it was added (else by the path it resolved to then: the file system may
    /// have changed since, a symbolic link removed; review of 8a28e8ff); returns its root when no other folder stands
    /// for it any more.
    pub fn remove(&mut self, named: &str) -> Option<PathBuf> {
        let at = self.folders.iter().position(|f| f.named == named).or_else(|| {
            let now = canonical(Path::new(named));
            self.folders.iter().position(|f| f.canonical == now)
        })?;
        let Folder { root, .. } = self.folders.remove(at);
        (!self.folders.iter().any(|f| f.root == root)).then_some(root)
    }

    /// The roots of the session, each once.
    pub fn roots(&self) -> Vec<PathBuf> {
        let mut roots: Vec<PathBuf> = self.folders.iter().map(|f| f.root.clone()).collect();
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
            self.add(&path);
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
        assert_eq!(f.add(&ws.join("a").display().to_string()), ws, "a member stands for its workspace");
        f.add(&ws.join("b").display().to_string());
        assert_eq!(f.roots(), vec![ws.clone()]);
        assert_eq!(f.remove(&ws.join("a").display().to_string()), None, "b still stands for it");
        assert_eq!(f.remove(&ws.join("b").display().to_string()), Some(ws), "the last one gone: the root goes");
        assert!(f.roots().is_empty());
    }

    /// Review of 8a28e8ff: a folder named through a symbolic link removed since stayed registered.
    #[cfg(unix)]
    #[test]
    fn test_a_folder_is_removed_by_the_name_it_was_added_with() {
        let dir = tempfile::tempdir().unwrap();
        let real = canonical(dir.path()).join("real");
        std::fs::create_dir_all(&real).unwrap();
        let link = canonical(dir.path()).join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let mut f = Folders::default();
        let root = f.add(&link.display().to_string());
        std::fs::remove_file(&link).unwrap();
        assert_eq!(f.remove(&link.display().to_string()), Some(root));
        assert!(f.roots().is_empty());
    }
}
