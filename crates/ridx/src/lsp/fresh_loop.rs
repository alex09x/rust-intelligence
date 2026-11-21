/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! The server loop's side of `minVersion` (`fresh`) and `ridx/status`: what is answered before the ordinary dispatch,
//! and against which index.

use super::folders::Folders;
use super::fresh::{Coverage, Waiting};
use super::session_index::SessionIndexer;
use crate::lsp::presentation::session_diag::{readiness, Readiness};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Set on a request admitted on the indexes refreshed for it: dispatch answers from them without reloading, so the
/// index that passed `minVersion` is the one that answers (review of 2045537b: a publication between admission and
/// dispatch changed it).
pub(crate) const PINNED: &str = "__ridxPinned";

/// The loop's state before dispatch: requests held for `minVersion`, and the client's folders. (A request's id is
/// retired when its response is written: `requests::RetiringWriter`.)
#[derive(Default)]
pub struct Gate {
    pub waiting: Waiting,
    folders: Folders,
}

impl Gate {
    fn send_answers<W: Write>(&self, writer: &mut W, answers: Vec<Value>) -> io::Result<()> {
        for answer in answers {
            super::protocol::send_response(writer, &answer)?;
        }
        Ok(())
    }

    /// `initialize` was accepted: its folders, and the root of the current directory the server took when it named
    /// none, are the roots of the session (review of 8a28e8ff: a rejected `initialize` added its folders, and the
    /// current directory's root was none).
    pub fn initialized(&mut self, msg: &Value, reindex: &super::reindex::Reindexer) {
        self.folders.add_initialize(msg);
        let known = self.folders.roots();
        for root in reindex.queued_roots() {
            let root = ridx_format::canonicalize_best_effort(&root);
            if !known.contains(&root) {
                self.folders.add_root(&root);
            }
        }
    }
}

/// The hash of `file` the index answering for it recorded, when all its units agree on it: one unit compiled from the
/// buffer's text and another from the text before is no coverage (review of e535e9cf).
pub(crate) fn answering_hash(mr: &super::MultiRootWorkspace, file: &str) -> Option<u64> {
    let ws = mr.find_workspace_for_path(Path::new(file))?;
    ws.units_agree_on_source(file).then(|| ws.source_hash_for_file(file)).flatten()
}

/// The indexes as they are now: a root of the session (a client folder's, or one indexed this session) whose first
/// index was published since, by anyone (reviews of e535e9cf and 2045537b), is loaded, and a loaded one whose manifest
/// changed is reloaded.
fn refresh<W: Write>(
    gate: &Gate,
    writer: &mut W,
    lifecycle: &mut super::LifecycleManager,
    mr: &mut super::MultiRootWorkspace,
    ov: &mut crate::overlay::OverlayManager,
) -> io::Result<()> {
    let loaded: HashSet<PathBuf> = mr.all_workspaces().filter_map(|ws| ws.workspace_root()).map(ridx_format::canonicalize_best_effort).collect();
    for root in gate.folders.roots().into_iter().chain(lifecycle.reindex.roots()) {
        if !loaded.contains(&root) && root.join("target/ridx/ridx_manifest.json").is_file() && mr.add_root(root.clone()) {
            if let Some(ws) = mr.all_workspaces().last() {
                super::rebase_open_buffers(ws, ov);
            }
        }
    }
    super::dispatch::reload_changed_index(writer, lifecycle, mr, ov)
}

/// The server loop's turn for waiting requests: after `shutdown` they are answered `-32600`; otherwise the indexes are
/// refreshed and the ended or covered ones come out. Returns the requests to dispatch (through `intercept` again).
pub(crate) fn release_waiting<W: Write>(
    gate: &mut Gate,
    writer: &mut W,
    lifecycle: &mut super::LifecycleManager,
    mr: &mut super::MultiRootWorkspace,
    ov: &mut crate::overlay::OverlayManager,
    session: &SessionIndexer,
    cancelled: &Mutex<HashSet<Value>>,
) -> io::Result<Vec<Value>> {
    if gate.waiting.is_empty() {
        return Ok(Vec::new());
    }
    if lifecycle.is_shutdown() {
        let answers = gate.waiting.drain(-32600, "the server is shutting down");
        gate.send_answers(writer, answers)?;
        return Ok(Vec::new());
    }
    refresh(gate, writer, lifecycle, mr, ov)?;
    // A request held for its version whose index failed since gets that answer now, not a timeout (review of b3d0d293).
    let refused = gate.waiting.refuse(|m| refusal(m, mr, &lifecycle.reindex), cancelled);
    gate.send_answers(writer, refused)?;
    let ov: &crate::overlay::OverlayManager = ov;
    let fc = &lifecycle.flycheck;
    let cov = Coverage { ov, indexed_hash: &|f| answering_hash(mr, f), diagnostics: &|f| diagnostics_of(fc, ov, mr, &lifecycle.reindex, f) };
    // A pull held while the workspace its file belongs to changed (a nested folder added) has that one compile it now:
    // it is not asked again (review of 775998ea).
    for msg in gate.waiting.held_diagnostics() {
        demand(msg, &cov, mr, &lifecycle.reindex, session, false);
    }
    let (ready, answers) = gate.waiting.release(&cov, cancelled);
    gate.send_answers(writer, answers)?;
    Ok(ready)
}

