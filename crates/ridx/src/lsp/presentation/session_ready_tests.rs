/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Readiness of the compiler's diagnostics of a buffer, and the hint of a file no crate includes.

use super::tests::{message, report, uri, workspace, NO_CHECK};
use super::*;
use crate::lsp::presentation::compiler_merge::merge_with_compiler;
use crate::lsp::presentation::flycheck::FlycheckState;

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

/// Review of 43ac9527: a file no crate includes, changed on disk after it was opened, was admitted as saved by the copy
/// taken at open while the answer read the disk and gave no hint: an empty list.
#[test]
fn test_a_file_changed_on_disk_since_it_was_opened_is_not_as_saved() {
    let (_d, root) = workspace();
    let stray = root.join("src/stray.rs");
    let text = "pub fn s() {}\n".to_string();
    std::fs::write(&stray, &text).unwrap();
    let flycheck = FlycheckState::new();
    let mut r = report(Vec::new(), &[]);
    r.unheld.insert(stray.clone());
    flycheck.session.set_outcomes(&root, session_outcomes(&r, &[(stray.clone(), text.clone())], &HashMap::new()));
    let path = stray.to_string_lossy().to_string();
    let mut ov = crate::overlay::OverlayManager::new();
    ov.open_with_provenance(&uri(&stray), &path, 1, text, true);
    std::fs::write(&stray, "pub fn s() { changed() }\n").unwrap();
    assert_eq!(readiness(&flycheck, &ov, &path, &root), Readiness::Unavailable(1, UNLINKED), "not as saved any more: refused, not an empty list");
}

/// Review of 43ac9527: an inner root's `notInAnyCrate` admitted the pull, and an enclosing root's clean result was
/// answered.
#[test]
fn test_the_answer_is_the_innermost_sessions() {
    let (_d, root) = workspace();
    let inner = root.join("src");
    let a = root.join("src/a.rs");
    let text = std::fs::read_to_string(&a).unwrap();
    let texts = [(a.clone(), text.clone())];
    let flycheck = FlycheckState::new();
    let clean = report(Vec::new(), &[&a]);
    let outer_files = session_files(&root, &clean, &texts);
    flycheck.session.set_outcomes(&root, session_outcomes(&clean, &texts, &outer_files));
    flycheck.session.replace(&root, outer_files, NO_CHECK);
    let mut unlinked = report(Vec::new(), &[]);
    unlinked.unheld.insert(a.clone());
    flycheck.session.set_outcomes(&inner, session_outcomes(&unlinked, &texts, &HashMap::new()));
    let got = merge_with_compiler(&uri(&a), &[], &text, || Some(text.clone()), &flycheck);
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0]["code"], "unlinked-file");
}
