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

/// What a session's compile of a buffer gave, for the text compiled: its diagnostics (`None`), or why it gave none,
/// which an empty list would hide (prod-code, m=20178): `notChecked` (no unit owning it checked it to the end, a proc
/// macro that panicked), `pathsUntold` (a unit owning it remaps paths or reads an `@file`), `notInAnyCrate` (no unit
/// compiled includes it, a new file nothing declares: prod-code, m=20181).
#[derive(Clone, Debug, PartialEq)]
pub struct Outcome {
    pub text: String,
    pub unavailable: Option<&'static str>,
}

/// Per root whose session index answers, its buffers (canonical) with their diagnostics and the outcome of each buffer
/// compiled; and the URI each file's diagnostics were last published under, to clear them under that one.
#[derive(Default)]
pub struct SessionDiagnostics {
    roots: Mutex<Roots>,
    outcomes: Mutex<HashMap<PathBuf, HashMap<PathBuf, Outcome>>>,
    published: Mutex<HashMap<PathBuf, std::collections::BTreeSet<String>>>,
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

    /// The outcomes of the buffers the session of `root` now answering compiled.
    pub fn set_outcomes(&self, root: &Path, outcomes: HashMap<PathBuf, Outcome>) {
        self.outcomes.lock().unwrap_or_else(|e| e.into_inner()).insert(root.to_path_buf(), outcomes);
    }

    /// The outcome of the compile of buffer `file` (canonical) by the session of `root` exactly: a session of an
    /// enclosing workspace compiled it with other cfgs and dependencies (review of 7501440b).
    pub fn outcome_in(&self, root: &Path, file: &Path) -> Option<Outcome> {
        self.outcomes.lock().unwrap_or_else(|e| e.into_inner()).get(root).and_then(|o| o.get(file)).cloned()
    }

    /// The innermost root whose session compiled buffer `file` (canonical), with its outcome, and that root's result
    /// for the file: what is answered for it comes from the root whose outcome admits it (review of 43ac9527: an
    /// enclosing root's clean result was answered for an inner root's `notInAnyCrate`).
    pub fn outcome_root(&self, file: &Path) -> Option<(Outcome, Option<SessionFile>)> {
        let outcomes = self.outcomes.lock().unwrap_or_else(|e| e.into_inner());
        let (root, out) =
            outcomes.iter().filter(|(r, o)| file.starts_with(r) && o.contains_key(file)).max_by_key(|(r, _)| r.as_os_str().len()).map(|(r, o)| (r.clone(), o[file].clone()))?;
        drop(outcomes);
        let entry = self.roots.lock().unwrap_or_else(|e| e.into_inner()).get(&root).and_then(|files| files.get(file)).cloned();
        Some((out, entry))
    }

    /// The outcome of the compile of buffer `file` (canonical), from the innermost root whose session compiled it.
    pub fn outcome(&self, file: &Path) -> Option<Outcome> {
        let outcomes = self.outcomes.lock().unwrap_or_else(|e| e.into_inner());
        outcomes.iter().filter(|(r, o)| file.starts_with(r) && o.contains_key(file)).max_by_key(|(r, _)| r.as_os_str().len()).map(|(_, o)| o[file].clone())
    }

    /// The session of `root` no longer answers: returns the files whose result changes, back to the check's.
    pub fn clear(&self, root: &Path, has_check: &dyn Fn(&Path) -> bool) -> Vec<PathBuf> {
        self.outcomes.lock().unwrap_or_else(|e| e.into_inner()).remove(root);
        let keys = self.roots.lock().unwrap_or_else(|e| e.into_inner()).get(root).map(|f| f.keys().cloned().collect()).unwrap_or_default();
        self.change(keys, has_check, |roots| {
            roots.remove(root);
        })
    }

