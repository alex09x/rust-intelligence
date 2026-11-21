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

/// The source a file `name` in a diagnostic stands for. A name no remap gives is the file itself. Otherwise each source
/// the compiler would have named so: a remapped one (the last remap whose source prefix it has is the one giving
/// `name`), or `name` itself when no remap applies to it, among the sources the unit is known to have read (`known`,
/// canonical: its buffers and what its sidecar records); not the files that happen to exist (a buffer exists nowhere
/// on disk, review of 46ce96f2). Only one such source tells it; with none or several, `None`.
fn local_name(name: &Path, remaps: &[(PathBuf, PathBuf)], known: &HashSet<PathBuf>) -> Option<PathBuf> {
    if remaps.iter().all(|(_, to)| !name.starts_with(to)) {
        return Some(name.to_path_buf());
    }
    let mut candidates: Vec<PathBuf> = remaps
        .iter()
        .enumerate()
        .filter_map(|(i, (from, to))| {
            let local = from.join(name.strip_prefix(to).ok()?);
            // `remaps` is last first: this one applies to `local` only if no later one does.
            (remaps.iter().position(|(f, _)| local.starts_with(f)) == Some(i)).then_some(local)
        })
        .collect();
    if remaps.iter().all(|(from, _)| !name.starts_with(from)) {
        candidates.push(name.to_path_buf());
    }
    candidates.retain(|c| known.contains(&super::session::canonical(c)));
    candidates.sort();
    candidates.dedup();
    (candidates.len() == 1).then(|| candidates.remove(0))
}

/// Names `span`'s file, and its expansions' (where the macro was called), as on disk; `false` when one of them cannot be.
fn resolve_span(span: &mut Value, remaps: &[(PathBuf, PathBuf)], known: &HashSet<PathBuf>) -> bool {
    if let Some(Value::String(name)) = span.get_mut("file_name") {
        match local_name(Path::new(name.as_str()), remaps, known) {
            Some(local) => *name = local.display().to_string(),
            None => return false,
        }
    }
    let expansion_ok = match span.get_mut("expansion") {
        Some(Value::Object(exp)) => ["span", "def_site_span"].iter().all(|k| exp.get_mut(*k).is_none_or(|s| s.is_null() || resolve_span(s, remaps, known))),
        _ => true,
    };
    expansion_ok
}

/// `messages` with every file a span names (its expansions' and its children's too) named as on disk, where the unit
/// remapped it (review of 2e1fcfb1: a remapped file read as one outside the workspace). A span whose file cannot be told
/// is left out, and a message whose primary one cannot be is left out whole: it would be published on some other file
/// (review of 46ce96f2). Returns whether every file could be told.
pub(super) fn unmap(messages: &mut Vec<Value>, remaps: &[(PathBuf, PathBuf)], known: &HashSet<PathBuf>) -> bool {
    if remaps.is_empty() {
        return true;
    }
    let mut sure = true;
    messages.retain_mut(|m| {
        let mut primary_lost = false;
        let retain = |spans: &mut Vec<Value>, sure: &mut bool, primary_lost: &mut bool| {
            spans.retain_mut(|s| {
                let ok = resolve_span(s, remaps, known);
                *sure &= ok;
                *primary_lost |= !ok && s.get("is_primary") == Some(&Value::Bool(true));
                ok
            })
        };
        if let Some(Value::Array(spans)) = m.get_mut("spans") {
            retain(spans, &mut sure, &mut primary_lost);
        }
        // A child's spans (a note's, a help's) are secondary to the message: left out, not the message.
        let mut child_primary = false;
        for child in m.get_mut("children").and_then(Value::as_array_mut).into_iter().flatten() {
            if let Some(Value::Array(spans)) = child.get_mut("spans") {
                retain(spans, &mut sure, &mut child_primary);
            }
        }
        !primary_lost
    });
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
        let known: HashSet<PathBuf> = ["/w/sub/a.rs", "/w/m.rs"].map(PathBuf::from).into();
        let mut m = vec![serde_json::json!({ "spans": [{ "file_name": "/other/a.rs", "is_primary": true, "expansion": { "span": { "file_name": "/src/m.rs" } } }],
                                               "children": [{ "spans": [{ "file_name": "/elsewhere/x.rs" }] }] })];
        assert!(unmap(&mut m, &remaps(&args), &known));
        assert_eq!(m[0]["spans"][0]["file_name"], "/w/sub/a.rs", "the last remap that matches");
        assert_eq!(m[0]["spans"][0]["expansion"]["span"]["file_name"], "/w/m.rs");
        assert_eq!(m[0]["children"][0]["spans"][0]["file_name"], "/elsewhere/x.rs", "a name no remap gives is the file");
    }

    fn primary(name: &str) -> Vec<Value> {
        vec![serde_json::json!({ "spans": [{ "file_name": name, "is_primary": true }, { "file_name": name, "is_primary": false }] })]
    }

    /// Reviews of 68375e6e and 46ce96f2: two sources remapped to one prefix are told by the unit's sources, not by the
    /// files that exist (a buffer exists nowhere); a file the compiler did not remap is a candidate too.
    #[test]
    fn test_a_remapped_name_is_told_by_the_units_sources() {
        let args =
            |pairs: &[(&str, &str)]| -> Vec<String> { std::iter::once("rustc".to_string()).chain(pairs.iter().map(|(f, t)| format!("--remap-path-prefix={f}={t}"))).collect() };
        let known = |files: &[&str]| -> HashSet<PathBuf> { files.iter().map(PathBuf::from).collect() };
        let two = remaps(&args(&[("/A", "/m"), ("/B", "/m")]));
        let mut m = primary("/m/src/new.rs");
        assert!(unmap(&mut m, &two, &known(&["/A/src/new.rs"])));
        assert_eq!(m[0]["spans"][0]["file_name"], "/A/src/new.rs", "the unit's buffer, whatever exists on disk");
        let mut m = primary("/m/src/lib.rs");
        assert!(!unmap(&mut m, &two, &known(&["/A/src/lib.rs", "/B/src/lib.rs"])));
        assert!(m.is_empty(), "a primary span not told leaves the message out, not on an innocent file");
        let mut m = primary("/B/lib.rs");
        assert!(unmap(&mut m, &remaps(&args(&[("/A", "/B")])), &known(&["/B/lib.rs"])));
        assert_eq!(m[0]["spans"][0]["file_name"], "/B/lib.rs", "B is not remapped by A=B: it is B's own file");
        let mut m = vec![serde_json::json!({ "spans": [{ "file_name": "/A/src/lib.rs", "is_primary": true }, { "file_name": "/m/x.rs", "is_primary": false }] })];
        assert!(!unmap(&mut m, &two, &known(&["/A/src/lib.rs"])));
        assert_eq!(m[0]["spans"].as_array().map(Vec::len), Some(1), "a secondary span not told is left out");
    }
}
