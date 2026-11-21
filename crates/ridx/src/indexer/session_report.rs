/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! What the compiler reported in the compiles of a session index (`session`): its diagnostics in rustc's JSON, with
//! the files named as on disk, and the files it is known to have checked, for which no diagnostic means none.

use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// The compiler's report of a session.
#[derive(Debug, Default)]
pub struct CompilerReport {
    /// rustc's JSON diagnostics, of every unit, in the order the units ran.
    pub messages: Vec<Value>,
    /// The sources (canonical) of the units that ran their analysis to the end: every error and lint in them is in
    /// `messages`. A unit that stopped first (a parse error, a driver that failed) certifies none of its files.
    pub checked: HashSet<PathBuf>,
    /// When the session started reading the files: a file on disk written since was not the text compiled.
    pub started: Option<SystemTime>,
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

/// The `--remap-path-prefix FROM=TO` of `args`, the last first: rustc applies the last one that matches.
pub(super) fn remaps(args: &[String]) -> Vec<(PathBuf, PathBuf)> {
    let mut out = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let spec = if a == "--remap-path-prefix" { it.next().map(String::as_str) } else { a.strip_prefix("--remap-path-prefix=") };
        if let Some((from, to)) = spec.and_then(|s| s.split_once('=')) {
            out.push((PathBuf::from(from), PathBuf::from(to)));
        }
    }
    out.reverse();
    out
}

/// `messages` with every file a span names (its expansions' and its children's too) named as on disk, where the unit
/// remapped it (review of 2e1fcfb1: a remapped file read as one outside the workspace).
pub(super) fn unmap(messages: &mut [Value], remaps: &[(PathBuf, PathBuf)]) {
    if remaps.is_empty() {
        return;
    }
    fn walk(v: &mut Value, remaps: &[(PathBuf, PathBuf)]) {
        match v {
            Value::Object(map) => {
                if let Some(Value::String(name)) = map.get_mut("file_name") {
                    if let Some(local) = remaps.iter().find_map(|(from, to)| Path::new(name.as_str()).strip_prefix(to).ok().map(|rest| from.join(rest))) {
                        *name = local.display().to_string();
                    }
                }
                map.values_mut().for_each(|c| walk(c, remaps));
            }
            Value::Array(items) => items.iter_mut().for_each(|c| walk(c, remaps)),
            _ => {}
        }
    }
    messages.iter_mut().for_each(|m| walk(m, remaps));
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

    /// Review of 2e1fcfb1: with `--remap-path-prefix`, rustc names the remapped file.
    #[test]
    fn test_remapped_files_are_named_as_on_disk() {
        let args: Vec<String> = ["rustc", "--remap-path-prefix", "/w=/src", "--remap-path-prefix=/w/sub=/other"].map(String::from).into();
        let mut m = vec![serde_json::json!({ "spans": [{ "file_name": "/other/a.rs", "expansion": { "span": { "file_name": "/src/m.rs" } } }],
                                               "children": [{ "spans": [{ "file_name": "/elsewhere/x.rs" }] }] })];
        unmap(&mut m, &remaps(&args));
        assert_eq!(m[0]["spans"][0]["file_name"], "/w/sub/a.rs", "the last remap that matches");
        assert_eq!(m[0]["spans"][0]["expansion"]["span"]["file_name"], "/w/m.rs");
        assert_eq!(m[0]["children"][0]["spans"][0]["file_name"], "/elsewhere/x.rs");
    }
}