    /// Drops the results of the files `keep` does not keep: a buffer closed is a file as saved, which the check speaks
    /// for, also while a session of the root's other buffers answers or a build of it is in flight (review of
    /// fb2bed09). Returns the files whose result changes.
    pub fn keep_only(&self, keep: &dyn Fn(&Path) -> bool, has_check: &dyn Fn(&Path) -> bool) -> Vec<PathBuf> {
        self.outcomes.lock().unwrap_or_else(|e| e.into_inner()).values_mut().for_each(|o| o.retain(|p, _| keep(p)));
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

    /// The URIs something of `file` is still published under (`active` says which still show something), in order; the
    /// others are forgotten (review of 68375e6e: a cleared alias was used again after it was gone).
    fn published_uris(&self, file: &Path, active: &dyn Fn(&str) -> bool) -> Vec<String> {
        let mut published = self.published.lock().unwrap_or_else(|e| e.into_inner());
        let Some(uris) = published.get_mut(file) else { return Vec::new() };
        uris.retain(|u| active(u));
        uris.iter().cloned().collect()
    }

    /// `file`'s diagnostics are out under `uri` (`active`), or no longer: every URI of the file still showing something
    /// is kept, not only the last (review of c7b4013f: two aliases closed in turn lost the first).
    fn record_published(&self, file: &Path, uri: &str, active: bool) {
        let mut published = self.published.lock().unwrap_or_else(|e| e.into_inner());
        let uris = published.entry(file.to_path_buf()).or_default();
        if active {
            uris.insert(uri.to_string());
        } else {
            uris.remove(uri);
        }
    }
}

/// Publishes `file` under `target` (with `version`, an open buffer's) and clears it under every other URI something of
/// it is still published under: the one place a file's publication moves, whatever moves it (reviews of 88fe53d0,
/// 2810515d and c7b4013f). Only URIs known to be this file's are cleared, never a key the check names it by, which may
/// be another file's URI (`a%20b.rs`).
fn publish_under<W: Write>(
    file: &Path,
    target: &str,
    version: Option<i32>,
    lifecycle: &crate::lsp::lifecycle::LifecycleManager,
    ov: &crate::overlay::OverlayManager,
    writer: &mut W,
) -> std::io::Result<()> {
    let fc = &lifecycle.flycheck;
    super::reconcile::publish_reconciled_diagnostics(target, version, ov, fc, writer)?;
    for other in fc.session.published_uris(file, &|u| fc.has_active_diagnostics(u)).into_iter().filter(|u| u != target) {
        if fc.clear_published(&other) {
            let clear = json!({ "jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": { "uri": other, "diagnostics": [] } });
            crate::lsp::protocol::send_response(writer, &clear)?;
        }
        fc.session.record_published(file, &other, false);
    }
    fc.session.record_published(file, target, fc.has_active_diagnostics(target));
    Ok(())
}

/// The URI to publish closed `file` under: one something of it is still published under, else its own.
fn closed_target(file: &Path, fc: &super::flycheck::FlycheckState) -> String {
    fc.session.published_uris(file, &|u| fc.has_active_diagnostics(u)).into_iter().next().unwrap_or_else(|| crate::lsp::protocol::path_to_uri(&file.to_string_lossy()))
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

/// The outcome of a buffer no crate includes.
pub const UNLINKED: &str = "notInAnyCrate";

/// The hint for a file on disk no crate includes, the one rust-analyzer gives (`unlinked-file`), when a session found
/// `live` (the file as saved) in none: in place of an empty list, which reads as a file without errors (prod-code,
/// m=20185).
pub fn unlinked_hint(fc: &super::flycheck::FlycheckState, file: &Path, live: &str, saved: Option<&str>) -> Option<Value> {
    let out = fc.session.outcome(file)?;
    (out.unavailable == Some(UNLINKED) && out.text == live && saved == Some(live)).then(|| {
        json!({
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } },
            "severity": 4,
            "code": "unlinked-file",
            "source": "ridx",
            "message": "This file is not included in any crates, so ridx can't offer IDE services."
        })
    })
}

/// The outcome of each buffer of a session (`buffers` compiled, `files` from [`session_files`]): its diagnostics when it
/// has them, else why not.
pub fn session_outcomes(report: &CompilerReport, buffers: &[(PathBuf, String)], files: &HashMap<PathBuf, SessionFile>) -> HashMap<PathBuf, Outcome> {
    buffers
        .iter()
        .map(|(p, text)| {
            let path = ridx_format::canonicalize_best_effort(p);
            let unavailable = if files.contains_key(&path) {
                None
            } else if report.unresolved.contains(&path) {
                Some("targetUnresolved")
            } else if report.unheld.contains(&path) {
                Some(UNLINKED)
            } else if report.untold.contains(&path) {
                Some("pathsUntold")
            } else {
                Some("notChecked")
            };
            (path, Outcome { text: text.clone(), unavailable })
        })
        .collect()
}

