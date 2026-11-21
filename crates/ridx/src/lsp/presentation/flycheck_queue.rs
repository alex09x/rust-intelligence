/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Which roots the background check runs on. One worker checks them one after the other: a check per root would
//! cancel the one before (review of a34d8100: on start only the first of two workspace folders was checked). A new
//! request cancels the check running, puts its roots first and keeps the others still to check, the interrupted one
//! included (review of ca0c1ec7: a save in one root during the check on start dropped the other roots).

use super::{run_check_internal, FlycheckState};

/// The key the worker adds to each notification it hands to the server: the root it checked.
pub(super) const ROOT_KEY: &str = "ridxRoot";
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;

impl FlycheckState {
    /// The server took `diag` of generation `gen`: a clear (no diagnostics) of a root's URI ends what that root has to
    /// clear there, unless a newer check of the root started since (it may have published the URI again: review of
    /// 3650f8e3). Checked under the lock a worker registers a check with, so none starts in between.
    pub fn clear_delivered(&self, gen: u64, diag: &serde_json::Value) {
        let (Some(root), Some(uri)) = (diag.get(ROOT_KEY).and_then(serde_json::Value::as_str), diag["params"]["uri"].as_str()) else { return };
        if !diag["params"]["diagnostics"].as_array().is_some_and(|d| d.is_empty()) {
            return;
        }
        let gens = self.root_generation.lock().unwrap_or_else(|e| e.into_inner());
        if gens.get(Path::new(root)).is_some_and(|&g| g == gen) {
            if let Some(uris) = self.published_uris.lock().unwrap_or_else(|e| e.into_inner()).get_mut(Path::new(root)) {
                uris.remove(uri);
            }
        }
    }

    /// Whether diagnostics of generation `gen` are still to publish: those of a root (`ridxRoot`, set by the worker)
    /// until a newer check of that same root starts, others while no newer check of any root did. A check requested
    /// for another root no longer drops a root's finished results (review of 11310378).
    pub fn is_current(&self, gen: u64, diag: &serde_json::Value) -> bool {
        match diag.get(ROOT_KEY).and_then(serde_json::Value::as_str) {
            Some(root) => self.root_generation.lock().unwrap_or_else(|e| e.into_inner()).get(Path::new(root)).is_none_or(|&g| g == gen),
            None => gen == self.current_generation(),
        }
    }

    /// Triggers an asynchronous background cargo check, publishing diagnostics via notifier.
    pub fn trigger_check(&self, workspace_root: &Path) {
        self.trigger_checks(vec![workspace_root.to_path_buf()]);
    }

    /// Checks `roots`, then the roots an earlier request had not finished, one after the other.
    pub fn trigger_checks(&self, roots: Vec<PathBuf>) {
        self.cancel();
        let my_gen = self.generation.load(Ordering::SeqCst);
        {
            let mut queue = self.queue.lock().unwrap_or_else(|e| e.into_inner());
            let unfinished = std::mem::take(&mut *queue);
            for root in roots.into_iter().chain(unfinished) {
                if !queue.contains(&root) {
                    queue.push(root);
                }
            }
        }

        let running_child = Arc::clone(&self.running_child);
        let published_uris = Arc::clone(&self.published_uris);
        let latest_diagnostics = Arc::clone(&self.latest_diagnostics);
        let notifier = Arc::clone(&self.notifier);
        let generation = Arc::clone(&self.generation);
        let queue = Arc::clone(&self.queue);
        let root_generation = Arc::clone(&self.root_generation);

        let handle = std::thread::Builder::new().name("flycheck-worker".into()).spawn(move || loop {
            if generation.load(Ordering::SeqCst) != my_gen {
                break;
            }
            let Some(root) = queue.lock().unwrap_or_else(|e| e.into_inner()).first().cloned() else { break };
            {
                // A worker cancelled since it took the root stops before registering, and a root's generation only
                // grows: an old worker never makes a newer check's results stale (review of a21afbc0).
                let mut gens = root_generation.lock().unwrap_or_else(|e| e.into_inner());
                if generation.load(Ordering::SeqCst) != my_gen {
                    break;
                }
                let g = gens.entry(root.clone()).or_insert(my_gen);
                *g = (*g).max(my_gen);
            }
            let _ = run_check_internal(&root, my_gen, &generation, &running_child, &published_uris, &latest_diagnostics, |gen, mut notif| {
                notif[ROOT_KEY] = root.display().to_string().into();
                if let Some(ref cb) = *notifier.lock().unwrap() {
                    cb(gen, notif);
                }
                Ok(())
            });
            // Cancelled meanwhile: the root stays queued for the worker of the request that cancelled it.
            let mut q = queue.lock().unwrap_or_else(|e| e.into_inner());
            if generation.load(Ordering::SeqCst) != my_gen {
                break;
            }
            q.retain(|r| r != &root);
        });

        if let Ok(h) = handle {
            let mut guard = self.worker_handle.lock().unwrap();
            *guard = Some(h);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Review of 11310378: root A's finished results were dropped when a check of root B started before the server
    /// took them.
    #[test]
    fn test_a_result_is_stale_only_after_a_newer_check_of_its_root() {
        let state = FlycheckState::new();
        let (a, b) = (Path::new("/w/a"), Path::new("/w/b"));
        let gen_a = state.current_generation();
        state.root_generation.lock().unwrap().insert(a.to_path_buf(), gen_a);
        let result_a = json!({ (ROOT_KEY): a.display().to_string() });
        state.cancel(); // a check of B is requested
        state.root_generation.lock().unwrap().insert(b.to_path_buf(), state.current_generation());
        assert!(state.is_current(gen_a, &result_a), "B's check leaves A's result current");
        assert!(!state.is_current(gen_a, &json!({})), "a result without a root follows the global generation");
        state.root_generation.lock().unwrap().insert(a.to_path_buf(), state.current_generation());
        assert!(!state.is_current(gen_a, &result_a), "a newer check of A makes it stale");
    }

    /// Review of 503071cb: a clear dropped as stale lost the URI, and no later check cleared it again.
    #[test]
    fn test_a_uri_stays_to_clear_until_its_clear_is_taken() {
        let state = FlycheckState::new();
        let root = Path::new("/w/a");
        state.published_uris.lock().unwrap().insert(root.to_path_buf(), ["file:///w/a/x.rs".to_string()].into());
        let note = |diags: serde_json::Value| json!({ (ROOT_KEY): root.display().to_string(), "params": { "uri": "file:///w/a/x.rs", "diagnostics": diags } });
        let gen = state.current_generation();
        state.root_generation.lock().unwrap().insert(root.to_path_buf(), gen);
        state.clear_delivered(gen, &note(json!([{ "message": "w" }])));
        assert_eq!(state.published_uris.lock().unwrap()[root].len(), 1, "a diagnostic is not a clear");
        // Review of 3650f8e3: a newer check of the root started (and may have published the URI again).
        state.root_generation.lock().unwrap().insert(root.to_path_buf(), gen + 1);
        state.clear_delivered(gen, &note(json!([])));
        assert_eq!(state.published_uris.lock().unwrap()[root].len(), 1, "an older clear leaves the newer check's URI");
        state.clear_delivered(gen + 1, &note(json!([])));
        assert!(state.published_uris.lock().unwrap()[root].is_empty(), "the clear of the current check taken: nothing left to clear");
    }
}
