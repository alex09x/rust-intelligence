/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Atomic multi-file transaction publishing with permission and symlink preservation.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static TX_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Resolves real file referent if symlink, preserving the link itself during publication.
fn resolve_target(path_str: &str) -> (PathBuf, bool) {
    let p = Path::new(path_str);
    if let Ok(meta) = fs::symlink_metadata(p) {
        if meta.file_type().is_symlink() {
            if let Ok(canonical) = fs::canonicalize(p) {
                return (canonical, true);
            }
        }
    }
    (p.to_path_buf(), false)
}

/// Atomically commits closed file modifications to disk.
///
/// Ensures:
/// 1. Preflight writability checks on the real target (following symlinks).
/// 2. Exclusive creation of temporary staging files (`create_new(true)`) preventing clobbering.
/// 3. Preservation of original file permissions (modes, executable bits).
/// 4. Symlinks are preserved by publishing to the canonical referent.
/// 5. Metadata-only rename rollback without data loss on failure.
pub fn commit_closed_writes_atomic(closed_writes: &[(String, String)]) -> bool {
    if closed_writes.is_empty() {
        return true;
    }

    let mut resolved_targets = Vec::with_capacity(closed_writes.len());
    for (file_path, new_text) in closed_writes {
        let (real_path, _) = resolve_target(file_path);
        if real_path.exists() && fs::OpenOptions::new().write(true).open(&real_path).is_err() {
            return false;
        }
        let perms = fs::metadata(&real_path).ok().map(|m| m.permissions());
        resolved_targets.push((real_path, new_text, perms));
    }

    let pid = std::process::id();
    let tx_id = TX_COUNTER.fetch_add(1, Ordering::Relaxed);
    let now_nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);

    let mut tmp_files: Vec<(PathBuf, PathBuf)> = Vec::with_capacity(resolved_targets.len());

    for (i, (real_path, new_text, perms)) in resolved_targets.iter().enumerate() {
        let tmp_path = PathBuf::from(format!(
            "{}.ridx_tmp_{}_{}_{}_{}",
            real_path.display(),
            pid,
            now_nanos,
            tx_id,
            i
        ));

        let mut file = match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp_path)
        {
            Ok(f) => f,
            Err(_) => {
                for (_, tmp) in &tmp_files {
                    let _ = fs::remove_file(tmp);
                }
                return false;
            }
        };

        if file.write_all(new_text.as_bytes()).is_err() || file.flush().is_err() {
            drop(file);
            let _ = fs::remove_file(&tmp_path);
            for (_, tmp) in &tmp_files {
                let _ = fs::remove_file(tmp);
            }
            return false;
        }
        drop(file);

        if let Some(ref p) = perms {
            let _ = fs::set_permissions(&tmp_path, p.clone());
        }

        tmp_files.push((real_path.clone(), tmp_path));
    }

    let mut backups: Vec<(PathBuf, PathBuf)> = Vec::new();
    let mut publish_success = true;

    for (i, (real_path, tmp_path)) in tmp_files.iter().enumerate() {
        let bak_path = PathBuf::from(format!(
            "{}.ridx_bak_{}_{}_{}_{}",
            real_path.display(),
            pid,
            now_nanos,
            tx_id,
            i
        ));

        if real_path.exists() {
            if bak_path.exists() || fs::rename(real_path, &bak_path).is_err() {
                publish_success = false;
                break;
            }
            backups.push((real_path.clone(), bak_path));
        }

        if fs::rename(tmp_path, real_path).is_err() {
            publish_success = false;
            break;
        }
    }

    if !publish_success {
        for (real_path, bak_path) in backups {
            let _ = fs::rename(&bak_path, &real_path);
        }
        for (_, tmp_path) in &tmp_files {
            let _ = fs::remove_file(tmp_path);
        }
        return false;
    }

    for (_, bak_path) in backups {
        let _ = fs::remove_file(&bak_path);
    }

    true
}

/// Checks whether a path is an external read-only dependency (e.g. in cargo cache or toolchains).
pub fn is_readonly_dependency(path: &Path, workspace: Option<&ridx_format::WorkspaceIndex>) -> bool {
    if let Some(ws) = workspace {
        ws.is_external_path(path)
    } else {
        ridx_format::is_external_dependency_path(path)
    }
}

/// Validates that none of the URIs in a WorkspaceEdit target read-only external dependencies.
pub fn validate_workspace_edit(
    edit: &serde_json::Value,
    workspace: Option<&ridx_format::WorkspaceIndex>,
) -> crate::lsp::error::LspResult<()> {
    if let Some(changes) = edit.get("changes").and_then(|c| c.as_object()) {
        for uri in changes.keys() {
            let p = crate::lsp::protocol::uri_to_path(uri);
            if is_readonly_dependency(Path::new(&p), workspace) {
                return Err(crate::lsp::error::LspError::request_failed(format!(
                    "Cannot modify read-only external dependency: {p}"
                )));
            }
        }
    }

    if let Some(doc_changes) = edit.get("documentChanges").and_then(|c| c.as_array()) {
        for dc in doc_changes {
            if let Some(uri) = dc
                .get("textDocument")
                .and_then(|td| td.get("uri"))
                .and_then(|u| u.as_str())
            {
                let p = crate::lsp::protocol::uri_to_path(uri);
                if is_readonly_dependency(Path::new(&p), workspace) {
                    return Err(crate::lsp::error::LspError::request_failed(format!(
                        "Cannot modify read-only external dependency: {p}"
                    )));
                }
            } else if let Some(uri) = dc.get("uri").and_then(|u| u.as_str()) {
                let p = crate::lsp::protocol::uri_to_path(uri);
                if is_readonly_dependency(Path::new(&p), workspace) {
                    return Err(crate::lsp::error::LspError::request_failed(format!(
                        "Cannot modify read-only external dependency: {p}"
                    )));
                }
            }
        }
    }

    Ok(())
}

