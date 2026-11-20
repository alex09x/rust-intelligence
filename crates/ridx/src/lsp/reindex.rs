/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Reindexing on change. The index answers navigation, references and rename for what was compiled; an edit reaches
//! it once `ridx index` has run again, which the server then picks up by itself (the manifest changes on disk). So a
//! save in an indexed workspace, or a source file changed outside the editor (a checkout, a pull), starts
//! `ridx index <root>` in the background: one run per root at a time, the changes made while it runs folded into one
//! more run after it. A Cargo workspace opened without an index gets its first one the same way, as rust-analyzer
//! analyzes any project it opens. `RIDX_REINDEX_ON_SAVE=0` turns both off.

use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

/// Where a background run writes its output, under the workspace root.
const REINDEX_LOG: &str = "target/ridx/reindex.log";
/// What a finished `ridx index` publishes: a root without it has no index.
const MANIFEST: &str = "target/ridx/ridx_manifest.json";

#[derive(Default)]
struct RootState {
    /// When the current run started; `None` while idle.
    running_since: Option<SystemTime>,
    /// Another run was asked for while this one runs.
    again: bool,
    /// When the last run started: a file written before it is in the index.
    last_start: Option<SystemTime>,
    /// Why the last run failed, `None` after a run that succeeded (`ridx/status`).
    last_error: Option<String>,
    /// Which registration of the root this is: a run of one removed (and maybe added again) is not taken.
    registration: u64,
}

static REGISTRATIONS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Background reindexing per workspace root.
#[derive(Default)]
pub struct Reindexer {
    roots: Arc<Mutex<HashMap<PathBuf, RootState>>>,
    /// The roots whose run ended successfully since the server last looked.
    finished: Arc<Mutex<Vec<PathBuf>>>,
    /// Workspace folders from `initialize`, indexed once the client is `initialized` (its progress then shows).
    queued: Mutex<Vec<PathBuf>>,
}

impl Reindexer {
    pub fn new() -> Self {
        Self::default()
    }

