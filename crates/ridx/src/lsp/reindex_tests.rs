/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Tests of `reindex`.

use super::*;

#[test]
fn test_only_an_indexed_workspace_is_reindexed_and_never_from_a_test_binary() {
    let dir = tempfile::tempdir().unwrap();
    let r = Reindexer::new();
    let src = dir.path().join("src/lib.rs");
    r.trigger_for_change(dir.path(), &src);
    assert!(r.roots.lock().unwrap().is_empty(), "no index, no run");
    std::fs::create_dir_all(dir.path().join("target/ridx")).unwrap();
    std::fs::write(dir.path().join("target/ridx/ridx_manifest.json"), "{}").unwrap();
    assert!(ridx_exe().is_none(), "the test binary is not ridx");
    r.trigger_for_change(dir.path(), &src);
    assert!(r.roots.lock().unwrap().is_empty());
}

#[test]
fn test_watch_registration_names_the_sources_of_the_index() {
    let reg = watch_registration(serde_json::json!("ridx-refresh-1"));
    assert_eq!(reg["method"], "client/registerCapability");
    let r = &reg["params"]["registrations"][0];
    assert_eq!(r["method"], "workspace/didChangeWatchedFiles");
    let globs: Vec<&str> = r["registerOptions"]["watchers"].as_array().unwrap().iter().map(|w| w["globPattern"].as_str().unwrap()).collect();
    assert_eq!(globs, ["**/*.rs", "**/Cargo.toml", "**/Cargo.lock"]);
}

/// In Zed: a saved buffer reverted by `git checkout` kept "unused import" from the check of the saved text. A text
/// not checked yet is checked once, whichever of a save and its watched-file event comes first, and whether or not
/// the editor already reloaded or edited the buffer (review of 3902e41c: comparing with the buffer restarted the
/// check after a save and further typing, and missed a checkout the editor reloaded before reporting it).
#[test]
fn test_each_text_of_a_source_is_checked_once() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let src = root.join("src/lib.rs");
    std::fs::create_dir_all(src.parent().unwrap()).unwrap();
    let fc = crate::lsp::presentation::flycheck::FlycheckState::new();
    std::fs::write(&src, "pub fn a() {}\n").unwrap();
    assert!(fc.mark_saved_for_check(&src), "a save: checked");
    assert!(!fc.mark_changed_for_check(&src), "its watched-file event after didSave: not again");
    assert!(fc.mark_saved_for_check(&src), "saved again: checked again, as rust-analyzer does");
    std::fs::write(&src, "pub fn a() { let y = 2; }\n").unwrap();
    assert!(fc.mark_changed_for_check(&src), "Zed's watched-file event before didSave");
    assert!(!fc.mark_saved_for_check(&src), "then its didSave: the check of this text is running");
    std::fs::write(&src, "pub fn a() { let x = 1; }\n").unwrap();
    assert!(fc.mark_changed_for_check(&src), "a checkout, reloaded into the buffer or not");
    std::fs::remove_file(&src).unwrap();
    assert!(fc.mark_changed_for_check(&src), "a removed file");
    assert!(!fc.mark_changed_for_check(&src));
    assert!(is_checked_source(root, &src));
    assert!(!is_checked_source(root, &root.join("target/ridx/sources/1.rs")), "build output");
    assert!(!is_checked_source(root, &root.join("Cargo.toml")), "not a source");
}

/// A picked-up reindex asks the client to fetch again what came from the old index, for each kind it can refresh.
#[test]
fn test_refresh_requests_follow_the_client_capabilities() {
    let methods = |workspace_caps: serde_json::Value| {
        let mut lc = crate::lsp::lifecycle::LifecycleManager::new();
        let init = serde_json::json!({ "params": { "capabilities": { "workspace": workspace_caps } } });
        lc.handle_initialize(&init).unwrap();
        lc.index_refresh_requests().iter().map(|r| r["method"].as_str().unwrap().to_string()).collect::<Vec<_>>()
    };
    let zed = serde_json::json!({ "inlayHint": { "refreshSupport": true }, "semanticTokens": { "refreshSupport": true }, "codeLens": { "refreshSupport": true } });
    assert_eq!(methods(zed), ["workspace/inlayHint/refresh", "workspace/semanticTokens/refresh", "workspace/codeLens/refresh"]);
    assert_eq!(methods(serde_json::json!({ "inlayHint": { "refreshSupport": true }, "codeLens": { "refreshSupport": false } })), ["workspace/inlayHint/refresh"]);
    assert!(methods(serde_json::json!({})).is_empty());
    let r = Reindexer::new();
    assert!(r.take_finished().is_empty());
    r.finished.lock().unwrap().extend([PathBuf::from("/w"), PathBuf::from("/v")]);
    assert_eq!(r.take_finished(), [PathBuf::from("/w"), PathBuf::from("/v")]);
    assert!(r.take_finished().is_empty(), "taken once");
}

