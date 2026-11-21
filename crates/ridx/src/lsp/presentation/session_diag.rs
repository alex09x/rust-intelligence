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
//! and never saved is reported, in the buffer's coordinates (prod-code, m=20120). A file has them only for a text the
//! session is known to have compiled: a buffer, or a file on disk not written since the session started.

use super::compiler_message::parse_compiler_message_at;
use crate::indexer::session_report::CompilerReport;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

/// A file's diagnostics from a session, the text they were reported on, and when the session started (microseconds
/// since the epoch), to tell it from a check of the root started later.
#[derive(Clone, Debug, PartialEq)]
pub struct SessionFile {
    pub diagnostics: Vec<Value>,
    pub text: String,
    pub started_us: u64,
}

/// A check that reported on a file: its start (µs since the epoch), and the hash of the text it checked if known.
pub(crate) type CheckOf = (u64, Option<u64>);

pub(crate) fn micros(t: SystemTime) -> u64 {
    t.duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_micros() as u64)
}

/// Per root whose session index answers, its files (canonical) with their diagnostics; and the URI each file's
/// diagnostics were last published under, to clear them under that one.
#[derive(Default)]
pub struct SessionDiagnostics {
    roots: Mutex<HashMap<PathBuf, HashMap<PathBuf, SessionFile>>>,
    published: Mutex<HashMap<PathBuf, String>>,
}

type Roots = HashMap<PathBuf, HashMap<PathBuf, SessionFile>>;

/// The session's diagnostics of `file` (canonical) in `roots`: from the innermost root whose session has the file.
fn of_in<'a>(roots: &'a Roots, file: &Path) -> Option<&'a SessionFile> {
    roots.iter().filter(|(r, files)| file.starts_with(r) && files.contains_key(file)).max_by_key(|(r, _)| r.as_os_str().len()).map(|(_, files)| &files[file])
}

/// Whether what a client is sent of a file changes from `before` to `after` (the session's result for it, if any):
/// other diagnostics, or the same ones on another text, which places them elsewhere in the buffer (review of
/// 68375e6e); a result coming or going matters when it has diagnostics or the check (`has_check`) has some instead.
fn differs(before: Option<&SessionFile>, after: Option<&SessionFile>, has_check: bool) -> bool {
    match (before, after) {
        (Some(b), Some(a)) => b.diagnostics != a.diagnostics || (b.text != a.text && !a.diagnostics.is_empty()),
        (Some(x), None) | (None, Some(x)) => !x.diagnostics.is_empty() || has_check,
        (None, None) => false,
    }
}

impl SessionDiagnostics {
    /// Applies `change` to the roots and returns the files of `keys` whose result changes.
    fn change(&self, keys: HashSet<PathBuf>, has_check: &dyn Fn(&Path) -> bool, change: impl FnOnce(&mut Roots)) -> Vec<PathBuf> {
        let mut roots = self.roots.lock().unwrap_or_else(|e| e.into_inner());
        let before: Vec<(PathBuf, Option<SessionFile>)> = keys.into_iter().map(|k| (of_in(&roots, &k).cloned(), k)).map(|(b, k)| (k, b)).collect();
        change(&mut roots);
        let mut out: Vec<PathBuf> = before.into_iter().filter(|(k, b)| differs(b.as_ref(), of_in(&roots, k), has_check(k))).map(|(k, _)| k).collect();
        out.sort();
        out
    }

    /// The session of `root` now answers with `files`: returns the files whose result changes.
    pub fn replace(&self, root: &Path, files: HashMap<PathBuf, SessionFile>, has_check: &dyn Fn(&Path) -> bool) -> Vec<PathBuf> {
        let old: HashSet<PathBuf> = self.roots.lock().unwrap_or_else(|e| e.into_inner()).get(root).map(|f| f.keys().cloned().collect()).unwrap_or_default();
        let keys = old.into_iter().chain(files.keys().cloned()).collect();
        self.change(keys, has_check, |roots| {
            roots.insert(root.to_path_buf(), files);
        })
    }

    /// The session of `root` no longer answers: returns the files whose result changes, back to the check's.
    pub fn clear(&self, root: &Path, has_check: &dyn Fn(&Path) -> bool) -> Vec<PathBuf> {
        let keys = self.roots.lock().unwrap_or_else(|e| e.into_inner()).get(root).map(|f| f.keys().cloned().collect()).unwrap_or_default();
        self.change(keys, has_check, |roots| {
            roots.remove(root);
        })
    }

