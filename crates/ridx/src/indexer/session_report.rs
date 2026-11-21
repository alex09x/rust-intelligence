/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! What the compiler reported in the compiles of a session index (`session`): its diagnostics in rustc's JSON, and the
//! files it is known to have checked, for which no diagnostic means none.

use serde_json::Value;
use std::collections::HashSet;
use std::path::PathBuf;

/// The compiler's report of a session.
#[derive(Debug, Default)]
pub struct CompilerReport {
    /// rustc's JSON diagnostics, of every unit, in the order the units ran.
    pub messages: Vec<Value>,
    /// The sources (canonical) of the units that ran their analysis to the end: every error and lint in them is in
    /// `messages`. A unit that stopped first (a parse error, a driver that failed) certifies none of its files.
    pub checked: HashSet<PathBuf>,
}

/// `args` reporting in rustc's JSON, and only its diagnostics: the format and colour the unit was recorded with go.
pub(super) fn json_diagnostics(args: Vec<String>) -> Vec<String> {
    let mut out = Vec::with_capacity(args.len() + 1);
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        if matches!(a.as_str(), "--error-format" | "--json" | "--color") {
            it.next();
        } else if !(a.starts_with("--error-format=") || a.starts_with("--json=") || a.starts_with("--color=")) {
            out.push(a);
        }
    }
    out.push("--error-format=json".into());
    out
}

/// The diagnostics among the lines the compiler wrote to its standard error (`$message_type` "diagnostic"). Each line
/// is read on its own, as bytes: one that is not JSON, or not UTF-8 (a proc macro printing anything), costs only itself
/// (review of 2e1fcfb1).
pub(super) fn diagnostics_of(stderr: &[u8]) -> Vec<Value> {
    stderr
        .split(|&b| b == b'\n')
        .filter_map(|l| serde_json::from_slice::<Value>(l).ok())
        .filter(|m| m.get("$message_type").is_none_or(|t| t == "diagnostic") && m.get("spans").is_some())
        .collect()
}

/// Whether `args` remap source paths (`--remap-path-prefix`): the files its diagnostics name are then not the files
/// compiled, and which ones they stand for cannot be told for sure (reviews of 2e1fcfb1 to 710381e0), so a unit
/// compiled so reports nothing of its own and certifies nothing; its files keep the check of the saved files.
pub(super) fn remaps_paths(args: &[String]) -> bool {
    args.iter().any(|a| a == "--remap-path-prefix" || a.starts_with("--remap-path-prefix="))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_the_recorded_format_gives_way_to_json() {
        let args: Vec<String> = ["rustc", "--error-format", "human", "--json=diagnostic-rendered-ansi,artifacts", "--color", "always", "--color=never", "--edition=2021", "lib.rs"]
            .map(String::from)
            .into();
        assert_eq!(json_diagnostics(args), ["rustc", "--edition=2021", "lib.rs", "--error-format=json"]);
    }

    /// Review of 2e1fcfb1: a line that is not UTF-8 lost every diagnostic of the unit.
    #[test]
    fn test_each_line_is_read_on_its_own() {
        let mut stderr = b"warning: \xff not utf-8\n".to_vec();
        stderr.extend_from_slice(br#"{"$message_type":"diagnostic","message":"m","spans":[]}"#);
        stderr.extend_from_slice(b"\n{\"$message_type\":\"artifact\",\"artifact\":\"x\"}\n");
        let d = diagnostics_of(&stderr);
        assert_eq!(d.len(), 1, "{d:?}");
    }

    #[test]
    fn test_a_remapping_unit_is_told() {
        let args = |a: &[&str]| -> Vec<String> { a.iter().map(|s| s.to_string()).collect() };
        assert!(remaps_paths(&args(&["rustc", "--remap-path-prefix", "/w=/src"])));
        assert!(remaps_paths(&args(&["rustc", "--remap-path-prefix=/w=/src"])));
        assert!(!remaps_paths(&args(&["rustc", "--edition=2021"])));
    }
}
