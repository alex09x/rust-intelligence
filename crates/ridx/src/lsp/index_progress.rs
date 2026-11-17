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
use std::path::PathBuf;

/// The progress shown for the roots being indexed.
#[derive(Default)]
pub struct IndexProgress {
    token: Option<String>,
    shown: Vec<PathBuf>,
}

impl IndexProgress {
    /// Brings the client's progress in line with the roots indexed now: it begins when a run starts, names the roots
    /// while they change and ends when the last run ends. `next_id` numbers the server's requests.
    pub fn sync<W: Write>(&mut self, writer: &mut W, running: Vec<PathBuf>, next_id: impl Fn() -> String) -> io::Result<()> {
        if running == self.shown {
            return Ok(());
        }
        let names = running.iter().map(|r| r.file_name().map_or_else(|| r.display().to_string(), |n| n.to_string_lossy().into_owned())).collect::<Vec<_>>().join(", ");
        match (&self.token, running.is_empty()) {
            (None, false) => {
                let token = format!("ridx-index-{}", next_id());
                send_response(writer, &json!({ "jsonrpc": "2.0", "id": next_id(), "method": "window/workDoneProgress/create", "params": { "token": token } }))?;
                send_work_done_progress(writer, &json!(token), "begin", Some("Indexing"), Some(&names), None)?;
                self.token = Some(token);
            }
            (Some(token), false) => send_work_done_progress(writer, &json!(token), "report", None, Some(&names), None)?,
            (Some(token), true) => {
                send_work_done_progress(writer, &json!(token), "end", None, None, None)?;
                self.token = None;
            }
            (None, true) => {}
        }
        self.shown = running;
        Ok(())
    }
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
        p.sync(&mut out, vec![], next).unwrap();
        assert!(out.is_empty(), "nothing runs");

        p.sync(&mut out, vec![PathBuf::from("/w/hl-order-book-server")], next).unwrap();
        p.sync(&mut out, vec![PathBuf::from("/w/hl-order-book-server")], next).unwrap();
        p.sync(&mut out, vec![PathBuf::from("/w/BTCR"), PathBuf::from("/w/hl-order-book-server")], next).unwrap();
        p.sync(&mut out, vec![], next).unwrap();
        let m = messages(&out);
        assert_eq!(m.len(), 4, "{m:?}");
        assert_eq!(m[0]["method"], "window/workDoneProgress/create");
        let token = m[0]["params"]["token"].clone();
        assert_eq!((m[1]["params"]["token"].clone(), m[1]["params"]["value"]["kind"].as_str()), (token.clone(), Some("begin")));
        assert_eq!(m[1]["params"]["value"]["title"], "Indexing");
        assert_eq!(m[1]["params"]["value"]["message"], "hl-order-book-server");
        assert_eq!(m[2]["params"]["value"]["message"], "BTCR, hl-order-book-server", "a second root while the first runs");
        assert_eq!((m[3]["params"]["token"].clone(), m[3]["params"]["value"]["kind"].as_str()), (token, Some("end")));

        p.sync(&mut out, vec![PathBuf::from("/w/BTCR")], next).unwrap();
        let m = messages(&out);
        assert_eq!(m[4]["method"], "window/workDoneProgress/create", "a later run shows again");
        assert_ne!(m[4]["params"]["token"], m[0]["params"]["token"]);
    }
}
