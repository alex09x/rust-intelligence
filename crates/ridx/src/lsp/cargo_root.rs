/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Workspace folders the client sends, lifted to the root of their Cargo workspace.
//!
//! Helix, and any client that takes the nearest `Cargo.toml` as the root, opens a workspace member
//! as its own folder. Its index lives under the workspace's `target/`, and its dependencies are the
//! other members, so the server works on the workspace the member belongs to, as `cargo` does.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The client's folders with each workspace member replaced by its workspace root, without duplicates.
pub fn workspace_roots(folders: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::with_capacity(folders.len());
    for folder in folders {
        let root = match cargo_workspace_root(&folder) {
            Some(root) if root != folder => {
                super::logger::log_info(&format!("{} is a member of the Cargo workspace {}; using the workspace", folder.display(), root.display()));
                root
            }
            _ => folder,
        };
        if !out.contains(&root) {
            out.push(root);
        }
    }
    out
}

/// The directory of the workspace `Cargo.toml` that `dir/Cargo.toml` belongs to, as `cargo` resolves it.
/// `None` when `dir` is not absolute (an empty URI must not pick up the server's own directory), has no
/// manifest, or `cargo` cannot answer; the folder is then used as it is.
pub fn cargo_workspace_root(dir: &Path) -> Option<PathBuf> {
    if !dir.is_absolute() {
        return None;
    }
    let manifest = dir.join("Cargo.toml");
    if !manifest.is_file() {
        return None;
    }
    let output = Command::new("cargo").args(["locate-project", "--workspace", "--message-format", "plain", "--manifest-path"]).arg(&manifest).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let located = PathBuf::from(String::from_utf8(output.stdout).ok()?.trim());
    let root = located.parent()?;
    if ridx_format::canonicalize_best_effort(root) == ridx_format::canonicalize_best_effort(dir) {
        return Some(dir.to_path_buf());
    }
    Some(root.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn workspace() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path();
        write(&ws.join("Cargo.toml"), "[workspace]\nmembers = [\"core\", \"app\"]\nresolver = \"2\"\n");
        for name in ["core", "app"] {
            write(&ws.join(name).join("Cargo.toml"), &format!("[package]\nname = \"{name}_x\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"));
            write(&ws.join(name).join("src/lib.rs"), "");
        }
        tmp
    }

    #[test]
    fn member_folder_is_lifted_to_its_workspace() {
        let tmp = workspace();
        let ws = ridx_format::canonicalize_best_effort(tmp.path());
        let root = cargo_workspace_root(&tmp.path().join("app")).expect("cargo locates the workspace");
        assert_eq!(ridx_format::canonicalize_best_effort(&root), ws);
    }

    #[test]
    fn workspace_root_stays_and_members_collapse_into_it() {
        let tmp = workspace();
        let ws = tmp.path().to_path_buf();
        let roots = workspace_roots(vec![ws.join("app"), ws.clone(), ws.join("core")]);
        assert_eq!(roots.len(), 1, "{roots:?}");
        assert_eq!(ridx_format::canonicalize_best_effort(&roots[0]), ridx_format::canonicalize_best_effort(&ws));
    }

    #[test]
    fn folder_without_manifest_is_kept() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("notes");
        fs::create_dir_all(&dir).unwrap();
        assert_eq!(cargo_workspace_root(&dir), None);
        assert_eq!(workspace_roots(vec![dir.clone()]), vec![dir]);
    }

    #[test]
    fn added_member_folder_is_indexed_as_its_workspace() {
        let tmp = workspace();
        let member = tmp.path().join("app");
        let msg = serde_json::json!({ "params": { "event": {
            "added": [{ "uri": format!("file://{}", member.display()), "name": "app" }], "removed": []
        } } });
        let roots = super::super::workspace::MultiRootWorkspace::new().handle_did_change_workspace_folders(&msg);
        assert_eq!(roots.len(), 1, "{roots:?}");
        assert_eq!(ridx_format::canonicalize_best_effort(&roots[0]), ridx_format::canonicalize_best_effort(tmp.path()));
    }

    #[test]
    fn relative_or_empty_folder_is_not_resolved_against_the_current_directory() {
        assert_eq!(cargo_workspace_root(Path::new("")), None);
        assert_eq!(cargo_workspace_root(Path::new(".")), None);
    }
}
