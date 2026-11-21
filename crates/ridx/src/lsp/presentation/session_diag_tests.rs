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
    diags.record_published(f, "file:///w/one/a.rs", true);
    diags.record_published(f, "file:///w/two/a.rs", true);
    assert_eq!(diags.published_uris(f, &|_| true), ["file:///w/one/a.rs", "file:///w/two/a.rs"], "every URI still out, not only the last");
    diags.record_published(f, "file:///w/one/a.rs", false);
    assert_eq!(diags.published_uris(f, &|_| true), ["file:///w/two/a.rs"]);
    assert!(diags.published_uris(f, &|_| false).is_empty() && diags.published_uris(f, &|_| true).is_empty(), "one no longer showing anything is forgotten");
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
    // Review of 88fe53d0: another file named `a%20b.rs` is not the one asked for by the URI `a%20b.rs` of `a b.rs`.
    let literal = root.join("src/a%20b.rs");
    std::fs::write(&literal, "x\n").unwrap();
    let other = json!({ "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 1 } }, "code": "E0599", "source": "rustc", "message": "of a%20b.rs" });
    let flycheck = FlycheckState::new();
    flycheck.set_latest_diagnostics_with_baseline(&uri(&literal), vec![other], Some("x\n".into()));
    assert!(merge_with_compiler(&encoded, &[], "x\n", || Some("x\n".into()), &flycheck).is_empty());
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

/// Review of 88fe53d0: the check's clear of `a b.rs` went to its own spelling, while the client was shown the file's
/// error under `a%20b.rs`.
#[test]
fn test_the_checks_result_goes_where_the_client_was_shown_the_file() {
    let (_d, root) = workspace();
    let spaced = root.join("src/a b.rs");
    std::fs::write(&spaced, "x\n").unwrap();
    let lifecycle = crate::lsp::LifecycleManager::new();
    let ov = crate::overlay::OverlayManager::new();
    let encoded = crate::lsp::protocol::path_to_uri(&spaced.to_string_lossy());
    lifecycle.flycheck.mark_published(&encoded);
    lifecycle.flycheck.session.record_published(&spaced, &encoded, true);
    let mut out = Vec::new();
    publish_check(&uri(&spaced), &lifecycle, &ov, &mut out).unwrap();
    let sent = String::from_utf8(out).unwrap();
    assert!(sent.contains(&encoded) && sent.contains("\"diagnostics\":[]"), "{sent}");
    assert!(!lifecycle.flycheck.has_active_diagnostics(&encoded));
}

/// Review of 2810515d: the check's key of a file named `a%20b.rs` was decoded to `a b.rs`, and its open buffer
/// (`a%2520b.rs`) got nothing.
#[test]
fn test_the_checks_key_is_a_path_not_a_uri() {
    let (_d, root) = workspace();
    let literal = root.join("src/a%20b.rs");
    std::fs::write(&literal, "x\n").unwrap();
    let lifecycle = crate::lsp::LifecycleManager::new();
    let mut ov = crate::overlay::OverlayManager::new();
    let client = crate::lsp::protocol::path_to_uri(&literal.to_string_lossy());
    ov.open_with_provenance(&client, &literal.to_string_lossy(), 1, "x\n".into(), true);
    let e0425 = json!({ "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 1 } }, "code": "E0425", "source": "rustc", "message": "saved" });
    lifecycle.flycheck.set_latest_diagnostics_with_baseline(&uri(&literal), vec![e0425], Some("x\n".into()));
    let mut out = Vec::new();
    publish_check(&uri(&literal), &lifecycle, &ov, &mut out).unwrap();
    let sent = String::from_utf8(out).unwrap();
    assert!(sent.contains(&client) && sent.contains("E0425"), "{sent}");
}

/// Review of 2810515d: a file shown under an alias when closed, opened again under its own URI, kept the alias's
/// error when the check cleared it.
#[test]
fn test_a_uri_the_file_was_shown_under_before_is_cleared() {
    let (_d, root) = workspace();
    let a = root.join("src/a.rs");
    let lifecycle = crate::lsp::LifecycleManager::new();
    let mut ov = crate::overlay::OverlayManager::new();
    let alias = "file:///elsewhere/link/a.rs";
    lifecycle.flycheck.mark_published(alias);
    lifecycle.flycheck.session.record_published(&a, alias, true);
    ov.open_with_provenance(&uri(&a), &a.to_string_lossy(), 1, std::fs::read_to_string(&a).unwrap(), true);
    let mut out = Vec::new();
    publish_check(&uri(&a), &lifecycle, &ov, &mut out).unwrap();
    let sent = String::from_utf8(out).unwrap();
    assert!(sent.contains(alias) && !lifecycle.flycheck.has_active_diagnostics(alias), "{sent}");
}

/// Review of c7b4013f: closed under two aliases in turn, the file kept the first's error when the check cleared it.
#[test]
fn test_every_uri_the_file_was_shown_under_is_cleared() {
    let (_d, root) = workspace();
    let a = root.join("src/a.rs");
    let lifecycle = crate::lsp::LifecycleManager::new();
    let ov = crate::overlay::OverlayManager::new();
    for alias in ["file:///one/a.rs", "file:///two/a.rs"] {
        lifecycle.flycheck.mark_published(alias);
        lifecycle.flycheck.session.record_published(&a, alias, true);
    }
    let mut out = Vec::new();
    publish_check(&uri(&a), &lifecycle, &ov, &mut out).unwrap();
    assert!(!lifecycle.flycheck.has_active_diagnostics("file:///one/a.rs") && !lifecycle.flycheck.has_active_diagnostics("file:///two/a.rs"));
}

/// Review of c7b4013f: publishing `a%20b.rs` cleared `a b.rs`, whose URI is that file's check key.
#[test]
fn test_a_check_key_is_never_cleared_as_a_uri() {
    let (_d, root) = workspace();
    let (literal, spaced) = (root.join("src/a%20b.rs"), root.join("src/a b.rs"));
    std::fs::write(&literal, "x\n").unwrap();
    std::fs::write(&spaced, "x\n").unwrap();
    let lifecycle = crate::lsp::LifecycleManager::new();
    let ov = crate::overlay::OverlayManager::new();
    let spaced_uri = crate::lsp::protocol::path_to_uri(&spaced.to_string_lossy());
    assert_eq!(spaced_uri, uri(&literal), "the space file's URI is the literal file's key");
    lifecycle.flycheck.mark_published(&spaced_uri);
    lifecycle.flycheck.session.record_published(&spaced, &spaced_uri, true);
    let mut out = Vec::new();
    publish_check(&uri(&literal), &lifecycle, &ov, &mut out).unwrap();
    assert!(lifecycle.flycheck.has_active_diagnostics(&spaced_uri), "{}", String::from_utf8_lossy(&out));
}

/// Review of c7b4013f: the check's own pass read `a%20b.rs` as `a b.rs`; a key is read back as the path it holds.
#[test]
fn test_a_check_key_is_read_back_as_its_path() {
    let p = Path::new("/w/src/a%20b.rs");
    assert_eq!(crate::lsp::presentation::compiler_merge::check_path(&crate::lsp::presentation::compiler_merge::check_key(p)), p);
}

/// prod-code, m=20178: a pull before the session compiled the buffer answered `[]`, read as clean; a compile that gave
/// no diagnostics must say so.
#[test]
fn test_a_buffers_diagnostics_are_known_once_its_text_is_compiled() {
    let (_d, root) = workspace();
    let (a, lib) = (root.join("src/a.rs"), root.join("src/lib.rs"));
    let flycheck = FlycheckState::new();
    let path = a.to_string_lossy().to_string();
    let ov = edited(&a, "pub fn value() -> u32 { \"s\" }\n");
    assert_eq!(readiness(&flycheck, &ov, &path, &root), Readiness::Pending, "not compiled yet");
    let texts = [(a.clone(), "pub fn value() -> u32 { \"s\" }\n".to_string())];
    let r = report(vec![message(&a, 1, 25, 28, "E0308", "mismatched types")], &[&a]);
    let files = session_files(&root, &r, &texts);
    flycheck.session.set_outcomes(&root, session_outcomes(&r, &texts, &files));
    assert_eq!(readiness(&flycheck, &ov, &path, &root), Readiness::Ready(2));
    let stale = edited(&a, "pub fn value() -> u32 { 2 }\n");
    assert_eq!(readiness(&flycheck, &stale, &path, &root), Readiness::Pending, "another text was compiled");

    let mut untold = report(Vec::new(), &[]);
    untold.untold.insert(a.clone());
    flycheck.session.set_outcomes(&root, session_outcomes(&untold, &texts, &HashMap::new()));
    assert_eq!(readiness(&flycheck, &ov, &path, &root), Readiness::Unavailable(2, "pathsUntold"));
    flycheck.session.set_outcomes(&root, session_outcomes(&report(Vec::new(), &[]), &texts, &HashMap::new()));
    assert_eq!(readiness(&flycheck, &ov, &path, &root), Readiness::Unavailable(2, "notChecked"), "stopped before checking it");

    // Review of 13eacef0: a buffer as saved is ready once a session compiled its text too, whatever a check of the
    // saved files did (it may have failed, read another text, or been another workspace's).
    let mut saved = crate::overlay::OverlayManager::new();
    let lib_path = lib.to_string_lossy().to_string();
    let lib_text = std::fs::read_to_string(&lib).unwrap();
    saved.open_with_provenance(&uri(&lib), &lib_path, 1, lib_text.clone(), true);
    flycheck.set_latest_diagnostics_with_baseline(&uri(&lib), Vec::new(), Some(lib_text.clone()));
    assert_eq!(readiness(&flycheck, &saved, &lib_path, &root), Readiness::Pending, "the check of the saved files is not it");
    let lib_texts = [(lib.clone(), lib_text)];
    let r = report(Vec::new(), &[&lib]);
    flycheck.session.set_outcomes(&root, session_outcomes(&r, &lib_texts, &session_files(&root, &r, &lib_texts)));
    assert_eq!(readiness(&flycheck, &saved, &lib_path, &root), Readiness::Ready(1));
}

/// An overlay manager with `file` open and edited to `text`, not saved (version 2).
fn edited(file: &Path, text: &str) -> crate::overlay::OverlayManager {
    let mut ov = crate::overlay::OverlayManager::new();
    let path = file.to_string_lossy();
    ov.open_with_provenance(&uri(file), &path, 1, std::fs::read_to_string(file).unwrap(), true);
    ov.apply_content_changes(&path, 2, &[json!({ "text": text })]);
    ov
}

/// prod-code, m=20185: a file on disk no crate includes answered `[]`, read as a file without errors; it gets
/// rust-analyzer's `unlinked-file` hint, while an edited one stays refused.
#[test]
fn test_a_file_no_crate_includes_says_so() {
    let (_d, root) = workspace();
    let stray = root.join("tests/stray/helper.rs");
    std::fs::create_dir_all(stray.parent().unwrap()).unwrap();
    let text = "pub fn helper() -> u32 { 1 }\n".to_string();
    std::fs::write(&stray, &text).unwrap();
    let flycheck = FlycheckState::new();
    let texts = [(stray.clone(), text.clone())];
    let mut r = report(Vec::new(), &[]);
    r.unheld.insert(stray.clone());
    flycheck.session.set_outcomes(&root, session_outcomes(&r, &texts, &HashMap::new()));
    let path = stray.to_string_lossy().to_string();
    let mut ov = crate::overlay::OverlayManager::new();
    ov.open_with_provenance(&uri(&stray), &path, 1, text.clone(), true);
    assert_eq!(readiness(&flycheck, &ov, &path, &root), Readiness::Ready(1), "as saved: answered");
    let got = merge_with_compiler(&uri(&stray), &[], &text, || Some(text.clone()), &flycheck);
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!((got[0]["code"].as_str(), got[0]["severity"].as_u64(), got[0]["source"].as_str()), (Some("unlinked-file"), Some(4), Some("ridx")));
    assert!(got[0]["message"].as_str().unwrap().contains("not included in any crates"));
    let edited = edited(&stray, "pub fn helper() -> u32 { 2 }\n");
    assert_eq!(readiness(&flycheck, &edited, &path, &root), Readiness::Pending, "another text: not this one's outcome");
}

/// Review of 7501440b: an enclosing workspace's session, which compiled the file with its own cfgs and dependencies,
/// released the pull of a file of a nested workspace.
#[test]
fn test_only_the_files_own_workspace_says() {
    let (_d, root) = workspace();
    let nested = root.join("src");
    let a = root.join("src/a.rs");
    let flycheck = FlycheckState::new();
    let text = std::fs::read_to_string(&a).unwrap();
    let texts = [(a.clone(), text.clone())];
    let r = report(Vec::new(), &[&a]);
    flycheck.session.set_outcomes(&root, session_outcomes(&r, &texts, &session_files(&root, &r, &texts)));
    let mut ov = crate::overlay::OverlayManager::new();
    let path = a.to_string_lossy().to_string();
    ov.open_with_provenance(&uri(&a), &path, 1, text, true);
    assert_eq!(readiness(&flycheck, &ov, &path, &root), Readiness::Ready(1));
    assert_eq!(readiness(&flycheck, &ov, &path, &nested), Readiness::Pending, "the enclosing workspace's outcome is not the nested one's");
}
