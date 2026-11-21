/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use super::*;
use crate::lsp::presentation::compiler_merge::{merge_with_compiler, set_client_moves_rustc};
use crate::lsp::presentation::flycheck::FlycheckState;
use std::time::Duration;

/// A rustc JSON diagnostic with its primary span on `line` (1-based), columns `col..end` (1-based).
fn message(file: &Path, line: u64, col: u64, end: u64, code: &str, text: &str) -> Value {
    json!({
        "$message_type": "diagnostic", "message": text, "code": { "code": code }, "level": "error", "children": [],
        "spans": [{ "file_name": file.display().to_string(), "is_primary": true, "line_start": line, "line_end": line,
                    "column_start": col, "column_end": end, "label": null, "text": [] }]
    })
}

fn workspace() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/a.rs"), "pub fn value() -> u32 { 1 }\n").unwrap();
    std::fs::write(root.join("src/lib.rs"), "pub mod a;\npub fn b() { a::value(); }\n").unwrap();
    std::fs::write(root.join("src/m.rs"), "pub fn m() {}\n").unwrap();
    (dir, root)
}

fn uri(p: &Path) -> String {
    format!("file://{}", p.display())
}

/// A report of a session started now, after the files were written.
fn report(messages: Vec<Value>, checked: &[&Path]) -> CompilerReport {
    CompilerReport { messages, checked: checked.iter().map(|p| p.to_path_buf()).collect(), started: Some(SystemTime::now()) }
}

fn file(diagnostics: Vec<Value>, text: &str, started_us: u64) -> SessionFile {
    SessionFile { diagnostics, text: text.into(), started_us }
}

const NO_CHECK: &dyn Fn(&Path) -> bool = &|_| false;

#[test]
fn test_a_sessions_files_are_the_ones_checked_and_the_ones_reported_on() {
    let (_d, root) = workspace();
    let (a, lib, m) = (root.join("src/a.rs"), root.join("src/lib.rs"), root.join("src/m.rs"));
    let edited = "pub fn value() -> u32 { \"s\" }\n".to_string();
    let e0308 = message(&a, 1, 25, 28, "E0308", "mismatched types");
    let messages = vec![e0308.clone(), e0308, message(&lib, 2, 17, 22, "E0061", "wrong argument count"), message(Path::new("/elsewhere/x.rs"), 1, 1, 2, "E0425", "outside")];
    let files = session_files(&root, &report(messages, &[&a, &lib, &m]), &[(a.clone(), edited.clone())]);
    assert_eq!(files.len(), 3, "{:?}", files.keys().collect::<Vec<_>>());
    assert_eq!(files[&a].text, edited, "a buffer's diagnostics are of its text");
    assert_eq!(files[&a].diagnostics.len(), 1, "the same error of two units is one");
    assert_eq!(files[&a].diagnostics[0]["range"]["start"], json!({ "line": 0, "character": 24 }));
    assert_eq!(files[&lib].text, std::fs::read_to_string(&lib).unwrap(), "another file's are of its text on disk");
    assert!(files[&m].diagnostics.is_empty(), "a file checked with no error has none: the check's older ones go");
}

/// Review of 2e1fcfb1: a compile that stopped before checking certified its buffer clean.
#[test]
fn test_a_buffer_the_compiler_did_not_check_is_not_the_sessions() {
    let (_d, root) = workspace();
    let a = root.join("src/a.rs");
    assert!(session_files(&root, &report(Vec::new(), &[]), &[(a.clone(), "x".into())]).is_empty());
    let parse_error = vec![message(&a, 1, 1, 2, "", "expected item")];
    assert_eq!(session_files(&root, &report(parse_error, &[]), &[(a.clone(), "x".into())])[&a].diagnostics.len(), 1, "what it did report stays");
}

/// Review of 2e1fcfb1: a file written after the session started was paired with spans of the text before.
#[test]
fn test_a_file_written_since_the_session_started_gets_nothing() {
    let (_d, root) = workspace();
    let lib = root.join("src/lib.rs");
    let mut r = report(vec![message(&lib, 2, 17, 22, "E0061", "wrong argument count")], &[&lib]);
    r.started = Some(SystemTime::now() - Duration::from_secs(60));
    assert!(session_files(&root, &r, &[]).is_empty());
}

