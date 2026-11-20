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
//! published one until a newer published index loads, or the buffers are all back to their files. The server loop's
//! side is `session_loop`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

/// How long the buffers of a workspace stay unchanged before their crates are compiled again.
const QUIET: Duration = Duration::from_millis(250);

/// An edited buffer as compiled: its key in the overlay manager, its path, its text.
#[derive(Clone)]
pub struct Snapshot {
    pub key: String,
    pub path: PathBuf,
    pub text: String,
}

/// What the published index of a root was when a session started: its manifest's change time and length.
pub(crate) fn published_stamp(root: &Path) -> Option<(SystemTime, u64)> {
    std::fs::metadata(root.join("target/ridx/ridx_manifest.json")).ok().and_then(|m| Some((m.modified().ok()?, m.len())))
}

#[derive(Default)]
struct RootState {
    /// The last edit not yet compiled.
    changed_at: Option<Instant>,
    building: bool,
    /// Counts the changes of base: a session built before one is dropped.
    epoch: u64,
    /// The session directory answering, and the published index it was built on.
    current: Option<(PathBuf, Option<(SystemTime, u64)>)>,
    /// Whether the directories left by servers that ended were removed.
    swept: bool,
}

/// A session index built, for the server to load, with the exact texts it compiled.
pub struct SessionBuilt {
    pub root: PathBuf,
    pub dir: PathBuf,
    pub buffers: Vec<Snapshot>,
    pub stamp: Option<(SystemTime, u64)>,
    epoch: u64,
}

#[derive(Default)]
pub struct SessionIndexer {
    roots: Arc<Mutex<HashMap<PathBuf, RootState>>>,
    built: Arc<Mutex<Vec<SessionBuilt>>>,
    count: Arc<Mutex<u64>>,
}

impl SessionIndexer {
    /// A buffer of `root` was opened, changed or closed.
    pub fn note_change(&self, root: &Path) {
        if session_off() {
            return;
        }
        self.roots.lock().unwrap_or_else(|e| e.into_inner()).entry(root.to_path_buf()).or_default().changed_at = Some(Instant::now());
    }

    /// Starts the build of every root whose buffers have been quiet long enough, with `edited(root)`: the edited
    /// buffers of that root. Returns the roots with no edited buffer left whose session index answers or is being
    /// built: they go back to the published index, and a build in flight is dropped (review of ca0c1ec7: a buffer
    /// reverted or closed kept answering from its discarded text).
    pub fn start_due(&self, edited: impl Fn(&Path) -> Vec<Snapshot>) -> Vec<PathBuf> {
        let mut retire = Vec::new();
        let mut roots = self.roots.lock().unwrap_or_else(|e| e.into_inner());
        for (root, st) in roots.iter_mut() {
            if !st.changed_at.is_some_and(|t| t.elapsed() >= QUIET) {
                continue;
            }
            let buffers = edited(root);
            if st.building {
                // The edit waits for the build in flight, unless nothing is edited any more: then that build is dropped.
                if buffers.is_empty() {
                    st.changed_at = None;
                    st.epoch += 1;
                    retire.push(root.clone());
                }
                continue;
            }
            st.changed_at = None;
            if buffers.is_empty() {
                if st.current.is_some() || st.building {
                    st.epoch += 1;
                    retire.push(root.clone());
                }
                continue;
            }
            st.building = true;
            if !std::mem::replace(&mut st.swept, true) {
                super::session_loop::sweep_ended_servers(&root.join("target/ridx/sessions"));
            }
            let n = {
                let mut c = self.count.lock().unwrap_or_else(|e| e.into_inner());
                *c += 1;
                *c
            };
            let dir = root.join("target/ridx/sessions").join(format!("{}-{n}", std::process::id()));
            let (epoch, stamp) = (st.epoch, published_stamp(root));
            let (root, roots, built) = (root.clone(), Arc::clone(&self.roots), Arc::clone(&self.built));
            std::thread::spawn(move || {
                let started = Instant::now();
                let texts: Vec<(PathBuf, String)> = buffers.iter().map(|b| (b.path.clone(), b.text.clone())).collect();
                let result = crate::indexer::session::build_session_index(&root, &dir, &texts);
                let msg = match &result {
                    Ok(_) => format!("session index of {} ({} buffer(s)) in {:.0} ms", root.display(), buffers.len(), started.elapsed().as_secs_f64() * 1000.0),
                    Err(e) => format!("no session index of {}: {e}", root.display()),
                };
                crate::lsp::logger::log_info(&msg);
                if let Some(st) = roots.lock().unwrap_or_else(|e| e.into_inner()).get_mut(&root) {
                    st.building = false;
                }
                match result {
                    Ok(dir) => built.lock().unwrap_or_else(|e| e.into_inner()).push(SessionBuilt { root, dir, buffers, stamp, epoch }),
                    Err(_) => {
                        let _ = std::fs::remove_dir_all(&dir);
                    }
                }
            });
        }
        retire
    }