/// Handles `msg` before the ordinary dispatch; returns it to dispatch unless answered or held here.
///
/// - `$/cancelRequest` was handled by the reader (`requests`): nothing to do in order.
/// - `workspace/didChangeWorkspaceFolders` is applied here, once: removing the last folder standing for a root removes
///   the root (its index, its reindex state and a success not taken yet), and a root another folder still stands for
///   stays (review of 8a28e8ff: the raw removal went on to dispatch and unloaded it).
/// - A request released from waiting takes back its deadline here, on every path: a cancelled one leaves none to a later
///   request reusing its id (review of 8a28e8ff).
/// - `ridx/status` and holding are only for a request (with an `id`) of an initialized server not shut down and not
///   cancelled: the rest goes through dispatch's own checks (review of a6909dbe). Both look at the indexes refreshed
///   first; a request admitted on them is pinned to them.
#[allow(clippy::too_many_arguments)]
pub(crate) fn intercept<W: Write>(
    mut msg: Value,
    gate: &mut Gate,
    writer: &mut W,
    lifecycle: &mut super::LifecycleManager,
    mr: &mut super::MultiRootWorkspace,
    ov: &mut crate::overlay::OverlayManager,
    session: &SessionIndexer,
    cancelled: &Mutex<HashSet<Value>>,
) -> io::Result<Option<Value>> {
    let id = msg.get("id").filter(|id| !id.is_null()).cloned();
    let deadline = id.as_ref().and_then(|id| gate.waiting.take_released(id));
    match msg["method"].as_str() {
        Some("$/cancelRequest") => return Ok(None),
        Some("workspace/didChangeWorkspaceFolders") => {
            let event = &msg["params"]["event"];
            for item in event["removed"].as_array().into_iter().flatten() {
                if let Some(root) = gate.folders.remove(&super::protocol::uri_to_path(item["uri"].as_str().unwrap_or(""))) {
                    lifecycle.reindex.forget(&root);
                    mr.remove_root(&root);
                }
            }
            for item in event["added"].as_array().into_iter().flatten() {
                let root = gate.folders.add(&super::protocol::uri_to_path(item["uri"].as_str().unwrap_or("")));
                mr.add_root(root.clone());
                lifecycle.reindex.index_if_missing(&root);
            }
            return Ok(None);
        }
        _ => {}
    }
    let is_cancelled = id.as_ref().is_some_and(|id| cancelled.lock().unwrap_or_else(|e| e.into_inner()).contains(id));
    if id.is_none() || is_cancelled || !lifecycle.is_initialized() || lifecycle.is_shutdown() {
        return Ok(Some(msg));
    }
    if msg["method"] == "ridx/status" {
        refresh(gate, writer, lifecycle, mr, ov)?;
        let result = status(&msg, &gate.folders, mr, ov, &lifecycle.reindex, session, &lifecycle.flycheck);
        super::protocol::send_response(writer, &json!({ "jsonrpc": "2.0", "id": msg["id"], "result": result }))?;
        return Ok(None);
    }
    // An index published since the run failed (by `ridx index` run by hand) answers: the refusal is judged on the
    // indexes refreshed first (review of b3d0d293).
    if index_failed(&msg, mr, &lifecycle.reindex).is_some() {
        refresh(gate, writer, lifecycle, mr, ov)?;
        if let Some(answer) = refusal(&msg, mr, &lifecycle.reindex) {
            super::protocol::send_response(writer, &answer)?;
            return Ok(None);
        }
    }
    if super::fresh::wanted(&msg).is_none() {
        return Ok(Some(msg));
    }
    refresh(gate, writer, lifecycle, mr, ov)?;
    msg[PINNED] = json!(true);
    let ov: &crate::overlay::OverlayManager = ov;
    let fc = &lifecycle.flycheck;
    let cov = Coverage { ov, indexed_hash: &|f| answering_hash(mr, f), diagnostics: &|f| diagnostics_of(fc, ov, mr, &lifecycle.reindex, f) };
    // A `textDocument/diagnostic` whose version compiled without diagnostics is told so now (prod-code, m=20178).
    if let Some(answer) = super::fresh::unavailable(&msg, &cov) {
        super::protocol::send_response(writer, &answer)?;
        return Ok(None);
    }
    demand(&msg, &cov, mr, &lifecycle.reindex, session, true);
    Ok(gate.waiting.admit(msg, &cov, deadline))
}

