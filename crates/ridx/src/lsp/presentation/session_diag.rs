/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! The compiler's diagnostics of the buffers a session index compiled (`indexer::session`): an error typed and never
//! saved is reported, in the buffer's coordinates (prod-code, m=20120). They are kept for the buffers only, each with
//! the text compiled, and stand in for the check of the saved files (`flycheck`) for those buffers while a session has
//! them; every other file keeps the check's (review of 710381e0: what a closed file's text and inputs were could not be
//! told for sure).

use super::compiler_message::parse_compiler_message_at;
use crate::indexer::session_report::CompilerReport;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// A buffer's diagnostics from a session, and the text they were reported on.
#[derive(Clone, Debug, PartialEq)]
pub struct SessionFile {
    pub diagnostics: Vec<Value>,
    pub text: String,
}

type Roots = HashMap<PathBuf, HashMap<PathBuf, SessionFile>>;

/// Per root whose session index answers, its buffers (canonical) with their diagnostics; and the URI each file's
/// diagnostics were last published under, to clear them under that one.
#[derive(Default)]
pub struct SessionDiagnostics {
    roots: Mutex<Roots>,
    published: Mutex<HashMap<PathBuf, String>>,
}

/// The root and the session's diagnostics of `file` (canonical) in `roots`: from the innermost root whose session has
/// the file.
fn of_in<'a>(roots: &'a Roots, file: &Path) -> Option<(&'a PathBuf, &'a SessionFile)> {
    roots.iter().filter(|(r, files)| file.starts_with(r) && files.contains_key(file)).max_by_key(|(r, _)| r.as_os_str().len()).map(|(r, files)| (r, &files[file]))
}

/// Whether what a client is sent of a file changes from `before` to `after` (the session's result for it and the root
/// giving it, if any): other diagnostics, or the same ones on another text, which places them elsewhere in the buffer;
/// with the check having diagnostics of the file (`has_check`), any other result or root (reviews of 68375e6e,
/// 46ce96f2 and 710381e0); a result coming or going matters when it has diagnostics or the check has some.
fn differs(before: Option<(&PathBuf, &SessionFile)>, after: Option<(&PathBuf, &SessionFile)>, has_check: bool) -> bool {
    match (before, after) {
        (Some((br, b)), Some((ar, a))) => b.diagnostics != a.diagnostics || ((b.text != a.text || br != ar) && (has_check || !a.diagnostics.is_empty())),
        (Some((_, x)), None) | (None, Some((_, x))) => !x.diagnostics.is_empty() || has_check,
        (None, None) => false,
    }
}

