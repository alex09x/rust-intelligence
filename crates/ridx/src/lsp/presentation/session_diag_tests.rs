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

#[test]
fn test_a_sessions_files_are_its_buffers_and_what_the_compiler_reported_on() {
    let (_d, root) = workspace();
    let (a, lib) = (root.join("src/a.rs"), root.join("src/lib.rs"));
    let edited = "pub fn value() -> u32 { \"s\" }\n".to_string();
    let e0308 = message(&a, 1, 25, 28, "E0308", "mismatched types");
    let messages = [e0308.clone(), e0308, message(&lib, 2, 17, 22, "E0061", "wrong argument count"), message(Path::new("/elsewhere/x.rs"), 1, 1, 2, "E0425", "outside")];
    let files = session_files(&root, &messages, &[(a.clone(), edited.clone())]);
    assert_eq!(files.len(), 2, "{:?}", files.keys().collect::<Vec<_>>());
    assert_eq!(files[&a].text, edited, "a buffer's diagnostics are of its text");
    assert_eq!(files[&a].diagnostics.len(), 1, "the same error of two units is one");
    assert_eq!(files[&a].diagnostics[0]["range"]["start"], json!({ "line": 0, "character": 24 }));
    assert_eq!(files[&lib].text, std::fs::read_to_string(&lib).unwrap(), "another file's are of its text on disk");

    let clean = session_files(&root, &[], &[(a.clone(), edited)]);
    assert!(clean[&a].diagnostics.is_empty(), "a buffer with no error is still the session's");
}

#[test]
fn test_the_innermost_session_answers_and_its_end_gives_the_files_back() {
    let (_d, root) = workspace();
    let a = root.join("src/a.rs");
    let one = |text: &str| HashMap::from([(a.clone(), SessionFile { diagnostics: Vec::new(), text: text.into() })]);
    let diags = SessionDiagnostics::default();
    assert_eq!(diags.replace(&root, one("outer")), std::slice::from_ref(&a));
    assert_eq!(diags.replace(&root.join("src"), one("inner")), std::slice::from_ref(&a));
    assert_eq!(diags.of(&a).unwrap().text, "inner");
    assert_eq!(diags.replace(&root.join("src"), HashMap::new()), std::slice::from_ref(&a), "a file the session no longer has is touched too");
    assert_eq!(diags.of(&a).unwrap().text, "outer");
    assert_eq!(diags.clear(&root), std::slice::from_ref(&a));
    assert!(diags.of(&a).is_none() && diags.clear(&root).is_empty());
}

/// prod-code, m=20120: an error typed and never saved was not reported; the check of the saved file was.
#[test]
fn test_a_session_stands_in_for_the_check_of_the_saved_file() {
    let (_d, root) = workspace();
    let a = root.join("src/a.rs");
    let flycheck = FlycheckState::new();
    let saved = std::fs::read_to_string(&a).unwrap();
    flycheck.set_latest_diagnostics_with_baseline(&uri(&a), vec![json!({ "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 3 } }, "code": "E0601", "source": "rustc", "message": "of the saved file" })], Some(saved.clone()));
    let compiled = "pub fn value() -> u32 {\n    \"s\"\n}\n".to_string();
    let messages = [message(&a, 2, 5, 8, "E0308", "mismatched types")];
    flycheck.session.replace(&root, session_files(&root, &messages, &[(a.clone(), compiled.clone())]));
    let editor = json!({ "range": { "start": { "line": 1, "character": 4 }, "end": { "line": 1, "character": 7 } }, "code": "E0308", "source": "ridx", "message": "heuristic" });

    set_client_moves_rustc(false);
    let got = merge_with_compiler(&uri(&a), std::slice::from_ref(&editor), &compiled, || Some(saved.clone()), &flycheck);
    assert_eq!(got.len(), 1, "the session's error, the editor's same one shadowed, the saved file's gone: {got:?}");
    assert_eq!((got[0]["source"].as_str(), got[0]["range"]["start"]["line"].as_u64()), (Some("rustc"), Some(1)));

    // Typed further since the build: a line above moves it down, in the buffer.
    let live = format!("// note\n{compiled}");
    let got = merge_with_compiler(&uri(&a), &[], &live, || Some(saved.clone()), &flycheck);
    assert_eq!(got[0]["range"]["start"]["line"], 2, "{got:?}");

    // Zed moves `rustc` diagnostics through unsaved edits itself; these were reported on the buffer.
    set_client_moves_rustc(true);
    let got = merge_with_compiler(&uri(&a), &[], &compiled, || Some(saved.clone()), &flycheck);
    set_client_moves_rustc(false);
    assert_eq!((got[0]["source"].as_str(), got[0]["range"]["start"]["line"].as_u64()), (Some("ridx"), Some(1)), "{got:?}");

    flycheck.session.clear(&root);
    let got = merge_with_compiler(&uri(&a), &[], &saved, || Some(saved.clone()), &flycheck);
    assert_eq!(got[0]["message"], "of the saved file", "the check answers again once the session ends");
}
