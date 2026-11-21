/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! The compiler's diagnostics on the text a session index compiled (`indexer::session`). For the files of a root
//! whose session answers they stand in for the last check's (`flycheck`), which read the files as saved: an error typed
//! and never saved is reported, in the buffer's coordinates (prod-code, m=20120).

use super::compiler_message::parse_compiler_message;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// A file's diagnostics from a session, and the text they were reported on.
#[derive(Clone, Debug, PartialEq)]
pub struct SessionFile {
    pub diagnostics: Vec<Value>,
    pub text: String,
}

/// Per root whose session index answers, its files (canonical) with their diagnostics.
#[derive(Default)]
pub struct SessionDiagnostics {
    roots: Mutex<HashMap<PathBuf, HashMap<PathBuf, SessionFile>>>,
}

impl SessionDiagnostics {
    /// The session of `root` now answers with `files`: returns the files whose diagnostics it set or took away.
    pub fn replace(&self, root: &Path, files: HashMap<PathBuf, SessionFile>) -> Vec<PathBuf> {
        let mut roots = self.roots.lock().unwrap_or_else(|e| e.into_inner());
        let old = roots.insert(root.to_path_buf(), files).unwrap_or_default();
        let mut touched: Vec<PathBuf> = old.into_keys().chain(roots[root].keys().cloned()).collect::<HashSet<_>>().into_iter().collect();
        touched.sort();
        touched
    }

    /// The session of `root` no longer answers: returns the files it had diagnostics for, which the check's are again.
    pub fn clear(&self, root: &Path) -> Vec<PathBuf> {
        let mut touched: Vec<PathBuf> = self.roots.lock().unwrap_or_else(|e| e.into_inner()).remove(root).map(|f| f.into_keys().collect()).unwrap_or_default();
        touched.sort();
        touched
    }

    /// The session's diagnostics of `file` (canonical), from the innermost root whose session has the file.
    pub fn of(&self, file: &Path) -> Option<SessionFile> {
        let roots = self.roots.lock().unwrap_or_else(|e| e.into_inner());
        roots.iter().filter(|(r, files)| file.starts_with(r) && files.contains_key(file)).max_by_key(|(r, _)| r.as_os_str().len()).map(|(_, files)| files[file].clone())
    }
}

/// The files of a session of `root`: every buffer it compiled (`buffers`, path and text), with no diagnostic when the
/// compiler reported none, since the check of the saved file does not speak for its text; and every other file of the
/// root the compiler reported on, with its text on disk, the one compiled.
pub fn session_files(root: &Path, messages: &[Value], buffers: &[(PathBuf, String)]) -> HashMap<PathBuf, SessionFile> {
    let mut files: HashMap<PathBuf, SessionFile> =
        buffers.iter().map(|(p, text)| (ridx_format::canonicalize_best_effort(p), SessionFile { diagnostics: Vec::new(), text: text.clone() })).collect();
    let mut seen = HashSet::new();
    for m in messages {
        let Some((uri, diag)) = parse_compiler_message(m, root) else { continue };
        let path = ridx_format::canonicalize_best_effort(Path::new(&crate::lsp::protocol::uri_to_path(&uri)));
        if !path.starts_with(root) {
            continue;
        }
        // A file two units compile (a library and its unit tests) reports its error twice; it is one.
        if !seen.insert((path.clone(), diag["range"].to_string(), diag["severity"].to_string(), diag["message"].to_string())) {
            continue;
        }
        if !files.contains_key(&path) {
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            files.insert(path.clone(), SessionFile { diagnostics: Vec::new(), text });
        }
        if let Some(f) = files.get_mut(&path) {
            f.diagnostics.push(diag);
        }
    }
    files
}

/// Sends the diagnostics of `files` again after a session changed them: pushed, each under the uri and version of its
/// open buffer, or for a client that pulls, a refresh when it takes one.
pub fn publish_session_files<W: Write>(
    files: &[PathBuf],
    lifecycle: &crate::lsp::lifecycle::LifecycleManager,
    ov: &crate::overlay::OverlayManager,
    writer: &mut W,
) -> std::io::Result<()> {
    if files.is_empty() || lifecycle.is_shutdown() {
        return Ok(());
    }
    if lifecycle.client_pull_diagnostics() {
        if lifecycle.client_refresh_diagnostics() {
            let refresh = json!({ "jsonrpc": "2.0", "method": "workspace/diagnostic/refresh", "id": lifecycle.next_refresh_id(), "params": {} });
            crate::lsp::protocol::send_response(writer, &refresh)?;
        }
        return Ok(());
    }
    let open: Vec<(PathBuf, String, i32)> =
        ov.open_file_paths().into_iter().filter_map(|key| ov.get(&key).map(|o| (ridx_format::canonicalize_best_effort(Path::new(&key)), o.uri.clone(), o.version))).collect();
    for file in files {
        let (uri, version) = match open.iter().find(|(p, _, _)| p == file) {
            Some((_, uri, version)) => (uri.clone(), Some(*version)),
            None => (format!("file://{}", file.display()), None),
        };
        super::reconcile::publish_reconciled_diagnostics(&uri, version, ov, &lifecycle.flycheck, writer)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "session_diag_tests.rs"]
mod tests;
