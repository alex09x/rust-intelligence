/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! The server loop's side of session indexes (`session_index`): which messages change the edited buffers, when a
//! session index replaces the published one and back, and the directories of servers that ended.

use super::presentation::flycheck::FlycheckState;
use super::session_index::{SessionIndexer, Snapshot};
use std::path::{Path, PathBuf};

/// Notes a buffer opened, changed or closed for the session index of its workspace (review of ca0c1ec7: a close, or an
/// edited buffer opened again after a restart, went unnoticed).
pub(crate) fn note_edit(session: &SessionIndexer, msg: &serde_json::Value, mr: &super::MultiRootWorkspace, reindex: &super::reindex::Reindexer) {
    if !matches!(msg["method"].as_str(), Some("textDocument/didChange" | "textDocument/didOpen" | "textDocument/didClose")) {
        return;
    }
    let file = super::protocol::uri_to_path(msg["params"]["textDocument"]["uri"].as_str().unwrap_or(""));
    if let Some(root) = super::dispatch::workspace_root_of(mr, reindex, Path::new(&file)) {
        session.note_change(&root);
    }
}

/// One turn of the server loop: a published index that changed (`finished` by this server's reindex, or by anyone)
/// replaces the session on it, roots with no edited buffer left go back to their published index, sessions due start,
/// and the ones built are loaded. Returns whether an index was replaced, for the client to fetch again; the files whose
/// compiler diagnostics a session changed (in `flycheck.session`) go to `touched`.
pub(crate) fn step(
    session: &SessionIndexer,
    finished: &[PathBuf],
    mr: &mut super::MultiRootWorkspace,
    ov: &mut crate::overlay::OverlayManager,
    flycheck: &FlycheckState,
    touched: &mut Vec<PathBuf>,
) -> bool {
    let diags = &flycheck.session;
    // Whether the check has diagnostics of a file, under the URI it names files by.
    let has_check = |p: &Path| !flycheck.get_latest_diagnostics(&format!("file://{}", p.display())).is_empty();
    let mut replaced = false;
    // A workspace folder removed takes its session with it (review of 2e1fcfb1: its diagnostics outlived it).
    // A root that had an index and has none any more was removed, also while its first session builds; a root with no
    // index yet (being indexed) is kept (review of 46ce96f2).
    let live: Vec<PathBuf> = mr.roots().map(Path::to_path_buf).collect();
    let removed = session.roots_removed(&live);
    session.forget_unless(|r| !removed.iter().any(|g| g == r));
    for root in diags.roots().into_iter().filter(|r| !live.contains(r)) {
        touched.extend(diags.clear(&root, &has_check));
    }
    for root in finished.iter().cloned().chain(session.published_changed()) {
        // The session built next, on the new published index, reports again.
        touched.extend(diags.clear(&root, &has_check));
        if session.published_loaded(&root) {
            replaced |= back_to_published(mr, ov, &root);
        }
    }
    for root in session.start_due(|root| edited_buffers(ov, root)) {
        touched.extend(diags.clear(&root, &has_check));
        if session.retire(&root) {
            replaced |= back_to_published(mr, ov, &root);
        }
    }
    for built in session.take_built() {
        let Some(ws) = ridx_format::WorkspaceIndex::open(&built.dir).ok().and_then(|ws| mr.replace_index(&built.root, ws)) else {
            let _ = std::fs::remove_dir_all(&built.dir);
            continue;
        };
        super::rebase_open_buffers(ws, ov);
        // The texts it compiled are its base, whatever the buffers hold now: one edited further during the build maps
        // through those edits (review of ca0c1ec7: the base stayed the saved text, under an index of the edited one).
        // Only a text the index records as compiled: a buffer whose crate stopped before analysis (a parse error) kept
        // the shard before, and its text stays uncompiled (review of 11310378: it read as covered).
        for b in &built.buffers {
            // And all the units holding the file agree: a library compiled from the buffer next to its unit tests kept
            // from before is not a compiled text (review of a21afbc0), as `rebase_open_buffers` holds too.
            let compiled = ws.units_agree_on_source(&b.key) && ws.source_hash_for_file(&b.key) == Some(ridx_format::normalized_source_hash(b.text.as_bytes()));
            if compiled && ov.get(&b.key).is_some() {
                ov.advance_compiled_baseline(&b.key, b.text.clone());
            }
        }
        let texts: Vec<(PathBuf, String)> = built.buffers.iter().map(|b| (b.path.clone(), b.text.clone())).collect();
        touched.extend(diags.replace(&built.root, super::presentation::session_diag::session_files(&built.root, &built.report, &texts), &has_check));
        session.loaded(&built.root, &built.dir, built.stamp);
        replaced = true;
    }
    replaced
}

