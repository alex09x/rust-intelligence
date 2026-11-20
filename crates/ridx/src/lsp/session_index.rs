/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Answers on code typed since the last save. The index is built from the files on disk, so references, rename and
//! navigation missed what a buffer added (prod-code: two calls typed into a buffer, references 2 where
//! rust-analyzer has 4). Once a workspace's buffers have been quiet for `QUIET`, the crates holding them are compiled
//! again from the buffers' text into an index of this session (`indexer::session`), which then answers in place of the
//! published one until the next published index loads.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long the buffers of a workspace stay unchanged before their crates are compiled again.
const QUIET: Duration = Duration::from_millis(250);

#[derive(Default)]
struct RootState {
    /// The last edit not yet compiled.
    changed_at: Option<Instant>,
    building: bool,
    /// Counts the loads of the published index: a session built on the one before is dropped.
    epoch: u64,
    /// The session directory in use, removed when another replaces it.
    current: Option<PathBuf>,
    /// Whether the directories left by servers that ended were removed.
    swept: bool,
}

/// A session index built, for the server to load.
pub struct SessionBuilt {
    pub root: PathBuf,
    pub dir: PathBuf,
    epoch: u64,
}

#[derive(Default)]
pub struct SessionIndexer {
    roots: Arc<Mutex<HashMap<PathBuf, RootState>>>,
    built: Arc<Mutex<Vec<SessionBuilt>>>,
    count: Arc<Mutex<u64>>,
}

impl SessionIndexer {
    /// A buffer of `root` changed.
    pub fn note_change(&self, root: &Path) {
        if session_off() {
            return;
        }
        self.roots.lock().unwrap_or_else(|e| e.into_inner()).entry(root.to_path_buf()).or_default().changed_at = Some(Instant::now());
    }

    /// Starts the build of every root whose buffers have been quiet long enough, with `dirty_buffers(root)`: the
    /// edited buffers of that root, path and text.
    pub fn start_due(&self, dirty_buffers: impl Fn(&Path) -> Vec<(PathBuf, String)>) {
        let mut roots = self.roots.lock().unwrap_or_else(|e| e.into_inner());
        for (root, st) in roots.iter_mut() {
            if st.building || !st.changed_at.is_some_and(|t| t.elapsed() >= QUIET) {
                continue;
            }
            st.changed_at = None;
            let buffers = dirty_buffers(root);
            if buffers.is_empty() {
                continue;
            }
            st.building = true;
            if !std::mem::replace(&mut st.swept, true) {
                sweep_ended_servers(&root.join("target/ridx/sessions"));
            }
            let n = {
                let mut c = self.count.lock().unwrap_or_else(|e| e.into_inner());
                *c += 1;
                *c
            };
            let dir = root.join("target/ridx/sessions").join(format!("{}-{n}", std::process::id()));
            let epoch = st.epoch;
            let (root, roots, built) = (root.clone(), Arc::clone(&self.roots), Arc::clone(&self.built));
            std::thread::spawn(move || {
                let started = Instant::now();
                let result = crate::indexer::session::build_session_index(&root, &dir, &buffers);
                let msg = match &result {
                    Ok(_) => format!("session index of {} ({} buffer(s)) in {:.0} ms", root.display(), buffers.len(), started.elapsed().as_secs_f64() * 1000.0),
                    Err(e) => format!("no session index of {}: {e}", root.display()),
                };
                crate::lsp::logger::log_info(&msg);
                if let Some(st) = roots.lock().unwrap_or_else(|e| e.into_inner()).get_mut(&root) {
                    st.building = false;
                }
                match result {
                    Ok(dir) => built.lock().unwrap_or_else(|e| e.into_inner()).push(SessionBuilt { root, dir, epoch }),
                    Err(_) => {
                        let _ = std::fs::remove_dir_all(&dir);
                    }
                }
            });
        }
    }

    /// The session indexes built since the last call on the published index loaded now; one built on an older one
    /// is dropped.
    pub fn take_built(&self) -> Vec<SessionBuilt> {
        let built = std::mem::take(&mut *self.built.lock().unwrap_or_else(|e| e.into_inner()));
        let roots = self.roots.lock().unwrap_or_else(|e| e.into_inner());
        let (current, stale): (Vec<_>, Vec<_>) = built.into_iter().partition(|b| roots.get(&b.root).is_some_and(|st| st.epoch == b.epoch));
        for b in stale {
            let _ = std::fs::remove_dir_all(&b.dir);
        }
        current
    }