    /// The roots with a session's diagnostics.
    pub fn roots(&self) -> Vec<PathBuf> {
        self.roots.lock().unwrap_or_else(|e| e.into_inner()).keys().cloned().collect()
    }

    /// The session's diagnostics of `file` (canonical), from the innermost root whose session has the file.
    pub fn of(&self, file: &Path) -> Option<SessionFile> {
        of_in(&self.roots.lock().unwrap_or_else(|e| e.into_inner()), file).cloned()
    }

    /// What the session says of `file`, for a cache of what was computed from it (`workspace_diag`).
    pub fn stamp(&self, file: &Path) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.of(file).map(|f| (serde_json::to_string(&f.diagnostics).unwrap_or_default(), f.text, f.started_us)).hash(&mut h);
        h.finish()
    }

    fn published_uri(&self, file: &Path) -> Option<String> {
        self.published.lock().unwrap_or_else(|e| e.into_inner()).get(file).cloned()
    }

    /// `file`'s diagnostics are out under `uri` (`active`), or no longer under any: an alias is kept only while there is
    /// something to clear under it (review of 68375e6e: a cleared alias was used again after it was gone).
    fn record_published(&self, file: &Path, uri: &str, active: bool) {
        let mut published = self.published.lock().unwrap_or_else(|e| e.into_inner());
        if active {
            published.insert(file.to_path_buf(), uri.to_string());
        } else {
            published.remove(file);
        }
    }
}

/// The files of a session of `root`, from the compiler's `report` and the `buffers` compiled (path and text): each
/// file the compiler checked to the end, with its diagnostics or none, and each other one it reported on. A file's text
/// is its buffer's, or the file on disk when not written since the session started: a later text is not the one the
/// diagnostics were reported on, and its file gets none (review of 2e1fcfb1).
pub fn session_files(root: &Path, report: &CompilerReport, buffers: &[(PathBuf, String)]) -> HashMap<PathBuf, SessionFile> {
    let started_us = report.started.map_or(0, micros);
    let buffers: HashMap<PathBuf, &String> = buffers.iter().map(|(p, t)| (ridx_format::canonicalize_best_effort(p), t)).collect();
    let text_of = |path: &Path| -> Option<String> {
        if let Some(t) = buffers.get(path) {
            return Some((*t).clone());
        }
        let text = std::fs::read_to_string(path).ok()?;
        let since = report.started?;
        (!super::flycheck::modified_since(path, since)).then_some(text)
    };
    let mut files: HashMap<PathBuf, SessionFile> = HashMap::new();
    for path in &report.checked {
        if let Some(text) = text_of(path) {
            files.insert(path.clone(), SessionFile { diagnostics: Vec::new(), text, started_us });
        }
    }
    let mut seen = HashSet::new();
    for m in &report.messages {
        let Some((path, diag)) = parse_compiler_message_at(m, root) else { continue };
        if !path.starts_with(root) {
            continue;
        }
        // A file two units compile (a library and its unit tests) reports its error twice; it is one.
        if !seen.insert((path.clone(), diag["range"].to_string(), diag["severity"].to_string(), diag["message"].to_string())) {
            continue;
        }
        if !files.contains_key(&path) {
            let Some(text) = text_of(&path) else { continue };
            files.insert(path.clone(), SessionFile { diagnostics: Vec::new(), text, started_us });
        }
        if let Some(f) = files.get_mut(&path) {
            f.diagnostics.push(diag);
        }
    }
    files
}

/// Sends the diagnostics of `files` again after a session changed them: pushed, each under the URI of its open buffer
/// with its version, else the URI they were last published under, else the file's; or for a client that pulls, a
/// refresh when it takes one.
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
    let session = &lifecycle.flycheck.session;
    for file in files {
        let (uri, version) = match open.iter().find(|(p, _, _)| p == file) {
            Some((_, uri, version)) => (uri.clone(), Some(*version)),
            None => (session.published_uri(file).unwrap_or_else(|| crate::lsp::protocol::path_to_uri(&file.to_string_lossy())), None),
        };
        super::reconcile::publish_reconciled_diagnostics(&uri, version, ov, &lifecycle.flycheck, writer)?;
        session.record_published(file, &uri, lifecycle.flycheck.has_active_diagnostics(&uri));
    }
    Ok(())
}

#[cfg(test)]
#[path = "session_diag_tests.rs"]
mod tests;
