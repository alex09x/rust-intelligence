/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use super::build_session_index_with;
use std::fs;
use std::path::Path;

const VARIANT: &str = "variants/dev-1";

/// A published index of three units: the library of package `a` (a/src/lib.rs), its integration test `a_it`
/// (a/tests/it.rs), and the library of package `b` (b/src/lib.rs), the last two depending on `a`. Like the driver's,
/// their sidecars record the sources of `a` they read through its metadata.
fn published(root: &Path) {
    let variant = root.join("target/ridx").join(VARIANT);
    fs::create_dir_all(variant.join(".deps")).unwrap();
    let lib_a = root.join("a/src/lib.rs");
    for (name, package, src) in [("a", "a", "a/src/lib.rs"), ("a_it", "a", "a/tests/it.rs"), ("b", "b", "b/src/lib.rs")] {
        let src = root.join(src);
        fs::create_dir_all(src.parent().unwrap()).unwrap();
        fs::write(&src, format!("pub fn {name}() {{}}\n")).unwrap();
        let shard = variant.join(format!("{name}-1.ridx"));
        fs::write(&shard, name.to_uppercase()).unwrap();
        let files = serde_json::json!({ src.display().to_string(): 1, lib_a.display().to_string(): 1 });
        let sidecar = serde_json::json!({
            "name": name,
            "compiler_args": ["rustc", "--crate-name", name, "--edition=2021", src.display().to_string()],
            "env": { "CARGO_CRATE_NAME": name, "CARGO_PKG_NAME": package, "CARGO_MANIFEST_DIR": root.join(package).display().to_string() },
            "file_hashes": files,
            "object_path": shard.display().to_string(),
        });
        fs::write(variant.join(format!(".deps/{name}-1.json")), sidecar.to_string()).unwrap();
    }
    fs::write(root.join("target/ridx/ridx_manifest.json"), serde_json::json!({ "variant_dir": VARIANT, "complete": true }).to_string()).unwrap();
}

/// A driver standing in for `ridx-driver`: keeps the buffer list it was given, writes the shard of `a` and rewrites
/// its sidecar in place, as the real one does.
fn fake_driver(dir: &Path) -> std::path::PathBuf {
    let driver = dir.join("fake-driver");
    fs::write(&driver, "#!/bin/sh\necho \"$CARGO_CRATE_NAME\" >> \"$RIDX_OUT_DIR/calls\"\ncp \"$RIDX_BUFFERS\" \"$RIDX_OUT_DIR/seen.json\"\nprintf NEW > \"$RIDX_OUT_DIR/a-1.ridx\"\nprintf '{\"rewritten\":1}' > \"$RIDX_OUT_DIR/.deps/a-1.json\"\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&driver, fs::Permissions::from_mode(0o755)).unwrap();
    }
    driver
}

#[test]
fn test_a_session_index_compiles_the_buffers_crates_and_leaves_the_published_index() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    published(&root);
    let driver = fake_driver(&root);
    let session = root.join("target/ridx/sessions/1-1");
    let a = root.join("a/src/lib.rs");

    let out = build_session_index_with(&driver, &root, &session, &[(a.clone(), "pub fn a() { edited() }\n".to_string())]).unwrap();
    assert_eq!(out, session);
    let variant = session.join(VARIANT);
    assert_eq!(fs::read_to_string(variant.join("a-1.ridx")).unwrap(), "NEW", "the crate holding the buffer is compiled into the session");
    assert_eq!(fs::read_to_string(variant.join("b-1.ridx")).unwrap(), "B", "the other crate is the published one, though it records a s source");
    assert_eq!(fs::read_to_string(variant.join("calls")).unwrap(), "a\n", "only the crate owning the buffer is compiled, not the test or the other package recording it");
    assert!(session.join("ridx_manifest.json").is_file());

    let published = root.join("target/ridx").join(VARIANT);
    assert_eq!(fs::read_to_string(published.join("a-1.ridx")).unwrap(), "A", "the published shard is left as it is");
    assert!(fs::read_to_string(published.join(".deps/a-1.json")).unwrap().contains("object_path"), "so is its sidecar, rewritten in the session only");

    let seen: serde_json::Value = serde_json::from_str(&fs::read_to_string(variant.join("seen.json")).unwrap()).unwrap();
    let text_file = seen[a.display().to_string()].as_str().expect("the buffer is in the driver's list");
    assert_eq!(fs::read_to_string(text_file).unwrap(), "pub fn a() { edited() }\n");
    assert_eq!(fs::read_to_string(&a).unwrap(), "pub fn a() {}\n", "the file on disk is not written");

    // A buffer the compiler stops on before analysis (a parse error) writes no shard: the crate keeps its published one.
    let failing = root.join("failing-driver");
    fs::write(&failing, "#!/bin/sh\nexit 1\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&failing, fs::Permissions::from_mode(0o755)).unwrap();
    }
    build_session_index_with(&failing, &root, &session, &[(a.clone(), "pub fn a( {\n".to_string())]).unwrap();
    assert_eq!(fs::read_to_string(variant.join("a-1.ridx")).unwrap(), "A", "the published shard answers for the crate");

    let elsewhere = root.join("a/src/unindexed.rs");
    assert!(build_session_index_with(&driver, &root, &session, &[(elsewhere, String::new())]).is_err(), "no crate holds the buffer");
}
