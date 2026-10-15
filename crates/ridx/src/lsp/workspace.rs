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

    pub fn clear(&mut self) {
        self.roots.clear();
    }

    pub fn open_from_folders(&mut self, paths: &[PathBuf]) {
        for p in paths {
            self.add_root(p.clone());
        }
    }

    pub fn add_root(&mut self, path: PathBuf) -> bool {
        let norm_path = path.canonicalize().unwrap_or_else(|_| path.clone());
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

        if self
            .roots
            .iter()
            .any(|(p, _)| p == &root_dir || p == &norm_path)
        {
            return false;
        }

        if let Ok(ws) = WorkspaceIndex::open(&norm_path) {
            self.roots.push((root_dir, ws));
            return true;
        }
        false
    }

    pub fn remove_root(&mut self, path: &Path) -> bool {
        let norm_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let before_len = self.roots.len();
        self.roots.retain(|(p, ws)| {
            if p == &norm_path || p == path {
                return false;
            }
            if ws.paths().iter().any(|ip| {
                ip == path
                    || ip == &norm_path
                    || ip.canonicalize().map(|c| c == norm_path).unwrap_or(false)
            }) {
                return false;
            }
            true
        });
        self.roots.len() < before_len
    }

    pub fn find_workspace_for_path(&self, file_path: &Path) -> Option<&WorkspaceIndex> {
        let canon = file_path
            .canonicalize()
            .unwrap_or_else(|_| file_path.to_path_buf());
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

        best_match
            .map(|(_, ws)| ws)
            .or_else(|| self.roots.first().map(|(_, ws)| ws))
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

    pub fn reload_all_if_modified(&mut self) -> bool {
        let mut any_reloaded = false;
        for (_, ws) in &mut self.roots {
            if let Ok(true) = ws.reload_if_modified() {
                any_reloaded = true;
            }
        }
        any_reloaded
    }

    pub fn handle_did_change_workspace_folders(&mut self, msg: &Value) {
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
                        self.add_root(PathBuf::from(path));
                    }
                }
            }
        }
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
            .or_else(|| {
                msg["params"]
                    .get("edit")
                    .and_then(|e| e.get("changes"))
                    .and_then(|c| c.as_object())
                    .and_then(|m| m.keys().next().map(|s| s.as_str()))
            });
        uri.and_then(|u| {
            let p = super::protocol::uri_to_path(u);
            self.find_workspace_for_path(Path::new(&p))
        })
    }

    pub fn check_file_provenance(&self, file: &str, text: &str) -> bool {
        let text_hash = ridx_format::normalized_source_hash(text.as_bytes());
        self.find_workspace_for_path(Path::new(file))
            .and_then(|ws| ws.source_hash_for_file(file))
            == Some(text_hash)
    }

    pub fn source_hash_for_file(&self, file: &str) -> Option<u64> {
        self.find_workspace_for_path(Path::new(file))
            .and_then(|ws| ws.source_hash_for_file(file))
    }

    pub fn handle_file_event(
        &mut self,
        method: &str,
        msg: &Value,
        overlay_mgr: &mut crate::overlay::OverlayManager,
        conv: &mut super::protocol::PositionConverter,
    ) {
        if method == "workspace/didRenameFiles" {
            if let Some(files) = msg["params"]["files"].as_array() {
                for op in files {
                    let old_uri = op["oldUri"].as_str().unwrap_or("");
                    let new_uri = op["newUri"].as_str().unwrap_or("");
                    overlay_mgr.rename(
                        &super::protocol::uri_to_path(old_uri),
                        new_uri,
                        &super::protocol::uri_to_path(new_uri),
                    );
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
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_multi_root_empty() {
        let mr = MultiRootWorkspace::new();
        assert!(mr.is_empty());
        assert_eq!(mr.len(), 0);
        assert!(mr.first_workspace().is_none());
        assert!(mr
            .find_workspace_for_path(Path::new("/non/existent"))
            .is_none());
    }

    #[test]
    fn test_multi_root_did_change_folders() {
        let mut mr = MultiRootWorkspace::new();
        let change_msg = json!({
            "params": {
                "event": {
                    "added": [{ "uri": "file:///tmp/added_ws" }],
                    "removed": [{ "uri": "file:///tmp/removed_ws" }]
                }
            }
        });
        mr.handle_did_change_workspace_folders(&change_msg);
        assert_eq!(mr.len(), 0); // /tmp/added_ws does not contain an index, so add_root returns false
    }

    #[test]
    fn test_multi_root_with_valid_index() {
        use ridx_format::IndexWriter;
        let temp_dir = std::env::temp_dir().join(format!("ridx_mr_valid_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let ridx_file = temp_dir.join("root.ridx");
        let mut writer = IndexWriter::new("mr_crate", "0.1.0");
        let mut f = std::fs::File::create(&ridx_file).unwrap();
        writer.write_to(&mut f).unwrap();

        let mut mr = MultiRootWorkspace::new();
        assert!(mr.add_root(ridx_file.clone()));
        assert_eq!(mr.len(), 1);
        assert!(!mr.is_empty());
        assert!(mr.first_workspace().is_some());
        assert_eq!(mr.all_workspaces().count(), 1);
        assert_eq!(mr.all_workspaces_mut().count(), 1);
        assert!(!mr.reload_all_if_modified());
        assert!(mr.find_workspace_for_path(&ridx_file).is_some());

        // Duplicate add returns false
        assert!(!mr.add_root(ridx_file.clone()));

        // Multi-root with longer match
        let sub_dir = temp_dir.join("sub_workspace");
        let _ = std::fs::create_dir_all(&sub_dir);
        let sub_ridx = sub_dir.join("sub.ridx");
        let mut sub_w = IndexWriter::new("sub_crate", "0.1.0");
        let mut f_sub = std::fs::File::create(&sub_ridx).unwrap();
        sub_w.write_to(&mut f_sub).unwrap();
        assert!(mr.add_root(sub_ridx.clone()));
        // Looking up path under sub_dir matches sub_ridx (longer match)
        let sub_file = sub_dir.join("src/lib.rs");
        assert!(mr.find_workspace_for_path(&sub_file).is_some());

        // Multi-root with longer directory match (lines 71-73)
        let mut mr_dirs = MultiRootWorkspace::new();
        let _ = mr_dirs.add_root(temp_dir.clone());
        let _ = mr_dirs.add_root(sub_dir.clone());
        assert!(mr_dirs.find_workspace_for_path(&sub_file).is_some());

        // Test default and open_from_folders
        let mr_def = MultiRootWorkspace::default();
        assert!(mr_def.is_empty());

        let mut mr_folders = MultiRootWorkspace::new();
        mr_folders.open_from_folders(&[ridx_file.clone(), sub_ridx.clone()]);
        assert_eq!(mr_folders.len(), 2);

        // Test handle_did_change_workspace_folders with valid URIs
        let change_valid = json!({
            "params": {
                "event": {
                    "added": [{ "uri": format!("file://{}", sub_ridx.display()) }],
                    "removed": [{ "uri": format!("file://{}", ridx_file.display()) }]
                }
            }
        });
        mr.handle_did_change_workspace_folders(&change_valid);

        // Test handle_did_change_workspace_folders missing fields
        mr.handle_did_change_workspace_folders(&json!({ "params": {} }));
        mr.handle_did_change_workspace_folders(&json!({ "params": { "event": { "added": [] } } }));
        mr.handle_did_change_workspace_folders(
            &json!({ "params": { "event": { "removed": [] } } }),
        );

        // Remove root
        assert!(mr.remove_root(&sub_ridx));

        // Reload modified root (line 101)
        let mut mr_reload = MultiRootWorkspace::new();
        mr_reload.add_root(sub_ridx.clone());
        let mut w_new = IndexWriter::new("sub_crate", "0.1.0");
        std::thread::sleep(std::time::Duration::from_millis(50));
        let mut f_new = std::fs::File::create(&sub_ridx).unwrap();
        w_new.write_to(&mut f_new).unwrap();
        drop(f_new);
        assert!(mr_reload.reload_all_if_modified());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_source_hash_for_file_and_new_with_source_hash() {
        use crate::overlay::DocumentOverlay;
        use ridx_format::{normalized_source_hash, IndexWriter};
        let temp_dir = std::env::temp_dir().join(format!("ridx_mr_compiled_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let src_file = temp_dir.join("lib.rs");
        let src_str = src_file.to_str().unwrap().to_string();
        let indexed = "pub fn answer() -> i32 { 42 }\n";
        let expected = normalized_source_hash(indexed.as_bytes());

        let mut writer = IndexWriter::new("compiled_crate", "0.1.0");
        writer.get_or_insert_file(&src_str);
        writer.add_file_hash(&src_str, expected);
        writer.write_to(&mut std::fs::File::create(temp_dir.join("compiled.ridx")).unwrap()).unwrap();
        let mut mr = MultiRootWorkspace::new();
        assert!(mr.add_root(temp_dir.clone()));
        assert_eq!(mr.source_hash_for_file(&src_str), Some(expected));

        let shifted = format!("{}{}", "\n".repeat(10), indexed);
        std::fs::write(&src_file, indexed).unwrap();
        let ov = DocumentOverlay::new_with_source_hash("file:///a.rs", &src_str, 1, shifted.clone(), mr.source_hash_for_file(&src_str));
        assert_eq!((ov.compiled_text.as_deref(), ov.disk_text.as_deref()), (Some(indexed), Some(indexed)));

        let diverged = "pub fn diverged() {}\n";
        std::fs::write(&src_file, diverged).unwrap();
        let ov2 = DocumentOverlay::new_with_source_hash("file:///a.rs", &src_str, 1, indexed.to_string(), mr.source_hash_for_file(&src_str));
        assert_eq!((ov2.compiled_text.as_deref(), ov2.disk_text.as_deref()), (Some(indexed), Some(diverged)));

        let ov3 = DocumentOverlay::new_with_source_hash("file:///a.rs", &src_str, 1, diverged.to_string(), mr.source_hash_for_file(&src_str));
        assert_eq!((ov3.compiled_text.as_deref(), ov3.disk_text.as_deref()), (None, Some(diverged)));
        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