/// Whether the compiler's diagnostics of an open buffer's text are known, for a request that waits for them
/// (`textDocument/diagnostic` with `minVersion`, prod-code m=20178: the answer before them read as clean).
#[derive(Debug, PartialEq)]
pub enum Readiness {
    /// Known, for this version of the buffer.
    Ready(i32),
    /// The compile of this version gave none, for this reason (see [`Outcome`]).
    Unavailable(i32, &'static str),
    /// Not known yet.
    Pending,
}

/// The [`Readiness`] of the diagnostics of `file` (the overlay key) of workspace `root`: ready once a session of that
/// workspace compiled the buffer's very text, edited or as saved (a buffer as saved is compiled for a request waiting for it, `SessionIndexer::want`). The
/// check of the saved files never stands in: that it ran says nothing of the text it read, of whether it failed before
/// the file, or of which workspace it was (review of 13eacef0). A file not open has no version and is ready.
pub fn readiness(fc: &super::flycheck::FlycheckState, ov: &crate::overlay::OverlayManager, file: &str, root: &Path) -> Readiness {
    let Some(o) = ov.get(file) else { return Readiness::Ready(0) };
    let path = ridx_format::canonicalize_best_effort(Path::new(file));
    match fc.session.outcome_in(root, &path) {
        Some(out) if out.text == o.text => match out.unavailable {
            None => Readiness::Ready(o.version),
            // A file on disk no crate includes is answered, with a hint saying so (`unlinked_hint`), as rust-analyzer
            // does; an edited one is refused, its edit validates nothing (prod-code, m=20185).
            // As saved by the text the answer reads too (`reconcile::saved_text`), not a copy taken at open: the hint is
            // given then, never an empty list (review of 43ac9527).
            Some(UNLINKED) if super::reconcile::saved_text(ov, file).as_deref() == Some(o.text.as_str()) => Readiness::Ready(o.version),
            Some(why) => Readiness::Unavailable(o.version, why),
        },
        // No session compiled this text yet (a session's result of another text answers for it until the next one).
        _ => Readiness::Pending,
    }
}

/// Publishes what the check of the saved files reported on the file its key `check_key` names (`file://` and the path
/// as it is, not a URI to decode: a file may be named `a%20b.rs`, review of 2810515d) under the URI the client knows
/// the file by: its open buffer's, else one something of it is still published under, else the file's own.
pub fn publish_check<W: Write>(check_key: &str, lifecycle: &crate::lsp::lifecycle::LifecycleManager, ov: &crate::overlay::OverlayManager, writer: &mut W) -> std::io::Result<()> {
    let file = ridx_format::canonicalize_best_effort(&super::compiler_merge::check_path(check_key));
    let open = ov
        .open_file_paths()
        .into_iter()
        .find_map(|k| (ridx_format::canonicalize_best_effort(Path::new(&k)) == file).then(|| ov.get(&k).map(|o| (o.uri.clone(), o.version))).flatten());
    let (target, version) = match open {
        Some((uri, version)) => (uri, Some(version)),
        None => (closed_target(&file, &lifecycle.flycheck), None),
    };
    publish_under(&file, &target, version, lifecycle, ov, writer)
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
    publish_under(&closed, uri, None, lifecycle, ov, writer)
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
    for file in files {
        let (uri, version) = match open.iter().find(|(p, _, _)| p == file) {
            Some((_, uri, version)) => (uri.clone(), Some(*version)),
            // A URI something is still out under, else the file's own (review of 46ce96f2: an alias cleared by another
            // path was used again).
            None => (closed_target(file, &lifecycle.flycheck), None),
        };
        publish_under(file, &uri, version, lifecycle, ov, writer)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "session_diag_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "session_ready_tests.rs"]
mod ready_tests;