fn back_to_published(mr: &mut super::MultiRootWorkspace, ov: &mut crate::overlay::OverlayManager, root: &Path) -> bool {
    let Some(ws) = ridx_format::WorkspaceIndex::open(root).ok().and_then(|ws| mr.replace_index(root, ws)) else { return false };
    super::rebase_open_buffers(ws, ov);
    true
}

/// The open buffers under `root` whose text is not the file's on disk: every edited one, not only those changed since
/// the last session index (that one compiled the others from their buffers too).
fn edited_buffers(ov: &crate::overlay::OverlayManager, root: &Path) -> Vec<Snapshot> {
    let mut out = Vec::new();
    for key in ov.open_file_paths() {
        let path = ridx_format::canonicalize_best_effort(Path::new(&key));
        if !path.starts_with(root) || path.extension().is_none_or(|x| x != "rs") {
            continue;
        }
        let Some(o) = ov.get(&key) else { continue };
        if std::fs::read_to_string(&path).ok().as_deref() != Some(o.text.as_str()) {
            out.push(Snapshot { text: o.text.clone(), key, path });
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// Removes the session directories (`<pid>-<n>`) of servers that ended: one killed, rather than shut down, leaves its
/// own behind. Only when the system says the process does not exist; a check that could not be made keeps the
/// directory (review of ca0c1ec7: a missing `kill` deleted a live server's session).
pub(crate) fn sweep_ended_servers(sessions: &Path) {
    let me = std::process::id().to_string();
    for entry in std::fs::read_dir(sessions).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some((pid, n)) = name.split_once('-') else { continue };
        let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
        if !digits(pid) || !digits(n) || pid == me {
            continue;
        }
        if process_ended(pid) {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// Whether `kill -0 <pid>` says there is no such process (ESRCH); `false` when it exists, is not ours (EPERM), or
/// the check failed.
fn process_ended(pid: &str) -> bool {
    std::process::Command::new("kill")
        .args(["-0", pid])
        .stdin(std::process::Stdio::null())
        .output()
        .is_ok_and(|o| !o.status.success() && String::from_utf8_lossy(&o.stderr).to_ascii_lowercase().contains("no such process"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_only_directories_of_ended_servers_are_swept() {
        let dir = tempfile::tempdir().unwrap();
        let mut alive = std::process::Command::new("sleep").arg("30").spawn().unwrap();
        let ended = {
            let mut c = std::process::Command::new("true").spawn().unwrap();
            let pid = c.id();
            c.wait().unwrap();
            pid
        };
        for name in [format!("{}-1", alive.id()), format!("{ended}-1"), format!("{}-2", std::process::id()), "notes".into(), "12x-1".into()] {
            std::fs::create_dir_all(dir.path().join(name)).unwrap();
        }
        sweep_ended_servers(dir.path());
        let left: std::collections::BTreeSet<String> = std::fs::read_dir(dir.path()).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert!(left.contains(&format!("{}-1", alive.id())), "a live server's session stays: {left:?}");
        assert!(!left.contains(&format!("{ended}-1")), "an ended one's goes: {left:?}");
        assert!(left.contains(&format!("{}-2", std::process::id())) && left.contains("notes") && left.contains("12x-1"), "{left:?}");
        let _ = alive.kill();
        let _ = alive.wait();
    }
}