/// A pull waiting for the diagnostics of a buffer as saved has the sessions compile it: only a compile of its very
/// text, in its own workspace, says what they are (review of 13eacef0). `again`: a session is due even when the buffer
/// is wanted there already (a request asking again retries a build that failed); else only when it is not.
fn demand(msg: &Value, cov: &Coverage, mr: &super::MultiRootWorkspace, reindex: &super::reindex::Reindexer, session: &SessionIndexer, again: bool) {
    if msg["method"] != "textDocument/diagnostic" {
        return;
    }
    let Some((file, _)) = super::fresh::wanted(msg).filter(|(f, _)| cov.ov.get(f).is_some() && (cov.diagnostics)(f) == Readiness::Pending) else { return };
    let Some(root) = owning_root(mr, reindex, &file) else { return };
    let file = ridx_format::canonicalize_best_effort(Path::new(&file));
    if again || !session.is_wanted(&root, &file) {
        session.want(&file, &root);
    }
}

/// The requests answered from the index alone: on a root without one, an empty answer would read as "none" (no
/// references, no definition). Hover, completion, symbols, formatting and the rest also answer from the syntax tree.
const INDEX_ONLY: &[&str] = &[
    "textDocument/definition",
    "textDocument/declaration",
    "textDocument/typeDefinition",
    "textDocument/implementation",
    "textDocument/references",
    "textDocument/rename",
    "textDocument/prepareRename",
    "textDocument/prepareCallHierarchy",
    "textDocument/prepareTypeHierarchy",
];

/// The root and error when `msg` asks the index of a root that has none loaded because its build failed (prod-code,
/// m=19837: `references` answered `[]` there, which reads as no references). A root with an older index loaded answers
/// from it; one still building its first index is answered as before (`minVersion` waits for it).
/// The root is the innermost of the roots loaded and those indexed this session holding the file: a loaded outer
/// workspace does not hide a nested one whose index failed, and a root counts as loaded by the index bound to it, not
/// by the workspace root that index records (review of b3d0d293).
fn index_failed(msg: &Value, mr: &super::MultiRootWorkspace, reindex: &super::reindex::Reindexer) -> Option<(PathBuf, String)> {
    if !INDEX_ONLY.contains(&msg["method"].as_str()?) {
        return None;
    }
    let file = ridx_format::canonicalize_best_effort(Path::new(&super::protocol::uri_to_path(msg["params"]["textDocument"]["uri"].as_str()?)));
    // The roots indexes are bound to, as the index answering a file is chosen: one opened as `W/target/ridx` is bound
    // to `W` (`MultiRootWorkspace::add_root`; reviews of 11a609ef and 3e51faf9).
    let loaded: Vec<PathBuf> = mr.roots().map(Path::to_path_buf).collect();
    let root = loaded.iter().cloned().chain(reindex.roots()).filter(|r| file.starts_with(r)).max_by_key(|r| r.as_os_str().len())?;
    if loaded.contains(&root) {
        return None;
    }
    match reindex.status_of(&root) {
        (false, Some(error)) => Some((root, error)),
        _ => None,
    }
}

/// The `-32803` answer to `msg` when `index_failed`.
fn refusal(msg: &Value, mr: &super::MultiRootWorkspace, reindex: &super::reindex::Reindexer) -> Option<Value> {
    let (root, error) = index_failed(msg, mr, reindex)?;
    let data = json!({ "reason": "indexFailed", "root": root.display().to_string(), "error": error });
    Some(super::fresh::response(&msg["id"], -32803, format!("no index of {}: its build failed", root.display()), data))
}

/// The workspace whose sessions say what the diagnostics of `file` are: the innermost root of the session holding it,
/// an index loaded for it or not (a root registered and indexed for the first time counts: review of 7501440b, a
/// loaded enclosing workspace was taken), else `dispatch::workspace_root_of`.
pub(crate) fn owning_root(mr: &super::MultiRootWorkspace, reindex: &super::reindex::Reindexer, file: &str) -> Option<PathBuf> {
    let file = ridx_format::canonicalize_best_effort(Path::new(file));
    let roots = mr.roots().map(Path::to_path_buf).chain(reindex.roots()).map(|r| ridx_format::canonicalize_best_effort(&r));
    roots.filter(|r| file.starts_with(r)).max_by_key(|r| r.as_os_str().len()).or_else(|| super::dispatch::workspace_root_of(mr, reindex, &file))
}

