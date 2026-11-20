/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! A running `ridx index` in the client's status bar (LSP work-done progress), as rust-analyzer shows its own
//! indexing. In Zed a project being indexed answered nothing and showed no sign of why.

use super::protocol::{send_response, send_work_done_progress};
use serde_json::json;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// The progress shown for the roots being indexed.
#[derive(Default)]
pub struct IndexProgress {
    token: Option<String>,
    shown: Vec<PathBuf>,
    /// Every root indexed under the current token, to say at its end which failed.
    ran: Vec<PathBuf>,
}

impl IndexProgress {
    /// Brings the client's progress in line with the roots indexed now: it begins when a run starts, names the roots
    /// while they change and ends when the last run ends. `next_id` numbers the server's requests. The end names the roots
    /// whose run failed (`failed`: the error of a root's last run), so `end` alone is not read as an index loaded (prod-code,
    /// m=19837). `ended` are the runs that ended since the last call: one that started and failed between two calls never
    /// shows in `running`, and still counts (review of b3d0d293); with no progress open, it is shown begun and ended.
    pub fn sync<W: Write>(
        &mut self,
        writer: &mut W,
        running: Vec<PathBuf>,
        ended: Vec<PathBuf>,
        next_id: impl Fn() -> String,
        failed: impl Fn(&Path) -> Option<String>,
    ) -> io::Result<()> {
        let unseen_failure = self.token.is_none() && running.is_empty() && ended.iter().any(|r| failed(r).is_some());
        // With nothing to show, the runs that ended do not belong to any progress: none is kept for a later one's end.
        let keep_ended = self.token.is_some() || !running.is_empty() || unseen_failure;
        for root in running.iter().chain(ended.iter().filter(|_| keep_ended)) {
            if !self.ran.contains(root) {
                self.ran.push(root.clone());
            }
        }
        if unseen_failure {
            let names = ended.iter().map(|r| name_of(r)).collect::<Vec<_>>().join(", ");
            self.begin(writer, &names, &next_id)?;
        }
        if running == self.shown && !(running.is_empty() && self.token.is_some()) {
            return Ok(());
        }
        let names = running.iter().map(|r| name_of(r)).collect::<Vec<_>>().join(", ");
        match (&self.token, running.is_empty()) {
            (None, false) => self.begin(writer, &names, &next_id)?,
            (Some(token), false) => send_work_done_progress(writer, &json!(token), "report", None, Some(&names), None)?,
            (Some(token), true) => {
                let failures: Vec<String> = self.ran.iter().filter_map(|r| failed(r).map(|e| format!("{}: {}", name_of(r), e.lines().next().unwrap_or("")))).collect();
                let message = (!failures.is_empty()).then(|| format!("Indexing failed: {}", failures.join("; ")));
                send_work_done_progress(writer, &json!(token), "end", None, message.as_deref(), None)?;
                self.token = None;
                self.ran.clear();
            }
            (None, true) => {}
        }
        self.shown = running;
        Ok(())
    }

    fn begin<W: Write>(&mut self, writer: &mut W, names: &str, next_id: &impl Fn() -> String) -> io::Result<()> {
        let token = format!("ridx-index-{}", next_id());
        send_response(writer, &json!({ "jsonrpc": "2.0", "id": next_id(), "method": "window/workDoneProgress/create", "params": { "token": token } }))?;
        send_work_done_progress(writer, &json!(token), "begin", Some("Indexing"), Some(names), None)?;
        self.token = Some(token);
        Ok(())
    }
}

