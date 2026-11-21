/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! What a session's compile of each buffer gave (`Outcome`), the hint for a buffer no crate includes, and whether a
//! request waiting for a buffer's diagnostics can be answered (`Readiness`).

use super::{Outcome, SessionFile};
use crate::indexer::session_report::CompilerReport;
use crate::lsp::presentation::flycheck::FlycheckState;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The outcome of a buffer no crate includes.
pub const UNLINKED: &str = "notInAnyCrate";

/// The hint for a file no crate includes, the one rust-analyzer gives (`unlinked-file`), when the session answering
/// for it (`out`, from `SessionDiagnostics::outcome_root`) found `live`, the buffer's text, in none: in place of an
/// empty list, which reads as a file without errors (prod-code, m=20185). It is of that compile alone, the disk is not
/// read again: a pull admitted on it keeps it whatever is written to the file after (review of d6df3ce0).
pub fn unlinked_hint(out: &Outcome, live: &str) -> Option<Value> {
    (out.unavailable == Some(UNLINKED) && out.text == live).then(|| {
        json!({
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } },
            "severity": 4,
            "code": "unlinked-file",
            "source": "ridx",
            "message": "This file is not included in any crates, so ridx can't offer IDE services."
        })
    })
}

/// The outcome of each buffer of a session (`buffers` compiled, `files` from `session_files`): its diagnostics when it
/// has them, else why not.
pub fn session_outcomes(report: &CompilerReport, buffers: &[(PathBuf, String)], files: &HashMap<PathBuf, SessionFile>) -> HashMap<PathBuf, Outcome> {
    buffers
        .iter()
        .map(|(p, text)| {
            let path = ridx_format::canonicalize_best_effort(p);
            // A test target not compiled stays unknown whatever another unit including the file as a module reported:
            // that compile says nothing of the target's (review of c6b62a99).
            let unavailable = if report.unresolved.contains(&path) {
                Some("targetUnresolved")
            } else if files.contains_key(&path) {
                None
            } else if report.unheld.contains(&path) {
                Some(UNLINKED)
            } else if report.untold.contains(&path) {
                Some("pathsUntold")
            } else {
                Some("notChecked")
            };
            (path, Outcome { text: text.clone(), unavailable })
        })
        .collect()
}

/// Whether the compiler's diagnostics of an open buffer's text are known, for a request that waits for them
/// (`textDocument/diagnostic` with `minVersion`, prod-code m=20178: the answer before them read as clean).
#[derive(Debug, PartialEq)]
pub enum Readiness {
    /// Known, for this version of the buffer.
    Ready(i32),
    /// The compile of this version gave none, for this reason (see [`Outcome`]).
    Unavailable(i32, &'static str),
    /// Not known yet.
    Pending,
}

/// The [`Readiness`] of the diagnostics of `file` (the overlay key) of workspace `root`: ready once a session of that
/// workspace compiled the buffer's very text, edited or as saved (a buffer as saved is compiled for a request waiting
/// for it, `SessionIndexer::want`). The check of the saved files never stands in: that it ran says nothing of the text
/// it read, of whether it failed before the file, or of which workspace it was (review of 13eacef0). A file not open
/// has no version and is ready.
pub fn readiness(fc: &FlycheckState, ov: &crate::overlay::OverlayManager, file: &str, root: &Path) -> Readiness {
    let Some(o) = ov.get(file) else { return Readiness::Ready(0) };
    let path = ridx_format::canonicalize_best_effort(Path::new(file));
    match fc.session.outcome_in(root, &path) {
        Some(out) if out.text == o.text => match out.unavailable {
            None => Readiness::Ready(o.version),
            // A file as saved no crate includes is answered, with the hint saying so (`unlinked_hint`, given for the
            // compiled text whatever the disk holds after), as rust-analyzer does; an edited one is refused, its edit
            // validates nothing (prod-code, m=20185).
            Some(UNLINKED) if crate::lsp::presentation::reconcile::saved_text(ov, file).as_deref() == Some(o.text.as_str()) => Readiness::Ready(o.version),
            Some(why) => Readiness::Unavailable(o.version, why),
        },
        // No session compiled this text yet (a session's result of another text answers for it until the next one).
        _ => Readiness::Pending,
    }
}
