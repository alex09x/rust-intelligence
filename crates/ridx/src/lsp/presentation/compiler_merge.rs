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

/// The diagnostics of `uri`: the compiler's from `flycheck` and the editor's own (`heuristic`, in the coordinates
/// of `live`), an editor diagnostic dropped where the compiler reports the same code over the same range. `saved`
/// gives the file as saved; it is read only when the compiler reported something and the client wants that.
pub fn merge_with_compiler(
    uri: &str,
    heuristic: &[Value],
    live: &str,
    saved: impl FnOnce() -> Option<String>,
    flycheck: &FlycheckState,
    ov: &crate::overlay::OverlayManager,
) -> Vec<Value> {
    let file = ridx_format::canonicalize_best_effort(std::path::Path::new(&crate::lsp::protocol::uri_to_path(uri)));
    if let Some((root, session)) = flycheck.session.of_with_root(&file) {
        // While the workspace has unsaved buffers, a check of the files as saved compiled other inputs than the buffers
        // (a sibling's text the file depends on, review of 46ce96f2): the session's result, of the buffers, applies.
        // With none, the check of the saved files does.
        if super::session_diag::unsaved_in(ov, &root) {
            return merge_session(uri, heuristic, live, &session);
        }
    }
    let raw = flycheck.get_latest_diagnostics(uri);
    if raw.is_empty() {
        return heuristic.to_vec();
    }
    let baseline = flycheck.get_diagnostic_baseline(uri);
    // Overlaps are compared where both kinds sit: in the buffer.
    let in_live = map_authoritative_diagnostics_to_text(&raw, baseline.as_deref(), live, uri);
    let emitted = if CLIENT_MOVES_RUSTC.with(Cell::get) {
        let saved = saved().unwrap_or_else(|| live.to_string());
        map_authoritative_diagnostics_to_text(&raw, baseline.as_deref(), &saved, uri)
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