    /// The roots whose run ended successfully since the last call: the server then loads the new index (a first one
    /// as a new root) and asks the client to fetch again what came from the old one (the watched-file events of a
    /// run arrive while it runs).
    pub fn take_finished(&self) -> Vec<PathBuf> {
        std::mem::take(&mut *self.finished.lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// A workspace folder the client removed: it leaves the roots of the session, with a run's success not yet taken
    /// (review of 2045537b: it loaded the removed workspace again).
    pub fn forget(&self, root: &Path) {
        let mut roots = self.roots.lock().unwrap_or_else(|e| e.into_inner());
        roots.remove(root);
        self.finished.lock().unwrap_or_else(|e| e.into_inner()).retain(|r| r != root);
    }

    /// The roots indexed this session, running or not, canonical.
    pub fn roots(&self) -> Vec<PathBuf> {
        self.roots.lock().unwrap_or_else(|e| e.into_inner()).keys().cloned().collect()
    }

    /// Records a failed run of `root` (tests of what the server answers then).
    #[cfg(test)]
    pub fn fail_for_test(&self, root: &Path, error: &str) {
        let mut roots = self.roots.lock().unwrap_or_else(|e| e.into_inner());
        roots.entry(root.to_path_buf()).or_default().last_error = Some(error.to_string());
    }

    /// For `ridx/status`: whether `root` is being indexed now, and why its last run failed (`None` after one that
    /// succeeded, or before any).
    pub fn status_of(&self, root: &Path) -> (bool, Option<String>) {
        let roots = self.roots.lock().unwrap_or_else(|e| e.into_inner());
        roots.get(root).map_or((false, None), |st| (st.running_since.is_some(), st.last_error.clone()))
    }

    /// The roots being indexed now, for the client's progress.
    pub fn running(&self) -> Vec<PathBuf> {
        let roots = self.roots.lock().unwrap_or_else(|e| e.into_inner());
        let mut running: Vec<PathBuf> = roots.iter().filter(|(_, st)| st.running_since.is_some()).map(|(r, _)| r.clone()).collect();
        running.sort();
        running
    }

    /// Builds the first index of `root` when it is a Cargo workspace without one, once per session. In Zed on
    /// hl-order-book-server, never indexed, a click on `NativeExecutor` went nowhere: the index answers navigation,
    /// and nothing built it.
    /// Roots are keyed canonical, as the workspace keys them (review of 30e62913: a folder given as `/var/…` and its
    /// files as `/private/var/…` were two roots).
    pub fn index_if_missing(&self, root: &Path) {
        let root = ridx_format::canonicalize_best_effort(root);
        if needs_first_index(&root) && !self.roots.lock().unwrap_or_else(|e| e.into_inner()).contains_key(&root) {
            crate::lsp::logger::log_info(&format!("no index in {}: indexing it", root.display()));
            self.start(&root, None);
        }
    }

    /// The roots `initialize` queued (the client's folders lifted to their workspaces, or the one of the current
    /// directory when it named none), not yet started.
    pub fn queued_roots(&self) -> Vec<PathBuf> {
        self.queued.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Keeps a workspace folder from `initialize` for `start_queued`.
    pub fn queue_first_index(&self, root: &Path) {
        self.queued.lock().unwrap_or_else(|e| e.into_inner()).push(root.to_path_buf());
    }

    /// Indexes the folders queued at `initialize` that have no index, now that the client is `initialized`, and
    /// returns all of them.
    pub fn start_queued(&self) -> Vec<PathBuf> {
        let queued = std::mem::take(&mut *self.queued.lock().unwrap_or_else(|e| e.into_inner()));
        for root in &queued {
            self.index_if_missing(root);
        }
        queued
    }

    /// The root indexed this session that holds `file` (canonical), the innermost: during a first run it is in no
    /// loaded workspace yet, and the nearest `Cargo.toml` above a file is its member crate's.
    pub fn root_for(&self, file: &Path) -> Option<PathBuf> {
        let roots = self.roots.lock().unwrap_or_else(|e| e.into_inner());
        roots.keys().filter(|r| file.starts_with(r)).max_by_key(|r| r.as_os_str().len()).cloned()
    }

    /// Reindexes `root` in the background for the file `changed` on disk (a save, or a watched-file event), when it
    /// has an index, unless the file lies in a build directory or a run that started after it was written already
    /// covers it: Zed reports a save as a watched-file change too, just before `didSave`.
    pub fn trigger_for_change(&self, root: &Path, changed: &Path) {
        let (root, changed) = (ridx_format::canonicalize_best_effort(root), ridx_format::canonicalize_best_effort(changed));
        if changed.strip_prefix(&root).unwrap_or(&changed).components().any(|c| c.as_os_str() == "target") {
            return;
        }
        // A root whose first index was asked for this session: a save while it runs is one more run after it, and a
        // save after a run that failed (a compile error, say) tries again.
        let first_index_asked = self.roots.lock().unwrap_or_else(|e| e.into_inner()).contains_key(&root);
        if reindex_off() || !(root.join(MANIFEST).is_file() || first_index_asked) {
            return;
        }
        let written = std::fs::metadata(&changed).and_then(|m| m.modified()).ok();
        self.start(&root, written);
    }

    /// Runs `ridx index root` in the background, for a change written at `written`.
    fn start(&self, root: &Path, written: Option<SystemTime>) {
        let Some(exe) = ridx_exe() else { return };
        let registration;
        {
            let mut roots = self.roots.lock().unwrap_or_else(|e| e.into_inner());
            let st = roots
                .entry(root.to_path_buf())
                .or_insert_with(|| RootState { registration: REGISTRATIONS.fetch_add(1, std::sync::atomic::Ordering::Relaxed), ..RootState::default() });
            registration = st.registration;
            let covered_since = st.running_since.or(st.last_start);
            if written.is_some_and(|w| covered_since.is_some_and(|s| w <= s)) {
                return;
            }
            if st.running_since.is_some() {
                st.again = true;
                return;
            }
            let now = SystemTime::now();
            st.running_since = Some(now);
            st.last_start = Some(now);
        }
        let roots = Arc::clone(&self.roots);
        let finished = Arc::clone(&self.finished);
        let root = root.to_path_buf();
        std::thread::spawn(move || loop {
            let started = std::time::Instant::now();
            // The run's output goes to `target/ridx/reindex.log`: why a crate did not build stays readable.
            let log = root.join(REINDEX_LOG);
            let _ = log.parent().map(std::fs::create_dir_all);
            let out = std::fs::File::create(&log).map_or_else(|_| Stdio::null(), Stdio::from);
            let ok = Command::new(&exe).arg("index").arg(&root).current_dir(&root).stdin(Stdio::null()).stdout(Stdio::null()).stderr(out).status().is_ok_and(|s| s.success());
            let msg = format!("reindexed {} in {:.1} s ({}; log {})", root.display(), started.elapsed().as_secs_f64(), if ok { "ok" } else { "failed" }, log.display());
            crate::lsp::logger::log_info(&msg);
            let mut roots = roots.lock().unwrap_or_else(|e| e.into_inner());
            // A folder the client removed meanwhile (`forget`), even if added again since, is neither loaded nor run again.
            let Some(st) = roots.get_mut(&root).filter(|st| st.registration == registration) else { break };
            if ok {
                finished.lock().unwrap_or_else(|e| e.into_inner()).push(root.clone());
            }
            st.last_error = (!ok).then(|| failure_reason(&log));
            if std::mem::take(&mut st.again) {
                let now = SystemTime::now();
                st.running_since = Some(now);
                st.last_start = Some(now);
            } else {
                st.running_since = None;
                break;
            }
        });
    }
}

/// Why a run failed: the last `error` line of its output (a missing `ridx-driver`, a manifest that does not parse), and
/// where the rest is (prod-code, m=19837: the reason showed only as "ridx index failed").
pub(crate) fn failure_reason(log: &Path) -> String {
    let text = std::fs::read_to_string(log).unwrap_or_default();
    let line = text.lines().rev().find(|l| l.trim_start().to_ascii_lowercase().starts_with("error")).or_else(|| text.lines().rev().find(|l| !l.trim().is_empty()));
    match line {
        Some(l) => format!("ridx index failed: {} (output in {})", l.trim().chars().take(300).collect::<String>(), log.display()),
        None => format!("ridx index failed; its output is in {}", log.display()),
    }
}

/// The `client/registerCapability` request asking the client to report changes of the files the index is built
/// from, as rust-analyzer registers them.
pub fn watch_registration(id: Value) -> Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "client/registerCapability",
        "params": { "registrations": [{
            "id": "ridx-watched-files",
            "method": "workspace/didChangeWatchedFiles",
            "registerOptions": { "watchers": [{ "globPattern": "**/*.rs" }, { "globPattern": "**/Cargo.toml" }, { "globPattern": "**/Cargo.lock" }] }
        }] }
    })
}