/// Review of 2e1fcfb1: `a%20b.rs` was taken for `a b.rs`.
#[test]
fn test_a_file_name_is_a_path_not_a_uri() {
    let (_d, root) = workspace();
    let (pct, spaced) = (root.join("src/a%20b.rs"), root.join("src/a b.rs"));
    std::fs::write(&pct, "x\n").unwrap();
    std::fs::write(&spaced, "y\n").unwrap();
    let files = session_files(&root, &report(vec![message(&pct, 1, 1, 2, "E0425", "x")], &[]), &[]);
    assert_eq!(files.keys().collect::<Vec<_>>(), [&pct]);
}

#[test]
fn test_the_innermost_session_answers_and_only_changes_are_touched() {
    let (_d, root) = workspace();
    let (a, m) = (root.join("src/a.rs"), root.join("src/m.rs"));
    let err = message(&a, 1, 1, 2, "E0425", "x");
    let diags = SessionDiagnostics::default();
    let outer = HashMap::from([(a.clone(), file(vec![err], "outer", 1)), (m.clone(), file(Vec::new(), "m", 1))]);
    assert_eq!(diags.replace(&root, outer.clone(), NO_CHECK), std::slice::from_ref(&a), "a clean file the check says nothing of changes nothing");
    assert_eq!(diags.replace(&root, outer, &|p| p == m), Vec::<PathBuf>::new(), "the same again");
    let inner = HashMap::from([(a.clone(), file(Vec::new(), "inner", 2))]);
    assert_eq!(diags.replace(&root.join("src"), inner, NO_CHECK), std::slice::from_ref(&a), "the inner session answers for a, clean");
    assert_eq!(diags.of(&a).unwrap().text, "inner");
    assert_eq!(diags.clear(&root.join("src"), NO_CHECK), std::slice::from_ref(&a), "the outer one's error is back");
    assert_eq!(diags.of(&a).unwrap().text, "outer");
    assert_eq!(diags.clear(&root, &|p| p == m), [a.clone(), m.clone()], "the check's diagnostics of m are back");
    assert!(diags.of(&a).is_none() && diags.roots().is_empty());
}

/// Review of 68375e6e: the same diagnostics on another text sit elsewhere in the buffer, and were not sent again.
#[test]
fn test_the_same_diagnostics_on_another_text_are_sent_again() {
    let (_d, root) = workspace();
    let a = root.join("src/a.rs");
    let err = message(&a, 2, 1, 2, "E0433", "x");
    let diags = SessionDiagnostics::default();
    diags.replace(&root, HashMap::from([(a.clone(), file(vec![err.clone()], "a\nb\n", 1))]), NO_CHECK);
    assert_eq!(diags.replace(&root, HashMap::from([(a.clone(), file(vec![err], "z\na\nb\n", 2))]), NO_CHECK), std::slice::from_ref(&a));
    assert_eq!(diags.replace(&root, HashMap::from([(a.clone(), file(Vec::new(), "q\n", 3))]), NO_CHECK), std::slice::from_ref(&a));
    assert!(diags.replace(&root, HashMap::from([(a.clone(), file(Vec::new(), "r\n", 4))]), NO_CHECK).is_empty(), "nothing on either text");
}

/// Review of 68375e6e: the URI of a publication cleared was used again for the file's next one.
#[test]
fn test_a_publication_is_remembered_only_while_it_is_out() {
    let diags = SessionDiagnostics::default();
    let f = Path::new("/w/src/a.rs");
    diags.record_published(f, "file:///w/link/a.rs", true);
    assert_eq!(diags.published_uri(f).as_deref(), Some("file:///w/link/a.rs"));
    diags.record_published(f, "file:///w/link/a.rs", false);
    assert_eq!(diags.published_uri(f), None);
}