fn name_of(root: &Path) -> String {
    root.file_name().map_or_else(|| root.display().to_string(), |n| n.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn messages(out: &[u8]) -> Vec<serde_json::Value> {
        let text = String::from_utf8_lossy(out);
        text.split("Content-Length:").filter_map(|m| m.find('{').map(|i| serde_json::from_str(m[i..].trim()).unwrap())).collect()
    }

    #[test]
    fn test_indexing_shows_in_the_status_bar_until_it_ends() {
        let mut p = IndexProgress::default();
        let n = std::cell::Cell::new(0);
        let next = || {
            n.set(n.get() + 1);
            format!("r{}", n.get())
        };
        let mut out = Vec::new();
        p.sync(&mut out, vec![], vec![], next, |_| None).unwrap();
        assert!(out.is_empty(), "nothing runs");

        p.sync(&mut out, vec![PathBuf::from("/w/hl-order-book-server")], vec![], next, |_| None).unwrap();
        p.sync(&mut out, vec![PathBuf::from("/w/hl-order-book-server")], vec![], next, |_| None).unwrap();
        p.sync(&mut out, vec![PathBuf::from("/w/BTCR"), PathBuf::from("/w/hl-order-book-server")], vec![], next, |_| None).unwrap();
        p.sync(&mut out, vec![], vec![], next, |_| None).unwrap();
        let m = messages(&out);
        assert_eq!(m.len(), 4, "{m:?}");
        assert_eq!(m[0]["method"], "window/workDoneProgress/create");
        let token = m[0]["params"]["token"].clone();
        assert_eq!((m[1]["params"]["token"].clone(), m[1]["params"]["value"]["kind"].as_str()), (token.clone(), Some("begin")));
        assert_eq!(m[1]["params"]["value"]["title"], "Indexing");
        assert_eq!(m[1]["params"]["value"]["message"], "hl-order-book-server");
        assert_eq!(m[2]["params"]["value"]["message"], "BTCR, hl-order-book-server", "a second root while the first runs");
        assert_eq!((m[3]["params"]["token"].clone(), m[3]["params"]["value"]["kind"].as_str()), (token, Some("end")));

        p.sync(&mut out, vec![PathBuf::from("/w/BTCR")], vec![], next, |_| None).unwrap();
        let m = messages(&out);
        assert_eq!(m[4]["method"], "window/workDoneProgress/create", "a later run shows again");
        assert_ne!(m[4]["params"]["token"], m[0]["params"]["token"]);
    }

    /// prod-code (m=19837): `end` came the same after a failed run; it names the roots that failed.
    #[test]
    fn test_the_end_names_the_roots_whose_run_failed() {
        let mut p = IndexProgress::default();
        let next = || "r".to_string();
        let mut out = Vec::new();
        let failed = |r: &Path| (r == Path::new("/w/b")).then(|| "ridx index failed: error: no driver\nmore".to_string());
        p.sync(&mut out, vec![PathBuf::from("/w/a"), PathBuf::from("/w/b")], vec![], next, failed).unwrap();
        p.sync(&mut out, vec![PathBuf::from("/w/a")], vec![], next, failed).unwrap();
        p.sync(&mut out, vec![], vec![], next, failed).unwrap();
        let end = messages(&out).pop().unwrap();
        assert_eq!(end["params"]["value"]["kind"], "end");
        assert_eq!(end["params"]["value"]["message"], "Indexing failed: b: ridx index failed: error: no driver", "a root that ended earlier counts too");

        p.sync(&mut out, vec![PathBuf::from("/w/a")], vec![], next, |_| None).unwrap();
        p.sync(&mut out, vec![], vec![], next, |_| None).unwrap();
        assert!(messages(&out).pop().unwrap()["params"]["value"].get("message").is_none(), "a run that succeeded ends plainly");
    }

    /// Review of b3d0d293: a run of B starting and failing between two looks at `running` never showed in it, and the
    /// end under A's token said nothing; with nothing running, the failure did not show at all.
    #[test]
    fn test_a_run_failing_between_two_looks_counts() {
        let mut p = IndexProgress::default();
        let next = || "r".to_string();
        let failed = |r: &Path| (r == Path::new("/w/b")).then(|| "no driver".to_string());
        let mut out = Vec::new();
        p.sync(&mut out, vec![PathBuf::from("/w/a")], vec![], next, failed).unwrap();
        p.sync(&mut out, vec![PathBuf::from("/w/a")], vec![PathBuf::from("/w/b")], next, failed).unwrap();
        p.sync(&mut out, vec![], vec![PathBuf::from("/w/a")], next, failed).unwrap();
        assert_eq!(messages(&out).pop().unwrap()["params"]["value"]["message"], "Indexing failed: b: no driver");

        let mut out = Vec::new();
        p.sync(&mut out, vec![], vec![PathBuf::from("/w/b")], next, failed).unwrap();
        let m = messages(&out);
        let kinds: Vec<_> = m.iter().filter_map(|m| m["params"]["value"]["kind"].as_str()).collect();
        assert_eq!(kinds, ["begin", "end"], "nothing was running: begun and ended at once: {m:?}");
        assert_eq!(m.last().unwrap()["params"]["value"]["message"], "Indexing failed: b: no driver");

        let mut out = Vec::new();
        p.sync(&mut out, vec![], vec![PathBuf::from("/w/a")], next, failed).unwrap();
        assert!(out.is_empty(), "a run that succeeded unseen shows nothing");
        p.sync(&mut out, vec![PathBuf::from("/w/c")], vec![], next, |r: &Path| (r == Path::new("/w/a")).then(|| "later".to_string())).unwrap();
        p.sync(&mut out, vec![], vec![PathBuf::from("/w/c")], next, |r: &Path| (r == Path::new("/w/a")).then(|| "later".to_string())).unwrap();
        assert!(messages(&out).pop().unwrap()["params"]["value"].get("message").is_none(), "a run outside the progress is not in its end");
    }
}