/// The [`Readiness`] of the diagnostics of `file`, from a session of the workspace it belongs to ([`owning_root`]).
fn diagnostics_of(
    fc: &crate::lsp::presentation::flycheck::FlycheckState,
    ov: &crate::overlay::OverlayManager,
    mr: &super::MultiRootWorkspace,
    reindex: &super::reindex::Reindexer,
    file: &str,
) -> Readiness {
    owning_root(mr, reindex, file).map_or(Readiness::Pending, |root| readiness(fc, ov, file, &root))
}

/// What `initialize` answers in `capabilities.experimental`: the `ridx/*` methods and request options this server
/// supports, each with the version of its contract (docs/integration.md §7), for clients to detect them.
///
/// `textDocument/diagnostic: 1` is the contract of a pull with `minVersion: N`: the compiler's diagnostics of version
/// `N`, `[]` only when that compile found nothing, else `-32803` (`diagnosticsUnavailable`, or `timeout`). 0.3.10 and
/// 0.3.11 declared `minVersion` with a pull answered once the index covered the version, which a tool could not tell
/// from this (prod-code, m=20197).
pub fn experimental_capabilities() -> Value {
    json!({ "ridx": { "methods": { "ridx/expandMacro": 1, "ridx/safeDelete": 1, "ridx/status": 1, "ridx/syntaxTree": 1, "textDocument/diagnostic": 1 }, "requestOptions": { "minVersion": 1 } } })
}

#[path = "fresh_status.rs"]
mod fresh_status;
pub use fresh_status::status;

#[cfg(test)]
#[path = "fresh_loop_tests.rs"]
mod index_failed_tests;

#[cfg(test)]
mod tests {
    use super::*;

    /// prod-code (m=19778): clients detect the `ridx/*` methods from `initialize`.
    #[test]
    fn test_initialize_lists_the_ridx_methods() {
        let mut lc = crate::lsp::lifecycle::LifecycleManager::new();
        let (caps, _) = lc.handle_initialize(&json!({ "id": 1, "method": "initialize", "params": { "capabilities": {} } })).unwrap();
        let ridx = &caps["capabilities"]["experimental"]["ridx"];
        assert_eq!(ridx["methods"]["ridx/status"], 1, "{caps}");
        assert_eq!(ridx["methods"]["ridx/safeDelete"], 1, "{caps}");
        assert_eq!(ridx["methods"]["ridx/syntaxTree"], 1, "{caps}");
        assert_eq!(ridx["methods"]["ridx/expandMacro"], 1, "{caps}");
        assert_eq!(ridx["requestOptions"]["minVersion"], 1, "{caps}");
        // prod-code (m=20197): the pull's contract, which 0.3.10 and 0.3.11 did not have.
        assert_eq!(ridx["methods"]["textDocument/diagnostic"], 1, "{caps}");
    }

    /// prod-code (m=19837): with `ridx-driver` missing the first index failed, and `references` answered `[]`, which
    /// reads as "no references". A request the index alone answers is refused there, with the reason.
    #[test]
    fn test_a_request_on_a_root_whose_index_failed_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let root = ridx_format::canonicalize_best_effort(dir.path());
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"w\"\nversion = \"0.1.0\"\n").unwrap();
        std::fs::write(root.join("src/lib.rs"), "pub fn a() {}\n").unwrap();
        let mr = crate::lsp::MultiRootWorkspace::new();
        let reindex = crate::lsp::reindex::Reindexer::new();
        let uri = format!("file://{}", root.join("src/lib.rs").display());
        let ask = |method: &str| json!({ "id": 1, "method": method, "params": { "textDocument": { "uri": uri }, "position": { "line": 0, "character": 7 } } });
        assert!(index_failed(&ask("textDocument/references"), &mr, &reindex).is_none(), "no run failed");

        reindex.fail_for_test(&root, "ridx index failed: error: ridx-driver not found");
        let (failed_root, error) = index_failed(&ask("textDocument/references"), &mr, &reindex).expect("refused");
        assert_eq!(failed_root, root);
        assert!(error.contains("ridx-driver"), "{error}");
        assert!(index_failed(&ask("textDocument/definition"), &mr, &reindex).is_some());
        assert!(index_failed(&ask("textDocument/documentSymbol"), &mr, &reindex).is_none(), "answered from the syntax tree");
        assert!(index_failed(&ask("textDocument/hover"), &mr, &reindex).is_none(), "answered from the syntax tree too");
    }

    /// prod-code (m=19837): the reason a run failed is the last `error` line of its output.
    #[test]
    fn test_a_failed_run_says_why() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("reindex.log");
        std::fs::write(&log, "   Compiling w\nerror: could not find `ridx-driver` next to ridx\nnote: install it\n").unwrap();
        let reason = crate::lsp::reindex::failure_reason(&log);
        assert!(reason.contains("error: could not find `ridx-driver` next to ridx"), "{reason}");
        assert!(reason.contains("reindex.log"), "{reason}");
    }
}