    /// The session indexes built since the last call on the current base; one built before a change of base is dropped.
    pub fn take_built(&self) -> Vec<SessionBuilt> {
        let built = std::mem::take(&mut *self.built.lock().unwrap_or_else(|e| e.into_inner()));
        let roots = self.roots.lock().unwrap_or_else(|e| e.into_inner());
        let (current, stale): (Vec<_>, Vec<_>) = built.into_iter().partition(|b| roots.get(&b.root).is_some_and(|st| st.epoch == b.epoch));
        for b in stale {
            let _ = std::fs::remove_dir_all(&b.dir);
        }
        current
    }

    /// `dir`, built on the published index `stamp`, now answers for `root`: the session directory it replaces goes.
    pub fn loaded(&self, root: &Path, dir: &Path, stamp: Option<(SystemTime, u64)>) {
        let previous = self.roots.lock().unwrap_or_else(|e| e.into_inner()).entry(root.to_path_buf()).or_default().current.replace((dir.to_path_buf(), stamp));
        if let Some((previous, _)) = previous.filter(|(p, _)| p != dir) {
            let _ = std::fs::remove_dir_all(previous);
        }
    }

    /// The published index of `root` changed (a reindex of this server's, `ridx index` by hand, a variant switch):
    /// builds in flight are dropped and the edited buffers compiled again on the new one, also when no session
    /// answered yet (review of ca0c1ec7: a first build's edit was lost). Returns whether a session index answered,
    /// which the published index now replaces.
    pub fn published_loaded(&self, root: &Path) -> bool {
        let mut roots = self.roots.lock().unwrap_or_else(|e| e.into_inner());
        let Some(st) = roots.get_mut(root) else { return false };
        st.epoch += 1;
        st.changed_at = Some(Instant::now());
        let Some((dir, _)) = st.current.take() else { return false };
        let _ = std::fs::remove_dir_all(dir);
        true
    }

    /// Ends the session index of `root` (no edited buffer left): whether one answered.
    pub fn retire(&self, root: &Path) -> bool {
        let mut roots = self.roots.lock().unwrap_or_else(|e| e.into_inner());
        let Some((dir, _)) = roots.get_mut(root).and_then(|st| st.current.take()) else { return false };
        let _ = std::fs::remove_dir_all(dir);
        true
    }

    /// The roots whose session index answers while their published index has changed since it was built, by anyone
    /// (review of ca0c1ec7: the session's copy of the manifest hid a `ridx index` run by hand).
    pub fn published_changed(&self) -> Vec<PathBuf> {
        let roots = self.roots.lock().unwrap_or_else(|e| e.into_inner());
        roots.iter().filter(|(root, st)| st.current.as_ref().is_some_and(|(_, stamp)| *stamp != published_stamp(root))).map(|(r, _)| r.clone()).collect()
    }

    /// For `ridx/status`: whether a session index of `root` is being built or waits to be, and whether one answers.
    pub fn state_of(&self, root: &Path) -> (bool, bool) {
        let roots = self.roots.lock().unwrap_or_else(|e| e.into_inner());
        roots.get(root).map_or((false, false), |st| (st.building || st.changed_at.is_some(), st.current.is_some()))
    }

    /// Removes this server's session directories.
    pub fn shutdown(&self) {
        for st in self.roots.lock().unwrap_or_else(|e| e.into_inner()).values_mut() {
            if let Some((dir, _)) = st.current.take() {
                let _ = std::fs::remove_dir_all(dir);
            }
        }
    }
}

/// `RIDX_SESSION_INDEX=0` turns session indexes off.
fn session_off() -> bool {
    std::env::var("RIDX_SESSION_INDEX").is_ok_and(|v| v == "0")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Review of ca0c1ec7: a change of base while no session answered yet lost the edit; with none left, the session
    /// index goes; one published by anyone else is noticed.
    #[test]
    fn test_sessions_follow_the_base_and_the_buffers() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let s = SessionIndexer::default();
        s.note_change(&root);
        assert!(!s.published_loaded(&root) && s.state_of(&root).0, "no session answered, the edit is still due");

        let session = root.join("target/ridx/sessions/1-1");
        std::fs::create_dir_all(&session).unwrap();
        s.loaded(&root, &session, published_stamp(&root));
        assert!(s.published_changed().is_empty(), "same published index");
        std::fs::create_dir_all(root.join("target/ridx")).unwrap();
        std::fs::write(root.join("target/ridx/ridx_manifest.json"), "{}").unwrap();
        assert_eq!(s.published_changed(), vec![root.clone()], "a manifest published by anyone");

        s.loaded(&root, &session, published_stamp(&root));
        s.note_change(&root);
        std::thread::sleep(QUIET + Duration::from_millis(30));
        assert_eq!(s.start_due(|_| Vec::new()), vec![root.clone()], "nothing edited any more: back to the published index");
        assert!(s.retire(&root) && !session.exists());
        s.note_change(&root);
        std::thread::sleep(QUIET + Duration::from_millis(30));
        assert!(s.start_due(|_| Vec::new()).is_empty(), "no session to retire");
    }
}
