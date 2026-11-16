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
//! more run after it. `RIDX_REINDEX_ON_SAVE=0` turns it off.

use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

/// Where a background run writes its output, under the workspace root.
const REINDEX_LOG: &str = "target/ridx/reindex.log";

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
}

impl Reindexer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reindexes `root` in the background for the file `changed` on disk (a save, or a watched-file event), when it
    /// has an index, unless the file lies in a build directory or a run that started after it was written already
    /// covers it: Zed reports a save as a watched-file change too, just before `didSave`.
    pub fn trigger_for_change(&self, root: &Path, changed: &Path) {
        if changed.strip_prefix(root).unwrap_or(changed).components().any(|c| c.as_os_str() == "target") {
            return;
        }
        let written = std::fs::metadata(changed).and_then(|m| m.modified()).ok();
        self.request(root, written);
    }

    fn request(&self, root: &Path, written: Option<SystemTime>) {
        if std::env::var("RIDX_REINDEX_ON_SAVE").is_ok_and(|v| v == "0") || !root.join("target/ridx/ridx_manifest.json").is_file() {
            return;
        }
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
        let root = root.to_path_buf();
        std::thread::spawn(move || loop {
            let started = std::time::Instant::now();
            // The run's output goes to `target/ridx/reindex.log`: why a crate did not build stays readable.
            let log = root.join(REINDEX_LOG);
            let out = std::fs::File::create(&log).map_or_else(|_| Stdio::null(), Stdio::from);
            let ok = Command::new(&exe).arg("index").arg(&root).current_dir(&root).stdin(Stdio::null()).stdout(Stdio::null()).stderr(out).status().is_ok_and(|s| s.success());
            let msg = format!("reindexed {} in {:.1} s ({}; log {})", root.display(), started.elapsed().as_secs_f64(), if ok { "ok" } else { "failed" }, log.display());
            crate::lsp::logger::log_info(&msg);
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
}
