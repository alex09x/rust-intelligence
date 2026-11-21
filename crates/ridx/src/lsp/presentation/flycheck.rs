/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Asynchronous background flycheck and compiler diagnostics publisher.

pub use super::compiler_message::{find_ridx_driver, find_ridx_driver_with, parse_compiler_message};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

#[inline]
fn kill_child_process_tree(child: &mut Child) {
    #[cfg(unix)]
    {
        let pid = child.id() as i32;
        if pid > 1 {
            unsafe {
                extern "C" {
                    fn kill(pid: i32, sig: i32) -> i32;
                }
                let _ = kill(-pid, 9);
            }
        }
    }
    let _ = child.kill();
}

pub type FlycheckCallback = Box<dyn Fn(u64, Value) + Send + Sync>;

#[derive(Default, Clone)]
pub struct CachedDiagnostic {
    pub items: Vec<Value>,
    pub baseline: Option<String>,
}

pub struct FlycheckState {
    running_child: Arc<Mutex<Option<(u64, Child)>>>,
    generation: Arc<AtomicU64>,
    published_uris: Arc<Mutex<HashMap<PathBuf, HashSet<String>>>>,
    latest_diagnostics: Arc<Mutex<HashMap<String, CachedDiagnostic>>>,
    active_diagnostic_uris: Arc<Mutex<HashSet<String>>>,
    worker_handle: Arc<Mutex<Option<JoinHandle<()>>>>,
    notifier: Arc<Mutex<Option<FlycheckCallback>>>,
    /// The hash of each source's text on disk when a check was last started for it (0: the file was gone), and
    /// whether a save started it.
    checked_sources: Arc<Mutex<HashMap<PathBuf, (u64, bool)>>>,
    /// The roots still to check, the one being checked first (see `queue`).
    queue: Arc<Mutex<Vec<PathBuf>>>,
    /// The generation of the last check started per root: a result is stale only when a newer check of its own root
    /// started (see `is_current`).
    root_generation: Arc<Mutex<HashMap<PathBuf, u64>>>,
    /// The compiler's diagnostics on the text of the session indexes answering, which stand in for the check's.
    pub session: super::session_diag::SessionDiagnostics,
    /// Per URI, when the check that last reported on it (diagnostics or their clearing) started, in microseconds since
    /// the epoch: a check started after a session index was built speaks for the saved text more recently than it.
    checked_at: Arc<Mutex<HashMap<String, u64>>>,
}

#[path = "flycheck_queue.rs"]
mod queue;

impl Default for FlycheckState {
    fn default() -> Self {
        Self {
            running_child: Arc::new(Mutex::new(None)),
            generation: Arc::new(AtomicU64::new(0)),
            published_uris: Arc::default(),
            latest_diagnostics: Arc::default(),
            active_diagnostic_uris: Arc::default(),
            worker_handle: Arc::new(Mutex::new(None)),
            notifier: Arc::new(Mutex::new(None)),
            checked_sources: Arc::default(),
            queue: Arc::default(),
            root_generation: Arc::default(),
            session: Default::default(),
            checked_at: Arc::default(),
        }
    }
}

