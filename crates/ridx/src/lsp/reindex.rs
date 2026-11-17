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
}

/// Background reindexing per workspace root.
#[derive(Default)]
pub struct Reindexer {
    roots: Arc<Mutex<HashMap<PathBuf, RootState>>>,
    /// The roots whose run ended successfully since the server last looked.
    finished: Arc<Mutex<Vec<PathBuf>>>,
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
    pub fn index_if_missing(&self, root: &Path) {
        if needs_first_index(root) && !self.roots.lock().unwrap_or_else(|e| e.into_inner()).contains_key(root) {
            crate::lsp::logger::log_info(&format!("no index in {}: indexing it", root.display()));
            self.start(root, None);
        }
    }

    /// Reindexes `root` in the background for the file `changed` on disk (a save, or a watched-file event), when it
    /// has an index, unless the file lies in a build directory or a run that started after it was written already
    /// covers it: Zed reports a save as a watched-file change too, just before `didSave`.
    pub fn trigger_for_change(&self, root: &Path, changed: &Path) {
        if changed.strip_prefix(root).unwrap_or(changed).components().any(|c| c.as_os_str() == "target") {
            return;
        }
        // A save while the first index of `root` is built: one more run after it.
        let first_run = self.roots.lock().unwrap_or_else(|e| e.into_inner()).contains_key(root);
        if reindex_off() || !(root.join(MANIFEST).is_file() || first_run) {
            return;
        }
        let written = std::fs::metadata(changed).and_then(|m| m.modified()).ok();
        self.start(root, written);
    }

    /// Runs `ridx index root` in the background, for a change written at `written`.
    fn start(&self, root: &Path, written: Option<SystemTime>) {
        let Some(exe) = ridx_exe() else { return };
        {
            let mut roots = self.roots.lock().unwrap_or_else(|e| e.into_inner());
            let st = roots.entry(root.to_path_buf()).or_default();
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
            if ok {
                finished.lock().unwrap_or_else(|e| e.into_inner()).push(root.clone());
            }
            let mut roots = roots.lock().unwrap_or_else(|e| e.into_inner());
            let st = roots.entry(root.clone()).or_default();
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
    let s = root.to_string_lossy();
    let cached = [".cargo/registry/", ".cargo/git/", ".rustup/toolchains/"].iter().any(|c| s.contains(c));
    !reindex_off() && !cached && root.join("Cargo.toml").is_file() && !root.join(MANIFEST).is_file()
}

/// This server's own executable when it is `ridx` (a test binary runs no indexing).
fn ridx_exe() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    (exe.file_stem().and_then(|s| s.to_str()) == Some("ridx")).then_some(exe)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_only_an_indexed_workspace_is_reindexed_and_never_from_a_test_binary() {
        let dir = tempfile::tempdir().unwrap();
        let r = Reindexer::new();
        let src = dir.path().join("src/lib.rs");
        r.trigger_for_change(dir.path(), &src);
        assert!(r.roots.lock().unwrap().is_empty(), "no index, no run");
        std::fs::create_dir_all(dir.path().join("target/ridx")).unwrap();
        std::fs::write(dir.path().join("target/ridx/ridx_manifest.json"), "{}").unwrap();
        assert!(ridx_exe().is_none(), "the test binary is not ridx");
        r.trigger_for_change(dir.path(), &src);
        assert!(r.roots.lock().unwrap().is_empty());
    }

    #[test]
    fn test_watch_registration_names_the_sources_of_the_index() {
        let reg = watch_registration(serde_json::json!("ridx-refresh-1"));
        assert_eq!(reg["method"], "client/registerCapability");
        let r = &reg["params"]["registrations"][0];
        assert_eq!(r["method"], "workspace/didChangeWatchedFiles");
        let globs: Vec<&str> = r["registerOptions"]["watchers"].as_array().unwrap().iter().map(|w| w["globPattern"].as_str().unwrap()).collect();
        assert_eq!(globs, ["**/*.rs", "**/Cargo.toml", "**/Cargo.lock"]);
    }

    /// In Zed: a saved buffer reverted by `git checkout` kept "unused import" from the check of the saved text. A text
    /// not checked yet is checked once, whichever of a save and its watched-file event comes first, and whether or not
    /// the editor already reloaded or edited the buffer (review of 3902e41c: comparing with the buffer restarted the
    /// check after a save and further typing, and missed a checkout the editor reloaded before reporting it).
    #[test]
    fn test_each_text_of_a_source_is_checked_once() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let src = root.join("src/lib.rs");
        std::fs::create_dir_all(src.parent().unwrap()).unwrap();
        let fc = crate::lsp::presentation::flycheck::FlycheckState::new();
        std::fs::write(&src, "pub fn a() {}\n").unwrap();
        assert!(fc.mark_saved_for_check(&src), "a save: checked");
        assert!(!fc.mark_changed_for_check(&src), "its watched-file event after didSave: not again");
        assert!(fc.mark_saved_for_check(&src), "saved again: checked again, as rust-analyzer does");
        std::fs::write(&src, "pub fn a() { let y = 2; }\n").unwrap();
        assert!(fc.mark_changed_for_check(&src), "Zed's watched-file event before didSave");
        assert!(!fc.mark_saved_for_check(&src), "then its didSave: the check of this text is running");
        std::fs::write(&src, "pub fn a() { let x = 1; }\n").unwrap();
        assert!(fc.mark_changed_for_check(&src), "a checkout, reloaded into the buffer or not");
        std::fs::remove_file(&src).unwrap();
        assert!(fc.mark_changed_for_check(&src), "a removed file");
        assert!(!fc.mark_changed_for_check(&src));
        assert!(is_checked_source(root, &src));
        assert!(!is_checked_source(root, &root.join("target/ridx/sources/1.rs")), "build output");
        assert!(!is_checked_source(root, &root.join("Cargo.toml")), "not a source");
    }

