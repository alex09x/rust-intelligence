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
    /// The opening of the document it was taken from (`DocumentOverlay::instance`).
    pub instance: u64,
}

/// What the published index of a root was when a session started: its manifest's change time and length.
pub(crate) fn published_stamp(root: &Path) -> Option<(SystemTime, u64)> {
    std::fs::metadata(root.join("target/ridx/ridx_manifest.json")).ok().and_then(|m| Some((m.modified().ok()?, m.len())))
}

struct RootState {
    /// This registration of the root, never reused: a build of a root forgotten and noted again is not this one's
    /// (review of 68375e6e: its epoch started from 0 again and an old build was taken).
    id: u64,
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

impl Default for RootState {
    fn default() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Self { id, changed_at: None, building: false, epoch: 0, current: None, swept: false }
    }
}

/// A session index built, for the server to load, with the exact texts it compiled.
pub struct SessionBuilt {
    pub root: PathBuf,
    pub dir: PathBuf,
    pub buffers: Vec<Snapshot>,
    pub stamp: Option<(SystemTime, u64)>,
    /// What the compiler reported on the texts compiled.
    pub report: crate::indexer::session_report::CompilerReport,
    epoch: u64,
    id: u64,
}

#[derive(Default)]
pub struct SessionIndexer {
    roots: Arc<Mutex<HashMap<PathBuf, RootState>>>,
    built: Arc<Mutex<Vec<SessionBuilt>>>,
    count: Arc<Mutex<u64>>,
    /// The roots with an index at the last `roots_removed`.
    known: Mutex<Vec<PathBuf>>,
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
                // Also a build done but not loaded yet is of a text nobody edits any more (review of 11310378).
                st.epoch += 1;
                if st.current.is_some() {
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
            let (epoch, id, stamp) = (st.epoch, st.id, published_stamp(root));
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
                if let Some(st) = roots.lock().unwrap_or_else(|e| e.into_inner()).get_mut(&root).filter(|st| st.id == id) {
                    st.building = false;
                    // A run of `ridx index` held the published index: tried again once the buffers are quiet again.
                    if result.as_ref().is_err_and(|e| e == crate::indexer::session::BUSY) {
                        st.changed_at.get_or_insert_with(Instant::now);
                    }
                }
                match result {
                    Ok(out) => {
                        built.lock().unwrap_or_else(|e| e.into_inner()).push(SessionBuilt { root, dir: out.dir, buffers, stamp, report: out.report, epoch, id });
                    }
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
        let (current, stale): (Vec<_>, Vec<_>) = built.into_iter().partition(|b| roots.get(&b.root).is_some_and(|st| st.id == b.id && st.epoch == b.epoch));
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

    /// The roots of `live` (the roots with an index now) were the last call's too; returns those of the last call `live`
    /// no longer has: removed.
    pub fn roots_removed(&self, live: &[PathBuf]) -> Vec<PathBuf> {
        let mut known = self.known.lock().unwrap_or_else(|e| e.into_inner());
        let removed = known.iter().filter(|k| !live.contains(k)).cloned().collect();
        *known = live.to_vec();
        removed
    }

    /// Forgets every root `keep` does not keep (a workspace folder removed), a build of it in flight or done included:
    /// its directory goes, and a build of it is dropped when it ends (reviews of 2e1fcfb1 and 46ce96f2). Returns those
    /// roots.
    pub fn forget_unless(&self, keep: impl Fn(&Path) -> bool) -> Vec<PathBuf> {
        let mut roots = self.roots.lock().unwrap_or_else(|e| e.into_inner());
        let gone: Vec<PathBuf> = roots.keys().filter(|r| !keep(r)).cloned().collect();
        for root in &gone {
            if let Some((dir, _)) = roots.remove(root).and_then(|st| st.current) {
                let _ = std::fs::remove_dir_all(dir);
            }
        }
        gone
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

    /// Review of 2e1fcfb1: a workspace folder removed kept its session.
    #[test]
    fn test_a_removed_root_takes_its_session_with_it() {
        let dir = tempfile::tempdir().unwrap();
        let (root, other) = (dir.path().join("w"), dir.path().join("v"));
        let session = root.join("target/ridx/sessions/1-1");
        std::fs::create_dir_all(&session).unwrap();
        let s = SessionIndexer::default();
        s.loaded(&root, &session, None);
        s.note_change(&other);
        assert!(s.roots_removed(std::slice::from_ref(&root)).is_empty(), "first look");
        assert!(s.roots_removed(std::slice::from_ref(&root)).is_empty(), "nothing gone");
        let removed = s.roots_removed(&[]);
        assert_eq!(removed, vec![root.clone()], "a root with an index no more; one never indexed is not removed");
        assert_eq!(s.forget_unless(|r| !removed.iter().any(|g| g == r)), vec![root.clone()]);
        assert!(!session.exists() && s.state_of(&root) == (false, false) && s.state_of(&other).0);
    }

    /// Review of 46ce96f2: a root removed while its first session built (none answering yet) kept that build.
    #[test]
    fn test_a_root_removed_during_its_first_build_drops_it() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("w");
        let s = SessionIndexer::default();
        s.note_change(&root);
        let old = s.roots.lock().unwrap()[&root].id;
        s.forget_unless(|_| false);
        s.note_change(&root);
        s.built.lock().unwrap().push(SessionBuilt {
            root: root.clone(),
            dir: dir.path().join("gone"),
            buffers: Vec::new(),
            stamp: None,
            report: Default::default(),
            epoch: 0,
            id: old,
        });
        assert!(s.take_built().is_empty());
    }

    /// Review of 68375e6e: a build of a root forgotten and noted again (its epoch back at 0) was taken.
    #[test]
    fn test_a_build_of_an_earlier_registration_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("w");
        let s = SessionIndexer::default();
        s.loaded(&root, &dir.path().join("w/target/ridx/sessions/1-1"), None);
        let id_of = |s: &SessionIndexer| s.roots.lock().unwrap()[&root].id;
        let old = id_of(&s);
        let built = |id: u64| SessionBuilt { root: root.clone(), dir: dir.path().join("gone"), buffers: Vec::new(), stamp: None, report: Default::default(), epoch: 0, id };
        s.forget_unless(|_| false);
        s.note_change(&root);
        assert_ne!(id_of(&s), old);
        s.built.lock().unwrap().push(built(old));
        assert!(s.take_built().is_empty(), "the old registration's build");
        s.built.lock().unwrap().push(built(id_of(&s)));
        assert_eq!(s.take_built().len(), 1, "this one's");
    }
}
