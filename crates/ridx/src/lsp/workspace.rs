/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Multi-root workspace tracking, dynamic folder addition/removal, and cross-workspace queries.

use ridx_format::WorkspaceIndex;
use serde_json::Value;
use std::path::{Path, PathBuf};

pub struct MultiRootWorkspace {
    roots: Vec<(PathBuf, WorkspaceIndex)>,
}

impl Default for MultiRootWorkspace {
    fn default() -> Self {
        Self::new()
    }
}

impl MultiRootWorkspace {
    pub fn new() -> Self {
        Self { roots: Vec::new() }
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.roots.is_empty()
    }

    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.roots.len()
    }

    /// The roots an index is loaded for (canonical), whatever the index records as its workspace root: a lone `.ridx`
    /// opened with `--index-path` records none (review of b3d0d293).
    pub fn roots(&self) -> impl Iterator<Item = &Path> {
        self.roots.iter().map(|(r, _)| r.as_path())
    }

    pub fn clear(&mut self) {
        self.roots.clear();
    }

    /// Answers for the root `root` (canonical) from `ws` from now on; `None` when there is no such root.
    pub fn replace_index(&mut self, root: &Path, ws: WorkspaceIndex) -> Option<&WorkspaceIndex> {
        let slot = self.roots.iter_mut().find(|(r, _)| r == root)?;
        slot.1 = ws;
        Some(&slot.1)
    }

    pub fn open_from_folders(&mut self, paths: &[PathBuf]) {
        for p in paths {
            self.add_root(p.clone());
        }
    }

    pub fn add_root(&mut self, path: PathBuf) -> bool {
        let norm_path = ridx_format::canonicalize_best_effort(&path);
        let root_dir = if norm_path.is_file() {
            if norm_path.file_name().and_then(|n| n.to_str()) == Some("Cargo.toml") {
                norm_path.parent().unwrap_or(&norm_path).to_path_buf()
            } else if norm_path.extension().and_then(|e| e.to_str()) == Some("ridx") {
                let mut dir = norm_path.parent().unwrap_or(&norm_path);
                while let Some(parent) = dir.parent() {
                    let name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    if name == ".ridx" || name == "ridx" || name == "target" || name == "variants" {
                        dir = parent;
                    } else {
                        break;
                    }
                }
                dir.to_path_buf()
            } else {
                norm_path.parent().unwrap_or(&norm_path).to_path_buf()
            }
        } else {
            norm_path.clone()
        };

        if self.roots.iter().any(|(p, _)| p == &root_dir || p == &norm_path) {
            return false;
        }

        if let Ok(ws) = WorkspaceIndex::open(&norm_path) {
            // An index opened as its directory (`W/target/ridx`) answers for the workspace it records and lies in (`W`):
            // the guard of a failed index and the choice of the index answering a file go by the same roots (review of
            // 3e51faf9: the directory's files are not under it, and an enclosing workspace answered for them).
            let index_dir = norm_path.join("ridx_manifest.json").is_file();
            let recorded = ws.workspace_root().map(ridx_format::canonicalize_best_effort).filter(|r| index_dir && norm_path.starts_with(r));
            let root_dir = recorded.unwrap_or(root_dir);
            if self.roots.iter().any(|(p, _)| p == &root_dir) {
                return false;
            }
            self.roots.push((root_dir, ws));
            return true;
        }
        false
    }

    pub fn remove_root(&mut self, path: &Path) -> bool {
        let norm_path = ridx_format::canonicalize_best_effort(path);
        let before_len = self.roots.len();
        self.roots.retain(|(p, ws)| {
            if p == &norm_path || p == path {
                return false;
            }
            if ws.paths().iter().any(|ip| ip == path || ip == &norm_path || ridx_format::canonicalize_best_effort(ip) == norm_path) {
                return false;
            }
            // The directory an index was opened as, inside the workspace it is bound to.
            if norm_path.starts_with(p) && ws.paths().iter().any(|ip| ridx_format::canonicalize_best_effort(ip).starts_with(&norm_path)) {
                return false;
            }
            true
        });
        self.roots.len() < before_len
    }

    pub fn find_workspace_for_path(&self, file_path: &Path) -> Option<&WorkspaceIndex> {
        if self.roots.len() <= 1 {
            return self.roots.first().map(|(_, ws)| ws);
        }
        let canon = ridx_format::canonicalize_cached(file_path);
        let mut best_match: Option<(usize, &WorkspaceIndex)> = None;

        for (root, ws) in &self.roots {
            if canon.starts_with(root) {
                let match_len = root.as_os_str().len();
                if let Some((best_len, _)) = best_match {
                    if match_len > best_len {
                        best_match = Some((match_len, ws));
                    }
                } else {
                    best_match = Some((match_len, ws));
                }
            }
        }

        best_match.map(|(_, ws)| ws).or_else(|| self.roots.first().map(|(_, ws)| ws))
    }

    pub fn first_workspace(&self) -> Option<&WorkspaceIndex> {
        self.roots.first().map(|(_, ws)| ws)
    }

    pub fn all_workspaces(&self) -> impl Iterator<Item = &WorkspaceIndex> {
        self.roots.iter().map(|(_, ws)| ws)
    }

    pub fn all_workspaces_mut(&mut self) -> impl Iterator<Item = &mut WorkspaceIndex> {
        self.roots.iter_mut().map(|(_, ws)| ws)
    }

    pub fn warm_background_caches(&self) {
        for (_, ws) in &self.roots {
            ws.warm_background_caches();
        }
    }

    pub fn reload_all_if_modified(&mut self) -> bool {
        let mut any_reloaded = false;
        for (_, ws) in &mut self.roots {
            if let Ok(true) = ws.reload_if_modified() {
                any_reloaded = true;
            }
        }
        any_reloaded
    }

    /// Applies the client's folder change and returns the added folders lifted to their Cargo workspaces,
    /// the roots the caller indexes.
    pub fn handle_did_change_workspace_folders(&mut self, msg: &Value) -> Vec<PathBuf> {
        let mut added_roots = Vec::new();
        if let Some(event) = msg.get("params").and_then(|p| p.get("event")) {
            if let Some(removed) = event.get("removed").and_then(|r| r.as_array()) {
                for item in removed {
                    if let Some(uri) = item.get("uri").and_then(|u| u.as_str()) {
                        let path = super::protocol::uri_to_path(uri);
                        self.remove_root(Path::new(&path));
                    }
                }
            }
            if let Some(added) = event.get("added").and_then(|a| a.as_array()) {
                for item in added {
                    if let Some(uri) = item.get("uri").and_then(|u| u.as_str()) {
                        let path = super::protocol::uri_to_path(uri);
                        for root in super::cargo_root::workspace_roots(vec![PathBuf::from(path)]) {
                            self.add_root(root.clone());
                            if !added_roots.contains(&root) {
                                added_roots.push(root);
                            }
                        }
                    }
                }
            }
        }
        added_roots
    }

    #[inline]
    pub fn doc_ws(&self, msg: &Value) -> Option<&WorkspaceIndex> {
        let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
        let file = super::protocol::uri_to_path(uri);
        self.find_workspace_for_path(Path::new(&file))
    }

    pub fn resolve_code_action_ws(&self, msg: &Value) -> Option<&WorkspaceIndex> {
        let uri = msg["params"]
            .get("data")
            .and_then(|d| d.get("uri"))
            .and_then(|u| u.as_str())
            .or_else(|| msg["params"].get("edit").and_then(|e| e.get("changes")).and_then(|c| c.as_object()).and_then(|m| m.keys().next().map(|s| s.as_str())));
        uri.and_then(|u| {
            let p = super::protocol::uri_to_path(u);
            self.find_workspace_for_path(Path::new(&p))
        })
    }

    pub fn check_file_provenance(&self, file: &str, text: &str) -> bool {
        let text_hash = ridx_format::normalized_source_hash(text.as_bytes());
        self.find_workspace_for_path(Path::new(file)).and_then(|ws| ws.source_hash_for_file(file)) == Some(text_hash)
    }

    pub fn source_hash_for_file(&self, file: &str) -> Option<u64> {
        self.find_workspace_for_path(Path::new(file)).and_then(|ws| ws.source_hash_for_file(file))
    }
    pub fn handle_file_event(&mut self, method: &str, msg: &Value, overlay_mgr: &mut crate::overlay::OverlayManager, conv: &mut super::protocol::PositionConverter) {
        if method == "workspace/didRenameFiles" {
            if let Some(files) = msg["params"]["files"].as_array() {
                for op in files {
                    let old_uri = op["oldUri"].as_str().unwrap_or("");
                    let new_uri = op["newUri"].as_str().unwrap_or("");
                    overlay_mgr.rename(&super::protocol::uri_to_path(old_uri), new_uri, &super::protocol::uri_to_path(new_uri));
                }
            }
        } else if method == "workspace/didDeleteFiles" {
            if let Some(files) = msg["params"]["files"].as_array() {
                for op in files {
                    let uri = op["uri"].as_str().unwrap_or("");
                    overlay_mgr.close(&super::protocol::uri_to_path(uri));
                }
            }
        }
        for ws in self.all_workspaces_mut() {
            super::sync_reloaded_index(ws, overlay_mgr);
        }
        conv.clear();
    }
}

#[cfg(test)]
#[path = "workspace_tests.rs"]
mod tests;
