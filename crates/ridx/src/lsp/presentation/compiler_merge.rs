/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Compiler diagnostics merged with the editor's own, in the coordinates the client expects.
//!
//! Zed under the rust-analyzer adapter treats source `rustc` as disk-based: it moves such diagnostics through the
//! buffer's unsaved edits itself, so they must come in the coordinates of the file as saved (placed in the buffer,
//! they moved twice: a warning sat a line below its name after a line was typed above it). Other clients take every
//! diagnostic in the buffer's coordinates, for the version it is published with.

use super::flycheck::FlycheckState;
use super::reconcile::{map_authoritative_diagnostics_to_text, ranges_overlap};
use serde_json::Value;
use std::cell::Cell;

thread_local! {
    static CLIENT_MOVES_RUSTC: Cell<bool> = const { Cell::new(false) };
}

/// Records from `initialize` whether the client moves `rustc` diagnostics itself (Zed does).
pub fn set_client(params: &Value) {
    set_client_moves_rustc(params["clientInfo"]["name"].as_str() == Some("Zed"));
}

pub fn set_client_moves_rustc(moves: bool) {
    CLIENT_MOVES_RUSTC.with(|c| c.set(moves));
}

/// The key the check keeps the result of `file` (canonical) under: its path as a raw `file://` URI.
pub fn check_key(file: &std::path::Path) -> String {
    format!("file://{}", file.display())
}

/// The file a check key names ([`check_key`]): the path after `file://` as it is, never percent-decoded.
pub fn check_path(key: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(key.strip_prefix("file://").unwrap_or(key))
}

/// The diagnostics of `uri`: the compiler's from `flycheck` and the editor's own (`heuristic`, in the coordinates
/// of `live`), an editor diagnostic dropped where the compiler reports the same code over the same range. `saved`
/// gives the file as saved; it is read only when the compiler reported something and the client wants that.
pub fn merge_with_compiler(uri: &str, heuristic: &[Value], live: &str, saved: impl FnOnce() -> Option<String>, flycheck: &FlycheckState) -> Vec<Value> {
    let file = ridx_format::canonicalize_best_effort(std::path::Path::new(&crate::lsp::protocol::uri_to_path(uri)));
    // A buffer a session index compiled has the compiler's result of the buffers, saved since or not: the check of the
    // saved files is of other inputs until a session no longer has it (reviews of 46ce96f2 and 710381e0).
    if let Some(session) = flycheck.session.of(&file) {
        return merge_session(uri, heuristic, live, &session);
    }
    // A file on disk no crate includes says so, with rust-analyzer's hint (prod-code, m=20185).
    if flycheck.session.outcome(&file).is_some_and(|o| o.unavailable == Some(super::session_diag::UNLINKED)) {
        let saved = saved();
        let mut out = merge_check(uri, heuristic, live, || saved.clone(), flycheck, &file);
        out.extend(super::session_diag::unlinked_hint(flycheck, &file, live, saved.as_deref()));
        return out;
    }
    merge_check(uri, heuristic, live, saved, flycheck, &file)
}

/// [`merge_with_compiler`] with the check of the saved files, for `file` (canonical).
fn merge_check(uri: &str, heuristic: &[Value], live: &str, saved: impl FnOnce() -> Option<String>, flycheck: &FlycheckState, file: &std::path::Path) -> Vec<Value> {
    // The check names a file by its path as a raw `file://` URI, whatever spelling the client's URI has (`a%20b.rs`,
    // review of d4541e29): that is where its result is looked up first.
    let key = check_key(file);
    // The client's URI as it is only when spelled as the check spells paths (no escape): `a%20b.rs` as a key is another
    // file's, one named so (review of 88fe53d0).
    let key = if flycheck.get_latest_diagnostics(&key).is_empty() && !uri.contains('%') { uri.to_string() } else { key };
    let raw = flycheck.get_latest_diagnostics(&key);
    if raw.is_empty() {
        return heuristic.to_vec();
    }
    let baseline = flycheck.get_diagnostic_baseline(&key);
    // Overlaps are compared where both kinds sit: in the buffer.
    let in_live = map_authoritative_diagnostics_to_text(&raw, baseline.as_deref(), live, &key);
    let emitted = if CLIENT_MOVES_RUSTC.with(Cell::get) {
        let saved = saved().unwrap_or_else(|| live.to_string());
        map_authoritative_diagnostics_to_text(&raw, baseline.as_deref(), &saved, &key)
    } else {
        in_live.clone()
    };
    let shadowed = |h: &Value| in_live.iter().any(|a| a.get("code").and_then(Value::as_str) == h.get("code").and_then(Value::as_str) && ranges_overlap(&h["range"], &a["range"]));
    emitted.into_iter().chain(heuristic.iter().filter(|h| !shadowed(h)).cloned()).collect()
}

/// The diagnostics of a file a session index compiled: the compiler's on the session's text, moved to `live` by its
/// edits since, always in the buffer's coordinates. For a client that moves `rustc` diagnostics through unsaved edits
/// itself they say `ridx`: they were reported on the buffer, not on the file as saved, and must stay where they are.
fn merge_session(uri: &str, heuristic: &[Value], live: &str, session: &super::session_diag::SessionFile) -> Vec<Value> {
    let mut in_live = map_authoritative_diagnostics_to_text(&session.diagnostics, Some(&session.text), live, uri);
    if CLIENT_MOVES_RUSTC.with(Cell::get) {
        for d in &mut in_live {
            d["source"] = "ridx".into();
        }
    }
    let shadowed = |h: &Value| in_live.iter().any(|a| a.get("code").and_then(Value::as_str) == h.get("code").and_then(Value::as_str) && ranges_overlap(&h["range"], &a["range"]));
    let editor: Vec<Value> = heuristic.iter().filter(|h| !shadowed(h)).cloned().collect();
    in_live.into_iter().chain(editor).collect()
}
