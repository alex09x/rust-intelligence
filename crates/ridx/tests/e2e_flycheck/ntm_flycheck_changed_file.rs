/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! A file rewritten while `cargo check` runs gets none of that check's diagnostics.

use std::fs;
use std::time::Duration;

use crate::PATH_MUTEX;

/// In Zed: a save starts the check, `git checkout -- src/main.rs` reverts the file before it ends. The warning about
/// the saved text (`unused variable: messy` on its line 2) was mapped onto the reverted text and underlined a
/// different name. The reverted file was never checked, so it shows nothing; a file left alone keeps its diagnostics.
#[test]
fn test_flycheck_drops_diagnostics_of_a_file_changed_during_the_check() {
    let _path_lock = PATH_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    use ridx::lsp::presentation::flycheck::FlycheckState;
    use ridx::lsp::protocol::path_to_uri;
    use std::sync::{Arc, Mutex};

    let tmp = std::env::temp_dir().join(format!("test_flycheck_changed_{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    let (bin_dir, ws_dir) = (tmp.join("bin"), tmp.join("workspace"));
    let src_dir = ws_dir.join("src");
    fs::create_dir_all(&bin_dir).unwrap();
    fs::create_dir_all(&src_dir).unwrap();
    fs::write(ws_dir.join("Cargo.toml"), "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n").unwrap();
    let (main_file, lib_file) = (src_dir.join("main.rs"), src_dir.join("lib.rs"));
    fs::write(&main_file, "fn main() {\n    let messy = 1;\n}\n").unwrap();
    fs::write(&lib_file, "pub fn f() {}\n").unwrap();

    let mock_cargo = bin_dir.join("cargo");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let script = format!(
            r#"#!/bin/sh
sleep 0.05
printf 'fn main() {{\n    run();\n}}\n' > "{}"
echo '{{"reason":"compiler-message","message":{{"message":"unused variable: `messy`","level":"warning","spans":[{{"file_name":"src/main.rs","is_primary":true,"line_start":2,"column_start":9,"line_end":2,"column_end":14}}]}}}}'
echo '{{"reason":"compiler-message","message":{{"message":"lib untouched","level":"warning","spans":[{{"file_name":"src/lib.rs","is_primary":true,"line_start":1,"column_start":8,"line_end":1,"column_end":9}}]}}}}'
exit 0
"#,
            main_file.display()
        );
        fs::write(&mock_cargo, script).unwrap();
        fs::set_permissions(&mock_cargo, fs::Permissions::from_mode(0o755)).unwrap();
    }

    let state = FlycheckState::new();
    let captured = Arc::new(Mutex::new(Vec::new()));
    let cap = Arc::clone(&captured);
    state.set_notifier(move |_gen, notif| cap.lock().unwrap().push(notif));
    state.trigger_check(&ws_dir);

    // The untouched file's report comes once the run's diagnostics have been sorted.
    let start = std::time::Instant::now();
    while !captured.lock().unwrap().iter().any(|n| n.to_string().contains("lib untouched")) {
        assert!(start.elapsed() < Duration::from_secs(10), "no diagnostics for lib.rs: {:?}", *captured.lock().unwrap());
        std::thread::sleep(Duration::from_millis(10));
    }
    std::thread::sleep(Duration::from_millis(100));

    let notifs = captured.lock().unwrap().clone();
    assert!(!notifs.iter().any(|n| n.to_string().contains("messy")), "a warning about the text no longer on disk: {notifs:?}");
    let main_uri = path_to_uri(main_file.to_str().unwrap());
    let lib_uri = path_to_uri(lib_file.to_str().unwrap());
    assert!(state.get_latest_diagnostics(&main_uri).is_empty(), "{:?}", state.get_latest_diagnostics(&main_uri));
    assert_eq!(state.get_latest_diagnostics(&lib_uri).len(), 1);

    state.cancel();
    state.shutdown();
    let _ = fs::remove_dir_all(&tmp);
}
