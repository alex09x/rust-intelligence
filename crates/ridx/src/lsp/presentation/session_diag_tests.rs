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
    (dir, root)
}

fn uri(p: &Path) -> String {
    format!("file://{}", p.display())
}

fn report(messages: Vec<Value>, checked: &[&Path]) -> CompilerReport {
    CompilerReport { messages, checked: checked.iter().map(|p| p.to_path_buf()).collect(), ..Default::default() }
}

fn file(diagnostics: Vec<Value>, text: &str) -> SessionFile {
    SessionFile { diagnostics, text: text.into() }
}

const NO_CHECK: &dyn Fn(&Path) -> bool = &|_| false;

/// Review of 710381e0: only the buffers compiled are the session's; what a closed file's text and inputs were is left
/// to the check.
#[test]
fn test_a_sessions_files_are_its_buffers() {
    let (_d, root) = workspace();
    let (a, lib) = (root.join("src/a.rs"), root.join("src/lib.rs"));
    let edited = "pub fn value() -> u32 { \"s\" }\n".to_string();
    let e0308 = message(&a, 1, 25, 28, "E0308", "mismatched types");
    let messages = vec![e0308.clone(), e0308, message(&lib, 2, 17, 22, "E0061", "wrong argument count")];
    let files = session_files(&root, &report(messages, &[&a, &lib]), &[(a.clone(), edited.clone())]);
    assert_eq!(files.keys().collect::<Vec<_>>(), [&a], "lib.rs, closed, keeps the check's");
    assert_eq!(files[&a].text, edited);
    assert_eq!(files[&a].diagnostics.len(), 1, "the same error of two units is one");
    assert_eq!(files[&a].diagnostics[0]["range"]["start"], json!({ "line": 0, "character": 24 }));
    let clean = session_files(&root, &report(Vec::new(), &[&a]), &[(a.clone(), edited)]);
    assert!(clean[&a].diagnostics.is_empty(), "a buffer checked with no error has none");
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

/// Review of 2e1fcfb1: `a%20b.rs` was taken for `a b.rs`.
#[test]
fn test_a_file_name_is_a_path_not_a_uri() {
    let (_d, root) = workspace();
    let (pct, spaced) = (root.join("src/a%20b.rs"), root.join("src/a b.rs"));
    std::fs::write(&pct, "x\n").unwrap();
    std::fs::write(&spaced, "y\n").unwrap();
    let buffers = [(pct.clone(), "x\n".to_string()), (spaced.clone(), "y\n".to_string())];
    let files = session_files(&root, &report(vec![message(&pct, 1, 1, 2, "E0425", "x")], &[]), &buffers);
    assert_eq!(files.keys().collect::<Vec<_>>(), [&pct]);
}

#[test]
fn test_the_innermost_session_answers_and_only_changes_are_touched() {
    let (_d, root) = workspace();
    let a = root.join("src/a.rs");
    let err = message(&a, 1, 1, 2, "E0425", "x");
    let diags = SessionDiagnostics::default();
    let outer = HashMap::from([(a.clone(), file(vec![err], "outer"))]);
    assert_eq!(diags.replace(&root, outer.clone(), NO_CHECK), std::slice::from_ref(&a));
    assert!(diags.replace(&root, outer, &|_| true).is_empty(), "the same again");
    let inner = HashMap::from([(a.clone(), file(Vec::new(), "inner"))]);
    assert_eq!(diags.replace(&root.join("src"), inner, NO_CHECK), std::slice::from_ref(&a), "the inner session answers for a, clean");
    assert_eq!(diags.of(&a).unwrap().text, "inner");
    assert_eq!(diags.clear(&root.join("src"), NO_CHECK), std::slice::from_ref(&a), "the outer one's error is back");
    assert_eq!(diags.clear(&root, NO_CHECK), std::slice::from_ref(&a));
    assert!(diags.of(&a).is_none() && diags.roots().is_empty());
}

/// Reviews of 68375e6e, 46ce96f2 and 710381e0: what a client is sent changes with the text the diagnostics are placed
/// by, and, while the check has diagnostics of the file, with a clean result's text or the root giving it.
#[test]
fn test_a_change_of_text_or_root_is_sent_again() {
    let (_d, root) = workspace();
    let a = root.join("src/a.rs");
    let err = message(&a, 2, 1, 2, "E0433", "x");
    let diags = SessionDiagnostics::default();
    let one = |f: SessionFile| HashMap::from([(a.clone(), f)]);
    diags.replace(&root, one(file(vec![err.clone()], "a\nb\n")), NO_CHECK);
    assert_eq!(diags.replace(&root, one(file(vec![err], "z\na\nb\n")), NO_CHECK), std::slice::from_ref(&a), "same diagnostics, other text");
    assert_eq!(diags.replace(&root, one(file(Vec::new(), "q\n")), NO_CHECK), std::slice::from_ref(&a));
    assert!(diags.replace(&root, one(file(Vec::new(), "r\n")), NO_CHECK).is_empty(), "nothing on either text, nothing from the check");
    assert_eq!(diags.replace(&root, one(file(Vec::new(), "s\n")), &|_| true), std::slice::from_ref(&a), "the check's error was hidden by another text");
    assert_eq!(diags.replace(&root.join("src"), one(file(Vec::new(), "s\n")), &|_| true), std::slice::from_ref(&a), "the same result from another root");
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

/// prod-code, m=20120: an error typed and never saved was not reported; the check of the saved file was. Review of
/// 710381e0: saving the buffer does not bring the check of the earlier saved text back.
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
    let got = merge_with_compiler(&uri(&a), &[], &compiled, || Some(compiled.clone()), &flycheck);
    assert_eq!(got[0]["message"].as_str().map(|m| m.starts_with("mismatched")), Some(true), "saved since: still the session's");

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

/// Review of fb2bed09: a buffer another unit owning it did not check to the end, or whose paths cannot be told, is not
/// certified clean by the unit that did.
#[test]
fn test_a_buffer_is_certified_only_by_all_its_owners() {
    let (_d, root) = workspace();
    let a = root.join("src/a.rs");
    let buffer = [(a.clone(), "x".to_string())];
    let mut r = report(Vec::new(), &[&a]);
    r.unchecked.insert(a.clone());
    assert!(session_files(&root, &r, &buffer).is_empty(), "an owner stopped first");
    let mut r = report(vec![message(&a, 1, 1, 2, "E0425", "x")], &[&a]);
    r.untold.insert(a.clone());
    assert!(session_files(&root, &r, &buffer).is_empty(), "an owner remaps paths: nothing of the session, even reported");
}

/// Review of fb2bed09: a buffer closed kept the session's result of its discarded text.
#[test]
fn test_a_closed_buffer_goes_back_to_the_check() {
    let (_d, root) = workspace();
    let (a, lib) = (root.join("src/a.rs"), root.join("src/lib.rs"));
    let diags = SessionDiagnostics::default();
    diags.replace(&root, HashMap::from([(a.clone(), file(Vec::new(), "x")), (lib.clone(), file(Vec::new(), "y"))]), NO_CHECK);
    assert_eq!(diags.keep_only(&|p| p != a, &|_| true), std::slice::from_ref(&a), "its saved file's errors are back");
    assert!(diags.of(&a).is_none() && diags.of(&lib).is_some());
    assert!(diags.keep_only(&|p| p != a, &|_| true).is_empty());
}

/// Review of d4541e29: the check's result of `a b.rs` is kept under its path, the client's URI says `a%20b.rs`.
#[test]
fn test_the_check_is_found_whatever_the_uri_spelling() {
    let (_d, root) = workspace();
    let spaced = root.join("src/a b.rs");
    std::fs::write(&spaced, "x\n").unwrap();
    let flycheck = FlycheckState::new();
    let e0425 = json!({ "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 1 } }, "code": "E0425", "source": "rustc", "message": "saved" });
    flycheck.set_latest_diagnostics_with_baseline(&uri(&spaced), vec![e0425], Some("x\n".into()));
    let encoded = crate::lsp::protocol::path_to_uri(&spaced.to_string_lossy());
    assert_ne!(encoded, uri(&spaced));
    assert_eq!(merge_with_compiler(&encoded, &[], "x\n", || Some("x\n".into()), &flycheck).len(), 1);
}

/// Review of d4541e29: a document closed and opened again is another buffer than the one a build in flight compiled.
#[test]
fn test_a_document_opened_again_is_another_buffer() {
    let (_d, root) = workspace();
    let a = root.join("src/a.rs");
    let path = a.to_string_lossy().to_string();
    let mut ov = crate::overlay::OverlayManager::new();
    ov.open_with_provenance(&uri(&a), &path, 1, "x".into(), true);
    let first = ov.get(&path).unwrap().instance;
    ov.close_document(&path);
    ov.open_with_provenance(&uri(&a), &path, 1, "x".into(), true);
    assert_ne!(ov.get(&path).unwrap().instance, first);
}