impl SessionDiagnostics {
    /// Applies `change` to the roots and returns the files of `keys` whose result changes.
    fn change(&self, keys: HashSet<PathBuf>, has_check: &dyn Fn(&Path) -> bool, change: impl FnOnce(&mut Roots)) -> Vec<PathBuf> {
        let mut roots = self.roots.lock().unwrap_or_else(|e| e.into_inner());
        let before: Vec<(PathBuf, Option<(PathBuf, SessionFile)>)> =
            keys.into_iter().map(|k| (of_in(&roots, &k).map(|(r, f)| (r.clone(), f.clone())), k)).map(|(b, k)| (k, b)).collect();
        change(&mut roots);
        let mut out: Vec<PathBuf> = before.into_iter().filter(|(k, b)| differs(b.as_ref().map(|(r, f)| (r, f)), of_in(&roots, k), has_check(k))).map(|(k, _)| k).collect();
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

    /// Drops the results of the files `keep` does not keep: a buffer closed is a file as saved, which the check speaks
    /// for, also while a session of the root's other buffers answers or a build of it is in flight (review of
    /// fb2bed09). Returns the files whose result changes.
    pub fn keep_only(&self, keep: &dyn Fn(&Path) -> bool, has_check: &dyn Fn(&Path) -> bool) -> Vec<PathBuf> {
        let gone: HashSet<PathBuf> = self.roots.lock().unwrap_or_else(|e| e.into_inner()).values().flat_map(|f| f.keys()).filter(|p| !keep(p)).cloned().collect();
        if gone.is_empty() {
            return Vec::new();
        }
        let drop = gone.clone();
        self.change(gone, has_check, |roots| roots.values_mut().for_each(|files| files.retain(|p, _| !drop.contains(p))))
    }

    /// The roots with a session's diagnostics.
    pub fn roots(&self) -> Vec<PathBuf> {
        self.roots.lock().unwrap_or_else(|e| e.into_inner()).keys().cloned().collect()
    }

    /// The session's diagnostics of `file` (canonical), from the innermost root whose session has the file.
    pub fn of(&self, file: &Path) -> Option<SessionFile> {
        of_in(&self.roots.lock().unwrap_or_else(|e| e.into_inner()), file).map(|(_, f)| f.clone())
    }

    /// What the session says of `file`, for a cache of what was computed from it (`workspace_diag`).
    pub fn stamp(&self, file: &Path) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.of(file).map(|f| (serde_json::to_string(&f.diagnostics).unwrap_or_default(), f.text)).hash(&mut h);
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

/// Whether the check of the saved files has diagnostics of `file`, under the URI it names files by.
pub fn check_has(flycheck: &super::flycheck::FlycheckState, file: &Path) -> bool {
    !flycheck.get_latest_diagnostics(&super::compiler_merge::check_key(file)).is_empty()
}

/// The buffers of a session of `root`, from the compiler's `report` and the `buffers` compiled (path and text): each
/// one every unit owning it checked to the end, with its diagnostics or none, and each one it reported on although a
/// unit stopped before the end (a parse error), with what it reported. A buffer owned by a unit whose paths cannot be
/// told gets nothing (review of fb2bed09); diagnostics of other files are left to the check.
pub fn session_files(root: &Path, report: &CompilerReport, buffers: &[(PathBuf, String)]) -> HashMap<PathBuf, SessionFile> {
    let buffers: HashMap<PathBuf, &String> = buffers.iter().map(|(p, t)| (ridx_format::canonicalize_best_effort(p), t)).filter(|(p, _)| !report.untold.contains(p)).collect();
    let mut files: HashMap<PathBuf, SessionFile> = HashMap::new();
    for (path, text) in &buffers {
        if report.checked.contains(path) && !report.unchecked.contains(path) {
            files.insert(path.clone(), SessionFile { diagnostics: Vec::new(), text: (*text).clone() });
        }
    }
    let mut seen = HashSet::new();
    for m in &report.messages {
        let Some((path, diag)) = parse_compiler_message_at(m, root) else { continue };
        let Some(text) = buffers.get(&path) else { continue };
        // A file two units compile (a library and its unit tests) reports its error twice; it is one.
        if !seen.insert((path.clone(), diag["range"].to_string(), diag["severity"].to_string(), diag["message"].to_string())) {
            continue;
        }
        files.entry(path).or_insert_with(|| SessionFile { diagnostics: Vec::new(), text: (*text).clone() }).diagnostics.push(diag);
    }
    files
}

/// The buffer `uri` (`file`) was closed: it is the file as saved again, which the check of the saved files speaks for
/// (review of fb2bed09). The session's result of it goes, and the document is published again whatever changed: what
/// its buffer alone had (an unbalanced brace typed) goes too (review of d4541e29).
pub fn on_close<W: Write>(uri: &str, file: &str, lifecycle: &crate::lsp::lifecycle::LifecycleManager, ov: &crate::overlay::OverlayManager, writer: &mut W) -> std::io::Result<()> {
    let closed = ridx_format::canonicalize_best_effort(Path::new(file));
    let fc = &lifecycle.flycheck;
    fc.session.keep_only(&|p| p != closed, &|p| check_has(fc, p));
    if lifecycle.is_shutdown() {
        return Ok(());
    }
    if lifecycle.client_pull_diagnostics() {
        return publish_session_files(std::slice::from_ref(&closed), lifecycle, ov, writer);
    }
    super::reconcile::publish_reconciled_diagnostics(uri, None, ov, fc, writer)?;
    fc.session.record_published(&closed, uri, fc.has_active_diagnostics(uri));
    Ok(())
}

/// Sends the diagnostics of `files` again after a session changed them: pushed, each under the URI of its open buffer
/// with its version, else the URI something of it is still published under, else the file's; or for a client that
/// pulls, a refresh when it takes one.
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
            // The URI something is still out under, else the file's own (review of 46ce96f2: an alias cleared by
            // another path was used again).
            None => {
                let out = session.published_uri(file).filter(|u| lifecycle.flycheck.has_active_diagnostics(u));
                (out.unwrap_or_else(|| crate::lsp::protocol::path_to_uri(&file.to_string_lossy())), None)
            }
        };
        super::reconcile::publish_reconciled_diagnostics(&uri, version, ov, &lifecycle.flycheck, writer)?;
        session.record_published(file, &uri, lifecycle.flycheck.has_active_diagnostics(&uri));
    }
    Ok(())
}

#[cfg(test)]
#[path = "session_diag_tests.rs"]
mod tests;