/// In Zed on hl-order-book-server, never indexed, a click on `NativeExecutor` went nowhere: a Cargo workspace
/// without an index gets its first one, not a folder without `Cargo.toml`, nor a library's sources in a cache.
#[test]
fn test_a_cargo_workspace_without_an_index_is_indexed() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    assert!(!needs_first_index(root), "no Cargo.toml");
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"w\"\n").unwrap();
    assert!(needs_first_index(root));
    let registry = root.join(".cargo/registry/src/index.crates.io-1/serde-1.0.0");
    std::fs::create_dir_all(&registry).unwrap();
    std::fs::write(registry.join("Cargo.toml"), "[package]\nname = \"serde\"\n").unwrap();
    assert!(!needs_first_index(&registry), "a dependency's sources");
    std::fs::create_dir_all(root.join("target/ridx")).unwrap();
    std::fs::write(root.join(MANIFEST), "{}").unwrap();
    assert!(!needs_first_index(root), "indexed");

    let r = Reindexer::new();
    r.index_if_missing(dir.path());
    assert!(r.running().is_empty(), "the test binary runs no indexing");
    r.roots.lock().unwrap().entry(PathBuf::from("/b")).or_default().running_since = Some(SystemTime::now());
    r.roots.lock().unwrap().entry(PathBuf::from("/a")).or_default().running_since = Some(SystemTime::now());
    r.roots.lock().unwrap().entry(PathBuf::from("/idle")).or_default();
    assert_eq!(r.running(), [PathBuf::from("/a"), PathBuf::from("/b")]);

    r.queue_first_index(dir.path());
    r.start_queued();
    assert!(r.queued.lock().unwrap().is_empty(), "the folders of `initialize` are taken at `initialized`");
}

/// Review of 30e62913: during a first run a save in a member crate went to that crate, a save in an unindexed second
/// root to the indexed first one, and a root reached through a symlink was a second root.
#[test]
fn test_a_saved_file_goes_to_the_root_being_indexed() {
    use crate::lsp::dispatch::workspace_root_of;
    let dir = tempfile::tempdir().unwrap();
    let base = ridx_format::canonicalize_best_effort(dir.path());
    let (a, b) = (base.join("a"), base.join("b"));
    for (root, member) in [(&a, "core"), (&b, "server")] {
        std::fs::create_dir_all(root.join(member).join("src")).unwrap();
        std::fs::write(root.join("Cargo.toml"), "[workspace]\n").unwrap();
        std::fs::write(root.join(member).join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(root.join(member).join("src/lib.rs"), "pub fn f() {}\n").unwrap();
    }
    let a_src = a.join("core/src/lib.rs");
    let mut w = ridx_format::writer::IndexWriter::new("core", "0.1.0");
    w.get_or_insert_file(a_src.to_str().unwrap());
    std::fs::create_dir_all(a.join("target/ridx")).unwrap();
    w.write_to(&mut std::fs::File::create(a.join("target/ridx/core.ridx")).unwrap()).unwrap();
    let mut mr = crate::lsp::workspace::MultiRootWorkspace::new();
    mr.open_from_folders(&[a.clone(), b.clone()]);
    assert_eq!(mr.len(), 1, "only the indexed root opens");
    let loaded = mr.all_workspaces().next().and_then(|ws| ws.workspace_root()).map(ridx_format::canonicalize_best_effort);

    let r = Reindexer::new();
    let b_src = b.join("server/src/lib.rs");
    assert_eq!(workspace_root_of(&mr, &r, &b_src), Some(b.join("server")), "nothing indexes b: its nearest Cargo.toml");
    r.roots.lock().unwrap().entry(b.clone()).or_default().running_since = Some(SystemTime::now());
    assert_eq!(workspace_root_of(&mr, &r, &b_src), Some(b.clone()), "b's first run: b, not its member crate");
    assert_eq!(r.root_for(&b_src), Some(b.clone()));
    let a_root = workspace_root_of(&mr, &r, &a_src).unwrap();
    assert!(a_root.starts_with(&a) && loaded.is_none_or(|l| l == a_root), "a file of a stays with a: {a_root:?}");

    #[cfg(unix)]
    {
        let link = base.join("link");
        std::os::unix::fs::symlink(&b, &link).unwrap();
        assert_eq!(workspace_root_of(&mr, &r, &link.join("server/src/lib.rs")), Some(b.clone()), "one root through a symlink");
    }
}