/// prod-code, m=20120: an error typed and never saved was not reported; the check of the saved file was.
#[test]
fn test_a_session_stands_in_for_the_check_of_the_saved_file() {
    let (_d, root) = workspace();
    let a = root.join("src/a.rs");
    let flycheck = FlycheckState::new();
    let saved = std::fs::read_to_string(&a).unwrap();
    let check_error =
        json!({ "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 3 } }, "code": "E0601", "source": "rustc", "message": "of the saved file" });
    flycheck.set_latest_diagnostics_with_baseline(&uri(&a), vec![check_error], Some(saved.clone()));
    let compiled = "pub fn value() -> u32 {\n    \"s\"\n}\n".to_string();
    let r = report(vec![message(&a, 2, 5, 8, "E0308", "mismatched types")], &[&a]);
    flycheck.session.replace(&root, session_files(&root, &r, &[(a.clone(), compiled.clone())]), NO_CHECK);
    let editor = json!({ "range": { "start": { "line": 1, "character": 4 }, "end": { "line": 1, "character": 7 } }, "code": "E0308", "source": "ridx", "message": "heuristic" });

    set_client_moves_rustc(false);
    let got = merge_with_compiler(&uri(&a), std::slice::from_ref(&editor), &compiled, || Some(saved.clone()), &flycheck);
    assert_eq!(got.len(), 1, "the session's error, the editor's same one shadowed, the saved file's gone: {got:?}");
    assert_eq!((got[0]["source"].as_str(), got[0]["range"]["start"]["line"].as_u64()), (Some("rustc"), Some(1)));

    let live = format!("// note\n{compiled}");
    let got = merge_with_compiler(&uri(&a), &[], &live, || Some(saved.clone()), &flycheck);
    assert_eq!(got[0]["range"]["start"]["line"], 2, "moved by a line typed above since: {got:?}");

    set_client_moves_rustc(true);
    let got = merge_with_compiler(&uri(&a), &[], &compiled, || Some(saved.clone()), &flycheck);
    set_client_moves_rustc(false);
    assert_eq!((got[0]["source"].as_str(), got[0]["range"]["start"]["line"].as_u64()), (Some("ridx"), Some(1)), "Zed must not move them: {got:?}");

    flycheck.session.clear(&root, NO_CHECK);
    let got = merge_with_compiler(&uri(&a), &[], &saved, || Some(saved.clone()), &flycheck);
    assert_eq!(got[0]["message"], "of the saved file", "the check answers again once the session ends");
}

/// Reviews of 2e1fcfb1 and 68375e6e: the result that applies is the one of the text the buffer holds, whatever the
/// start times say; of two of that text, the newer.
#[test]
fn test_the_result_of_the_text_the_buffer_holds_applies() {
    let (_d, root) = workspace();
    let a = root.join("src/a.rs");
    let saved = std::fs::read_to_string(&a).unwrap();
    let old = "pub fn value() -> u32 { 2 }\n";
    let flycheck = FlycheckState::new();
    let e0061 = json!({ "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 3 } }, "code": "E0061", "source": "rustc", "message": "check" });
    let session_err = message(&a, 1, 1, 4, "E0308", "session");
    let merge = |live: &str| merge_with_compiler(&uri(&a), &[], live, || Some(saved.clone()), &flycheck);
    let set_check = |text: &str, at: u64| {
        flycheck.set_latest_diagnostics_with_baseline(&uri(&a), vec![e0061.clone()], Some(text.to_string()));
        flycheck.set_checked_at(&uri(&a), at, Some(text));
    };
    let set_session = |diagnostics: Vec<Value>, text: &str, at: u64| {
        let parsed: Vec<Value> = diagnostics.iter().filter_map(|m| parse_compiler_message_at(m, &root)).map(|(_, d)| d).collect();
        flycheck.session.replace(&root, HashMap::from([(a.clone(), file(parsed, text, at))]), NO_CHECK);
    };

    set_session(Vec::new(), old, 200);
    set_check(&saved, 100);
    assert_eq!(merge(&saved)[0]["message"], "check", "a later session of another text does not hide the check of this one");
    set_session(vec![session_err], &saved, 100);
    set_check(old, 200);
    assert_eq!(merge(&saved)[0]["message"].as_str().map(|m| m.starts_with("session")), Some(true), "a later check of another text does not hide the session of this one");
    set_check(&saved, 200);
    assert_eq!(merge(&saved)[0]["message"], "check", "both of this text: the newer");
    set_check(&saved, 50);
    assert_eq!(merge(&saved)[0]["message"].as_str().map(|m| m.starts_with("session")), Some(true));
}
