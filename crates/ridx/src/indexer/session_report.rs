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

/// The file on disk a remapped `name` stands for: a source the compiler would have remapped to `name` (the last remap
/// whose source prefix it has is the one giving `name`), and when several would, the one that exists. `Err` when that
/// leaves more than one, or none of several (review of 68375e6e: two sources remapped to one prefix).
fn local_name(name: &Path, remaps: &[(PathBuf, PathBuf)]) -> Result<Option<PathBuf>, ()> {
    let mut candidates: Vec<PathBuf> = remaps
        .iter()
        .enumerate()
        .filter_map(|(i, (from, to))| {
            let local = from.join(name.strip_prefix(to).ok()?);
            // `remaps` is last first: this one applies to `local` only if no later one does.
            (remaps.iter().position(|(f, _)| local.starts_with(f)) == Some(i)).then_some(local)
        })
        .collect();
    candidates.dedup();
    match candidates.len() {
        0 => Ok(None),
        1 => Ok(candidates.pop()),
        _ => {
            let existing: Vec<PathBuf> = candidates.into_iter().filter(|c| c.is_file()).collect();
            if existing.len() == 1 {
                Ok(existing.into_iter().next())
            } else {
                Err(())
            }
        }
    }
}

/// `messages` with every file a span names (its expansions' and its children's too) named as on disk, where the unit
/// remapped it (review of 2e1fcfb1: a remapped file read as one outside the workspace). Returns whether every one could
/// be told for sure; a name that could not is left as it is.
pub(super) fn unmap(messages: &mut [Value], remaps: &[(PathBuf, PathBuf)]) -> bool {
    if remaps.is_empty() {
        return true;
    }
    fn walk(v: &mut Value, remaps: &[(PathBuf, PathBuf)], sure: &mut bool) {
        match v {
            Value::Object(map) => {
                if let Some(Value::String(name)) = map.get_mut("file_name") {
                    match local_name(Path::new(name.as_str()), remaps) {
                        Ok(Some(local)) => *name = local.display().to_string(),
                        Ok(None) => {}
                        Err(()) => *sure = false,
                    }
                }
                map.values_mut().for_each(|c| walk(c, remaps, sure));
            }
            Value::Array(items) => items.iter_mut().for_each(|c| walk(c, remaps, sure)),
            _ => {}
        }
    }
    let mut sure = true;
    messages.iter_mut().for_each(|m| walk(m, remaps, &mut sure));
    sure
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

    /// Review of 68375e6e: `A=/m` then `B=/m` named A's file as B's, and the error left the workspace.
    #[test]
    fn test_two_sources_remapped_to_one_prefix_are_told_by_the_file_that_exists() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("A"), dir.path().join("B"));
        std::fs::create_dir_all(a.join("src")).unwrap();
        std::fs::write(a.join("src/lib.rs"), "x").unwrap();
        let args: Vec<String> = ["rustc".to_string(), format!("--remap-path-prefix={}=/m", a.display()), format!("--remap-path-prefix={}=/m", b.display())].into();
        let mut m = vec![serde_json::json!({ "spans": [{ "file_name": "/m/src/lib.rs" }] })];
        assert!(unmap(&mut m, &remaps(&args)));
        assert_eq!(m[0]["spans"][0]["file_name"], a.join("src/lib.rs").display().to_string());
        std::fs::create_dir_all(b.join("src")).unwrap();
        std::fs::write(b.join("src/lib.rs"), "y").unwrap();
        let mut m = vec![serde_json::json!({ "spans": [{ "file_name": "/m/src/lib.rs" }] })];
        assert!(!unmap(&mut m, &remaps(&args)), "both exist: not sure, so nothing is certified");
    }
}