/// Whether a watched-file event for `file` under `root` is about a Rust source the compiler checks: not one in a build
/// directory (the index's own snapshots under `target/`).
pub fn is_checked_source(root: &Path, file: &Path) -> bool {
    file.extension().is_some_and(|e| e == "rs") && !file.strip_prefix(root).unwrap_or(file).components().any(|c| c.as_os_str() == "target")
}

/// The `workspace/*/refresh` requests a client announces in its workspace capabilities, sent once a reindex is
/// loaded: what it shows from the old index is fetched again only when it is told.
pub fn refresh_methods(workspace_caps: &Value) -> Vec<&'static str> {
    [("inlayHint", "workspace/inlayHint/refresh"), ("semanticTokens", "workspace/semanticTokens/refresh"), ("codeLens", "workspace/codeLens/refresh")]
        .into_iter()
        .filter(|(cap, _)| workspace_caps[cap]["refreshSupport"].as_bool() == Some(true))
        .map(|(_, method)| method)
        .collect()
}

fn reindex_off() -> bool {
    std::env::var("RIDX_REINDEX_ON_SAVE").is_ok_and(|v| v == "0")
}

/// Whether `root` is a Cargo workspace of the user's without an index: not one the toolchain or Cargo's caches hold
/// (a library's sources opened as a folder).
fn needs_first_index(root: &Path) -> bool {
    !reindex_off() && !is_cached_source(root) && root.join("Cargo.toml").is_file() && !root.join(MANIFEST).is_file()
}

/// Whether `path` lies in sources the toolchain or Cargo's caches hold, not the user's.
pub fn is_cached_source(path: &Path) -> bool {
    let s = path.to_string_lossy();
    [".cargo/registry/", ".cargo/git/", ".rustup/toolchains/"].iter().any(|c| s.contains(c))
}

/// This server's own executable when it is `ridx` (a test binary runs no indexing).
fn ridx_exe() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    (exe.file_stem().and_then(|s| s.to_str()) == Some("ridx")).then_some(exe)
}

#[cfg(test)]
#[path = "reindex_tests.rs"]
mod tests;