    /// `dir` now answers for `root`: the session directory it replaces goes.
    pub fn loaded(&self, root: &Path, dir: &Path) {
        let previous = self.roots.lock().unwrap_or_else(|e| e.into_inner()).entry(root.to_path_buf()).or_default().current.replace(dir.to_path_buf());
        if let Some(previous) = previous.filter(|p| p != dir) {
            let _ = std::fs::remove_dir_all(previous);
        }
    }

    /// The published index of `root` loaded again (a save): whether a session index was answering for it. The edited
    /// buffers are compiled again on top of the new one.
    pub fn published_loaded(&self, root: &Path) -> bool {
        let mut roots = self.roots.lock().unwrap_or_else(|e| e.into_inner());
        let Some(st) = roots.get_mut(root) else { return false };
        st.epoch += 1;
        let Some(dir) = st.current.take() else { return false };
        let _ = std::fs::remove_dir_all(dir);
        st.changed_at = Some(Instant::now());
        true
    }

    /// Removes this server's session directories.
    pub fn shutdown(&self) {
        for st in self.roots.lock().unwrap_or_else(|e| e.into_inner()).values_mut() {
            if let Some(dir) = st.current.take() {
                let _ = std::fs::remove_dir_all(dir);
            }
        }
    }
}

/// Notes an edit of a buffer for the session index of its workspace.
pub(crate) fn note_edit(session: &SessionIndexer, msg: &serde_json::Value, mr: &super::MultiRootWorkspace, reindex: &super::reindex::Reindexer) {
    if msg["method"] != "textDocument/didChange" {
        return;
    }
    let file = super::protocol::uri_to_path(msg["params"]["textDocument"]["uri"].as_str().unwrap_or(""));
    if let Some(root) = super::dispatch::workspace_root_of(mr, reindex, Path::new(&file)) {
        session.note_change(&root);
    }
}

/// One turn of the server loop: the published indexes of `finished` replace the sessions on them, sessions due start,
/// and the ones built are loaded. Returns whether an index was replaced, for the client to fetch again.
pub(crate) fn step(session: &SessionIndexer, finished: &[PathBuf], mr: &mut super::MultiRootWorkspace, ov: &mut crate::overlay::OverlayManager) -> bool {
    let mut replaced = false;
    for root in finished {
        if session.published_loaded(root) {
            if let Ok(ws) = ridx_format::WorkspaceIndex::open(root) {
                replaced |= replace(mr, ov, root, ws);
            }
        }
    }
    session.start_due(|root| edited_buffers(ov, root));
    for built in session.take_built() {
        let loaded = ridx_format::WorkspaceIndex::open(&built.dir).is_ok_and(|ws| replace(mr, ov, &built.root, ws));
        if loaded {
            session.loaded(&built.root, &built.dir);
            replaced = true;
        } else {
            let _ = std::fs::remove_dir_all(&built.dir);
        }
    }
    replaced
}

fn replace(mr: &mut super::MultiRootWorkspace, ov: &mut crate::overlay::OverlayManager, root: &Path, ws: ridx_format::WorkspaceIndex) -> bool {
    let Some(ws) = mr.replace_index(root, ws) else { return false };
    super::rebase_open_buffers(ws, ov);
    true
}

/// The open buffers under `root` whose text is not the file's on disk, with that text: every edited one, not only
/// those changed since the last session index (that one compiled the others from their buffers too).
fn edited_buffers(ov: &crate::overlay::OverlayManager, root: &Path) -> Vec<(PathBuf, String)> {
    let mut out = Vec::new();
    for file in ov.open_file_paths() {
        let path = ridx_format::canonicalize_best_effort(Path::new(&file));
        if !path.starts_with(root) || path.extension().is_none_or(|x| x != "rs") {
            continue;
        }
        let Some(o) = ov.get(&file) else { continue };
        if std::fs::read_to_string(&path).ok().as_deref() != Some(o.text.as_str()) {
            out.push((path, o.text.clone()));
        }
    }
    out.sort();
    out
}

/// Removes the session directories (`<pid>-<n>`) of servers no longer running: one killed, rather than shut down,
/// leaves its own behind.
fn sweep_ended_servers(sessions: &Path) {
    let me = std::process::id().to_string();
    for entry in std::fs::read_dir(sessions).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(pid) = name.split('-').next().filter(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())) else { continue };
        let running = pid == me || std::process::Command::new("kill").args(["-0", pid]).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success());
        if !running {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// `RIDX_SESSION_INDEX=0` turns session indexes off.
fn session_off() -> bool {
    std::env::var("RIDX_SESSION_INDEX").is_ok_and(|v| v == "0")
}