    /// A picked-up reindex asks the client to fetch again what came from the old index, for each kind it can refresh.
    #[test]
    fn test_refresh_requests_follow_the_client_capabilities() {
        let methods = |workspace_caps: serde_json::Value| {
            let mut lc = crate::lsp::lifecycle::LifecycleManager::new();
            let init = serde_json::json!({ "params": { "capabilities": { "workspace": workspace_caps } } });
            lc.handle_initialize(&init).unwrap();
            lc.index_refresh_requests().iter().map(|r| r["method"].as_str().unwrap().to_string()).collect::<Vec<_>>()
        };
        let zed = serde_json::json!({ "inlayHint": { "refreshSupport": true }, "semanticTokens": { "refreshSupport": true }, "codeLens": { "refreshSupport": true } });
        assert_eq!(methods(zed), ["workspace/inlayHint/refresh", "workspace/semanticTokens/refresh", "workspace/codeLens/refresh"]);
        assert_eq!(methods(serde_json::json!({ "inlayHint": { "refreshSupport": true }, "codeLens": { "refreshSupport": false } })), ["workspace/inlayHint/refresh"]);
        assert!(methods(serde_json::json!({})).is_empty());
        let r = Reindexer::new();
        assert!(r.take_finished().is_empty());
        r.finished.lock().unwrap().extend([PathBuf::from("/w"), PathBuf::from("/v")]);
        assert_eq!(r.take_finished(), [PathBuf::from("/w"), PathBuf::from("/v")]);
        assert!(r.take_finished().is_empty(), "taken once");
    }

    /// In Zed on hl-order-book-server, never indexed, a click on `NativeExecutor` went nowhere: a Cargo workspace
    /// without an index gets its first one, not a folder without `Cargo.toml`, nor a library's sources in a cache.
    #[test]
    fn test_a_cargo_workspace_without_an_index_is_indexed() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        assert!(!needs_first_index(root), "no Cargo.toml");
        std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"w\"\n").unwrap();
        assert!(needs_first_index(root));
        let registry = root.join(".cargo/registry/src/index.crates.io-1/serde-1.0.0");
        std::fs::create_dir_all(&registry).unwrap();
        std::fs::write(registry.join("Cargo.toml"), "[package]\nname = \"serde\"\n").unwrap();
        assert!(!needs_first_index(&registry), "a dependency's sources");
        std::fs::create_dir_all(root.join("target/ridx")).unwrap();
        std::fs::write(root.join(MANIFEST), "{}").unwrap();
        assert!(!needs_first_index(root), "indexed");

        let r = Reindexer::new();
        r.index_if_missing(dir.path());
        assert!(r.running().is_empty(), "the test binary runs no indexing");
        r.roots.lock().unwrap().entry(PathBuf::from("/b")).or_default().running_since = Some(SystemTime::now());
        r.roots.lock().unwrap().entry(PathBuf::from("/a")).or_default().running_since = Some(SystemTime::now());
        r.roots.lock().unwrap().entry(PathBuf::from("/idle")).or_default();
        assert_eq!(r.running(), [PathBuf::from("/a"), PathBuf::from("/b")]);
    }
}