impl FlycheckState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a save of `file` starts a check, recording its text as checked. Every save does, as with
    /// rust-analyzer, but the one whose watched-file event (Zed sends it just before `didSave`) already started the
    /// check of this very text.
    pub fn mark_saved_for_check(&self, file: &Path) -> bool {
        let hash = Self::disk_hash(file);
        let mut checked = self.checked_sources.lock().unwrap_or_else(|e| e.into_inner());
        checked.insert(file.to_path_buf(), (hash, true)) != Some((hash, false))
    }

    /// Whether a watched-file event for `file` starts a check: when its text on disk is not the one last checked
    /// (a checkout or a pull, reloaded into the buffer or not), never again for a text a save already checked (the
    /// event of that save, even once the buffer is edited again).
    pub fn mark_changed_for_check(&self, file: &Path) -> bool {
        let hash = Self::disk_hash(file);
        let mut checked = self.checked_sources.lock().unwrap_or_else(|e| e.into_inner());
        if checked.get(file).is_some_and(|&(h, _)| h == hash) {
            return false;
        }
        checked.insert(file.to_path_buf(), (hash, false));
        true
    }

    fn disk_hash(file: &Path) -> u64 {
        std::fs::read(file).map_or(0, |b| ridx_format::normalized_source_hash(&b))
    }

    pub fn current_generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    pub fn get_latest_diagnostics(&self, uri: &str) -> Vec<Value> {
        self.latest_diagnostics.lock().unwrap().get(uri).map(|c| c.items.clone()).unwrap_or_default()
    }

    pub fn get_diagnostic_baseline(&self, uri: &str) -> Option<String> {
        self.latest_diagnostics.lock().unwrap().get(uri).and_then(|c| c.baseline.clone())
    }

    pub fn set_latest_diagnostics(&self, uri: &str, diags: Vec<Value>) {
        self.set_latest_diagnostics_with_baseline(uri, diags, None);
    }

    pub fn set_latest_diagnostics_with_baseline(&self, uri: &str, diags: Vec<Value>, baseline: Option<String>) {
        let mut guard = self.latest_diagnostics.lock().unwrap();
        if diags.is_empty() {
            guard.remove(uri);
        } else {
            let base = baseline.or_else(|| std::fs::read_to_string(super::super::protocol::uri_to_path(uri)).ok());
            guard.insert(uri.to_string(), CachedDiagnostic { items: diags, baseline: base });
        }
    }

    /// Records that a check started at `started_us` reported on `uri` (see `checked_at`).
    pub fn set_checked_at(&self, uri: &str, started_us: u64) {
        self.checked_at.lock().unwrap().insert(uri.to_string(), started_us);
    }

    /// When the check that last reported on `uri` started.
    pub fn checked_at(&self, uri: &str) -> Option<u64> {
        self.checked_at.lock().unwrap().get(uri).copied()
    }

    pub fn mark_published(&self, uri: &str) {
        self.active_diagnostic_uris.lock().unwrap().insert(uri.to_string());
    }

    pub fn clear_published(&self, uri: &str) -> bool {
        self.active_diagnostic_uris.lock().unwrap().remove(uri)
    }

    pub fn has_active_diagnostics(&self, uri: &str) -> bool {
        self.active_diagnostic_uris.lock().unwrap().contains(uri)
    }

    pub fn set_notifier<F: Fn(u64, Value) + Send + Sync + 'static>(&self, notifier: F) {
        let mut guard = self.notifier.lock().unwrap();
        *guard = Some(Box::new(notifier));
    }

    pub fn cancel(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        let mut guard = self.running_child.lock().unwrap();
        if let Some((_gen, mut child)) = guard.take() {
            kill_child_process_tree(&mut child);
            let _ = child.wait();
        }
    }

    pub fn shutdown(&self) {
        self.cancel();
        let handle = self.worker_handle.lock().unwrap().take();
        if let Some(h) = handle {
            let _ = h.join();
        }
    }

    /// Synchronous runner reserved exclusively for direct unit and integration tests.
    /// Emits raw compiler messages directly to the test writer without overlay reconciliation.
    /// Production LSP workflows exclusively use `trigger_check`, routing through
    /// `InternalDiagnostic` and `publish_reconciled_diagnostics`.
    pub fn run_check<W: std::io::Write>(&self, workspace_root: &Path, writer: &mut W) -> std::io::Result<usize> {
        self.cancel();
        let my_gen = self.generation.load(Ordering::SeqCst);
        let running_child = Arc::clone(&self.running_child);
        let published_uris = Arc::clone(&self.published_uris);
        let generation = Arc::clone(&self.generation);

        run_check_internal(workspace_root, my_gen, &generation, &running_child, &published_uris, &self.latest_diagnostics, |_gen, notif| {
            super::super::protocol::send_response(writer, &notif)
        })
    }
}

