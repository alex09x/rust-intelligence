/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Reindexing on save. The index answers navigation, references and rename for what was compiled; a saved edit
//! reaches it once `ridx index` has run again, which the server then picks up by itself (the manifest changes on
//! disk). So a save of a file in an indexed workspace starts `ridx index <root>` in the background: one run per root at
//! a time, the saves made while it runs folded into one more run after it. `RIDX_REINDEX_ON_SAVE=0` turns it off.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

/// Per workspace root: whether a run is going and whether another was asked for meanwhile.
#[derive(Default)]
pub struct Reindexer {
    roots: Arc<Mutex<HashMap<PathBuf, bool>>>,
}

impl Reindexer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reindexes `root` in the background after a save, when it already has an index.
    pub fn trigger(&self, root: &Path) {
        if std::env::var("RIDX_REINDEX_ON_SAVE").is_ok_and(|v| v == "0") || !root.join("target/ridx/ridx_manifest.json").is_file() {
            return;
        }
        let Some(exe) = ridx_exe() else { return };
        {
            let mut roots = self.roots.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(again) = roots.get_mut(root) {
                *again = true;
                return;
            }
            roots.insert(root.to_path_buf(), false);
        }
        let roots = Arc::clone(&self.roots);
        let root = root.to_path_buf();
        std::thread::spawn(move || loop {
            let started = std::time::Instant::now();
            let ok =
                Command::new(&exe).arg("index").arg(&root).current_dir(&root).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success());
            crate::lsp::logger::log_info(&format!("reindexed {} after save in {:.1} s ({})", root.display(), started.elapsed().as_secs_f64(), if ok { "ok" } else { "failed" }));
            let mut roots = roots.lock().unwrap_or_else(|e| e.into_inner());
            if roots.get(&root) == Some(&true) {
                roots.insert(root.clone(), false);
            } else {
                roots.remove(&root);
                break;
            }
        });
    }
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
        r.trigger(dir.path());
        assert!(r.roots.lock().unwrap().is_empty(), "no index, no run");
        std::fs::create_dir_all(dir.path().join("target/ridx")).unwrap();
        std::fs::write(dir.path().join("target/ridx/ridx_manifest.json"), "{}").unwrap();
        assert!(ridx_exe().is_none(), "the test binary is not ridx");
        r.trigger(dir.path());
        assert!(r.roots.lock().unwrap().is_empty());
    }
}
