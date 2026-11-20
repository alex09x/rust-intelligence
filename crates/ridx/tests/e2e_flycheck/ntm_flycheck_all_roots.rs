/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use std::fs;
use std::time::Duration;

use crate::PATH_MUTEX;

/// Review of a34d8100: the check on start took the first workspace folder only, and a check per folder would have
/// cancelled the one before. Two independent workspaces are both checked.
#[test]
fn test_flycheck_checks_every_root_one_after_the_other() {
    let _path_lock = PATH_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    use ridx::lsp::presentation::flycheck::FlycheckState;
    use std::sync::{Arc, Mutex};

    let tmp = std::env::temp_dir().join(format!("test_flycheck_all_roots_{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    let bin_dir = tmp.join("bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let roots: Vec<_> = ["one", "two"].iter().map(|name| tmp.join(name)).collect();
    for root in &roots {
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("Cargo.toml"), "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n").unwrap();
        fs::write(root.join("src/lib.rs"), "pub fn f() {}\n").unwrap();
    }
    // A cargo that reports one warning naming the workspace it ran in (the parent's bin/cargo is the one used).
    let mock_cargo = bin_dir.join("cargo");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let script = r#"#!/bin/sh
echo "{\"reason\":\"compiler-message\",\"message\":{\"message\":\"checked $(basename "$PWD")\",\"level\":\"warning\",\"spans\":[{\"file_name\":\"src/lib.rs\",\"is_primary\":true,\"line_start\":1,\"column_start\":8,\"line_end\":1,\"column_end\":9}]}}"
exit 0
"#;
        fs::write(&mock_cargo, script).unwrap();
        fs::set_permissions(&mock_cargo, fs::Permissions::from_mode(0o755)).unwrap();
    }

    let state = FlycheckState::new();
    let captured = Arc::new(Mutex::new(Vec::new()));
    let cap = Arc::clone(&captured);
    state.set_notifier(move |_gen, notif| cap.lock().unwrap().push(notif.to_string()));
    state.trigger_checks(roots.clone());

    let start = std::time::Instant::now();
    let seen = |name: &str| captured.lock().unwrap().iter().any(|n| n.contains(&format!("checked {name}")));
    while !(seen("one") && seen("two")) {
        assert!(start.elapsed() < Duration::from_secs(10), "both roots checked: {:?}", *captured.lock().unwrap());
        std::thread::sleep(Duration::from_millis(10));
    }
    state.shutdown();
    let _ = fs::remove_dir_all(&tmp);
}

/// Review of ca0c1ec7: a save in the first root while the check on start ran there cancelled the batch, and the second
/// root was never checked.
#[test]
fn test_a_save_during_the_checks_on_start_keeps_the_other_roots() {
    let _path_lock = PATH_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    use ridx::lsp::presentation::flycheck::FlycheckState;
    use std::sync::{Arc, Mutex};

    let tmp = std::env::temp_dir().join(format!("test_flycheck_save_keeps_roots_{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    let bin_dir = tmp.join("bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let roots: Vec<_> = ["one", "two"].iter().map(|name| tmp.join(name)).collect();
    for root in &roots {
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("Cargo.toml"), "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n").unwrap();
        fs::write(root.join("src/lib.rs"), "pub fn f() {}\n").unwrap();
    }
    // The first check of `one` is slow, so that the save arrives while it runs.
    let mock_cargo = bin_dir.join("cargo");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let script = r#"#!/bin/sh
n=$(basename "$PWD")
if [ "$n" = one ] && [ ! -f ../slow-done ]; then touch ../slow-done; sleep 2; fi
echo "{\"reason\":\"compiler-message\",\"message\":{\"message\":\"checked $n\",\"level\":\"warning\",\"spans\":[{\"file_name\":\"src/lib.rs\",\"is_primary\":true,\"line_start\":1,\"column_start\":8,\"line_end\":1,\"column_end\":9}]}}"
exit 0
"#;
        fs::write(&mock_cargo, script).unwrap();
        fs::set_permissions(&mock_cargo, fs::Permissions::from_mode(0o755)).unwrap();
    }

    let state = FlycheckState::new();
    let captured = Arc::new(Mutex::new(Vec::new()));
    let cap = Arc::clone(&captured);
    state.set_notifier(move |_gen, notif| cap.lock().unwrap().push(notif.to_string()));
    state.trigger_checks(roots.clone());
    let start = std::time::Instant::now();
    while !tmp.join("slow-done").exists() {
        assert!(start.elapsed() < Duration::from_secs(10), "the first check started");
        std::thread::sleep(Duration::from_millis(10));
    }
    state.trigger_check(&roots[0]);

    let seen = |name: &str| captured.lock().unwrap().iter().any(|n| n.contains(&format!("checked {name}")));
    while !(seen("one") && seen("two")) {
        assert!(start.elapsed() < Duration::from_secs(10), "both roots checked after the save: {:?}", *captured.lock().unwrap());
        std::thread::sleep(Duration::from_millis(10));
    }
    state.shutdown();
    let _ = fs::remove_dir_all(&tmp);
}