/// Whether `path` was written after `since`, or is gone.
pub(crate) fn modified_since(path: &Path, since: std::time::SystemTime) -> bool {
    std::fs::metadata(path).and_then(|m| m.modified()).map_or(true, |t| t > since)
}

fn run_check_internal<F>(
    workspace_root: &Path,
    my_gen: u64,
    generation: &Arc<AtomicU64>,
    running_child: &Arc<Mutex<Option<(u64, Child)>>>,
    published_uris: &Arc<Mutex<HashMap<PathBuf, HashSet<String>>>>,
    latest_diagnostics: &Arc<Mutex<HashMap<String, CachedDiagnostic>>>,
    mut emit_notif: F,
) -> std::io::Result<usize>
where
    F: FnMut(u64, Value) -> std::io::Result<()>,
{
    if generation.load(Ordering::SeqCst) != my_gen {
        return Ok(0);
    }

    let cargo_bin = workspace_root.parent().map(|p| p.join("bin").join("cargo")).filter(|p| p.is_file()).unwrap_or_else(|| PathBuf::from("cargo"));
    let mut cmd = Command::new(cargo_bin);
    cmd.args(["check", "--message-format=json", "--workspace", "--all-targets"]);
    cmd.current_dir(workspace_root);
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }

    // Use isolated target directory to prevent lock contention and fingerprint invalidation
    let target_dir = workspace_root.join("target").join("ridx-flycheck");
    cmd.env("CARGO_TARGET_DIR", target_dir);

    // Only inject RUSTC_WRAPPER if explicitly requested via environment opt-in
    if std::env::var("RIDX_FLYCHECK_DRIVER").is_ok_and(|v| v == "1") {
        if let Some(driver) = find_ridx_driver(workspace_root) {
            // The driver loads its toolchain's compiler libraries and needs that toolchain's cargo check, as in
            // the index build (indexer/builder.rs).
            cmd.env("RUSTC_WRAPPER", driver).env("RUSTC_BOOTSTRAP", "1");
            if let Some(ld) = crate::indexer::cargo::compute_sysroot_ld_path() {
                cmd.env("LD_LIBRARY_PATH", &ld).env("DYLD_FALLBACK_LIBRARY_PATH", &ld);
            }
            if let (None, Some(tc)) = (std::env::var_os("RUSTUP_TOOLCHAIN"), crate::indexer::cargo::driver_toolchain()) {
                cmd.env("RUSTUP_TOOLCHAIN", tc);
            }
        }
    }

    let started = std::time::SystemTime::now();
    let started_us = super::session_diag::micros(started);
    let mut child = cmd.spawn()?;
    let Some(stdout) = child.stdout.take() else {
        return Ok(0);
    };

    {
        let mut guard = running_child.lock().unwrap();
        if generation.load(Ordering::SeqCst) != my_gen {
            kill_child_process_tree(&mut child);
            let _ = child.wait();
            return Ok(0);
        }
        *guard = Some((my_gen, child));
    }

    let reader = BufReader::new(stdout);
    let mut file_diagnostics: HashMap<String, Vec<Value>> = HashMap::new();
    let mut seen_diagnostics: HashSet<(String, u64, u64, u64, u64, i64, String)> = HashSet::new();

    for line_res in reader.lines() {
        if generation.load(Ordering::SeqCst) != my_gen {
            break;
        }
        let line = match line_res {
            Ok(l) => l,
            Err(_) => break,
        };
        if let Ok(val) = serde_json::from_str::<Value>(&line) {
            if val.get("reason").and_then(|r| r.as_str()) == Some("compiler-message") {
                if let Some((uri, diag)) = val.get("message").and_then(|m| parse_compiler_message(m, workspace_root)) {
                    let l_start = diag["range"]["start"]["line"].as_u64().unwrap_or(0);
                    let c_start = diag["range"]["start"]["character"].as_u64().unwrap_or(0);
                    let l_end = diag["range"]["end"]["line"].as_u64().unwrap_or(0);
                    let c_end = diag["range"]["end"]["character"].as_u64().unwrap_or(0);
                    let sev = diag["severity"].as_i64().unwrap_or(1);
                    let m_text = diag["message"].as_str().unwrap_or("").to_string();

                    let key = (uri.clone(), l_start, c_start, l_end, c_end, sev, m_text);
                    if seen_diagnostics.insert(key) {
                        file_diagnostics.entry(uri).or_default().push(diag);
                    }
                }
            }
        }
    }

    // Wait on child if it belongs to our generation; put back if a newer one was installed
    {
        let mut guard = running_child.lock().unwrap();
        if let Some((gen, mut c)) = guard.take() {
            if gen == my_gen {
                let _ = c.wait();
            } else {
                *guard = Some((gen, c));
            }
        }
    }

    // If cancelled or superseded during run, do not publish or clear anything
    if generation.load(Ordering::SeqCst) != my_gen {
        return Ok(0);
    }
    // The text each file's diagnostics describe. A file saved again, reverted (`git checkout`) or removed while the
    // check ran gets none: the text they describe is gone and the one on disk was not checked. The text is read
    // before the time is compared, so a write after the read shows in the time too.
    let mut baselines: HashMap<String, String> = HashMap::new();
    file_diagnostics.retain(|uri, _| {
        let path = PathBuf::from(super::super::protocol::uri_to_path(uri));
        let Ok(text) = std::fs::read_to_string(&path) else { return false };
        if modified_since(&path, started) {
            return false;
        }
        baselines.insert(uri.clone(), text);
        true
    });
    crate::lsp::logger::log_verbose(&format!("flycheck {my_gen}: diagnostics for {} file(s)", file_diagnostics.len()));

    let mut published_guard = published_uris.lock().unwrap();
    let prev_published = published_guard.get(workspace_root).cloned().unwrap_or_default();
    let mut current_uris = HashSet::new();

    // 1. Publish new / updated diagnostics
    for (uri, diags) in &file_diagnostics {
        if generation.load(Ordering::SeqCst) != my_gen {
            return Ok(0);
        }
        current_uris.insert(uri.clone());
        // Known as published before it goes out: a check cancelled halfway leaves it to the next one to clear (review
        // of 11310378: a warning of a cancelled batch stayed for good).
        published_guard.entry(workspace_root.to_path_buf()).or_default().insert(uri.clone());
        let notif = json!({
            "jsonrpc": "2.0",
            "method": "textDocument/publishDiagnostics",
            "params": { "uri": uri, "diagnostics": diags, "baseline": baselines.get(uri), "started": started_us }
        });
        emit_notif(my_gen, notif)?;
    }

    // 2. Clear previously published URIs for THIS root that now have 0 errors
    for old_uri in &prev_published {
        if generation.load(Ordering::SeqCst) != my_gen {
            return Ok(0);
        }
        if !current_uris.contains(old_uri) {
            let clear_notif = json!({
                "jsonrpc": "2.0",
                "method": "textDocument/publishDiagnostics",
                "params": { "uri": old_uri, "diagnostics": [], "started": started_us }
            });
            emit_notif(my_gen, clear_notif)?;
        }
    }

    if generation.load(Ordering::SeqCst) == my_gen {
        // A URI cleared stays known until the server takes its clear (`clear_delivered`): one dropped as stale (a newer
        // check of the root started first) is cleared again by the next check (review of 503071cb).
        published_guard.insert(workspace_root.to_path_buf(), current_uris.union(&prev_published).cloned().collect());
        let mut latest_guard = latest_diagnostics.lock().unwrap();
        for (uri, diags) in &file_diagnostics {
            latest_guard.insert(uri.clone(), CachedDiagnostic { items: diags.clone(), baseline: baselines.remove(uri) });
        }
        for old_uri in &prev_published {
            if !current_uris.contains(old_uri) {
                latest_guard.remove(old_uri);
            }
        }
    }
    let total_diags = file_diagnostics.values().map(|d| d.len()).sum();
    Ok(total_diags)
}
