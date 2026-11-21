/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use super::{build_session_index_with, json_diagnostics};
use std::fs;
use std::path::{Path, PathBuf};

const VARIANT: &str = "variants/dev-1";

struct UnitSpec<'a> {
    name: &'a str,
    package: &'a str,
    src: &'a str,
    /// Sources recorded in `file_hashes` besides its own (read through dependencies' metadata).
    also_reads: &'a [&'a str],
    /// `own_sources`, when the sidecar has it.
    own: Option<&'a [&'a str]>,
    externs: &'a [&'a str],
    extra_env: &'a [(&'a str, &'a str)],
}

/// A published index of the given units, each with its shard (its name in capitals) and sidecar.
fn published(root: &Path, units: &[UnitSpec], variant_dir: &str) {
    let variant = root.join("target/ridx").join(VARIANT);
    fs::create_dir_all(variant.join(".deps")).unwrap();
    for u in units {
        let src = root.join(u.src);
        fs::create_dir_all(src.parent().unwrap()).unwrap();
        fs::write(&src, format!("pub fn {}() {{}}\n", u.name)).unwrap();
        let shard = variant.join(format!("{}-1.ridx", u.name));
        fs::write(&shard, u.name.to_uppercase()).unwrap();
        let mut files = serde_json::json!({ src.display().to_string(): 1 });
        for r in u.also_reads {
            files[root.join(r).display().to_string()] = 1.into();
        }
        let mut args = vec!["rustc".to_string(), "--crate-name".into(), u.name.into(), "--edition=2021".into(), src.display().to_string()];
        for e in u.externs {
            // `binding=crate` binds a crate under another name, as Cargo does for a renamed dependency.
            // `binding=crate.so`: bound to a dylib, as a proc macro is.
            let (binding, krate) = e.split_once('=').unwrap_or((e, e));
            let file = if krate.contains('.') { format!("lib{krate}") } else { format!("lib{krate}.rmeta") };
            args.extend(["--extern".to_string(), format!("{binding}={}", root.join("target/ridx_cargo").join(file).display())]);
        }
        let mut env = serde_json::json!({ "CARGO_CRATE_NAME": u.name, "CARGO_PKG_NAME": u.package, "CARGO_MANIFEST_DIR": root.join(u.package).display().to_string() });
        for (k, v) in u.extra_env {
            env[*k] = (*v).into();
        }
        let mut sidecar = serde_json::json!({ "name": u.name, "compiler_args": args, "env": env, "file_hashes": files, "object_path": shard.display().to_string() });
        if let Some(own) = u.own {
            sidecar["own_sources"] = own.iter().map(|o| root.join(o).display().to_string()).collect::<Vec<_>>().into();
        }
        fs::write(variant.join(format!(".deps/{}-1.json", u.name)), sidecar.to_string()).unwrap();
    }
    fs::write(root.join("target/ridx/ridx_manifest.json"), serde_json::json!({ "variant_dir": variant_dir, "complete": true }).to_string()).unwrap();
}

/// A driver standing in for `ridx-driver`: logs its crate and arguments, keeps the buffer list, writes its shard and
/// metadata, and rewrites its sidecar in place as the real one does. `extra` runs first.
fn driver(dir: &Path, name: &str, extra: &str) -> PathBuf {
    let path = dir.join(name);
    let script = format!(
        "#!/bin/sh\n{extra}\necho \"$CARGO_CRATE_NAME $*\" >> \"$RIDX_OUT_DIR/calls\"\ncp \"$RIDX_BUFFERS\" \"$RIDX_OUT_DIR/seen.json\"\nprintf NEW > \"$RIDX_OUT_DIR/$CARGO_CRATE_NAME-1.ridx\"\nprintf meta > \"$RIDX_OUT_DIR/../../direct/deps/lib$CARGO_CRATE_NAME.rmeta\"\nprintf '{{\"rewritten\":1}}' > \"$RIDX_OUT_DIR/.deps/$CARGO_CRATE_NAME-1.json\"\n"
    );
    fs::write(&path, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    path
}

fn read(p: PathBuf) -> String {
    fs::read_to_string(p).unwrap_or_default()
}

const A: UnitSpec = UnitSpec { name: "a", package: "a", src: "a/src/lib.rs", also_reads: &[], own: None, externs: &[], extra_env: &[] };
const A_IT: UnitSpec = UnitSpec { name: "a_it", package: "a", src: "a/tests/it.rs", also_reads: &["a/src/lib.rs"], own: None, externs: &["a"], extra_env: &[] };
const B: UnitSpec = UnitSpec { name: "b", package: "b", src: "b/src/lib.rs", also_reads: &["a/src/lib.rs"], own: None, externs: &["a"], extra_env: &[] };

#[test]
fn test_a_session_index_compiles_the_buffers_crates_and_leaves_the_published_index() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    published(&root, &[A, A_IT, B], VARIANT);
    let session = root.join("target/ridx/sessions/1-1");
    let a = root.join("a/src/lib.rs");

    let out = build_session_index_with(&driver(&root, "d", ""), &root, &session, &[(a.clone(), "pub fn a() { edited() }\n".to_string())]).unwrap();
    assert_eq!(out.dir, session);
    assert!(out.messages.is_empty(), "the compiler reported nothing");
    let variant = session.join(VARIANT);
    assert_eq!(read(variant.join("a-1.ridx")), "NEW", "the crate holding the buffer is compiled into the session");
    assert_eq!(read(variant.join("b-1.ridx")), "B", "the other crate is the published one, though it records a's source");
    assert!(read(variant.join("calls")).starts_with("a ") && read(variant.join("calls")).lines().count() == 1, "only a: {}", read(variant.join("calls")));
    let published = root.join("target/ridx").join(VARIANT);
    assert_eq!(read(published.join("a-1.ridx")), "A", "the published shard is left as it is");
    assert!(read(published.join(".deps/a-1.json")).contains("object_path"), "so is its sidecar");
    let seen: serde_json::Value = serde_json::from_str(&read(variant.join("seen.json"))).unwrap();
    assert_eq!(read(PathBuf::from(seen[a.display().to_string()].as_str().unwrap())), "pub fn a() { edited() }\n");
    assert_eq!(read(a.clone()), "pub fn a() {}\n", "the file on disk is not written");
    assert!(build_session_index_with(&driver(&root, "d", ""), &root, &session, &[(root.join("a/src/new.rs"), String::new())]).is_err(), "no crate holds it");
}

/// Review of ca0c1ec7: a compile that writes no shard (a parse error) gets back the shard set aside, even when a reindex
/// replaced the published one meanwhile.
#[test]
fn test_a_failed_compile_keeps_the_shard_it_set_aside() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    published(&root, &[A], VARIANT);
    let published = root.join("target/ridx").join(VARIANT).join("a-1.ridx");
    let failing = driver(&root, "f", &format!("printf AFTER > {0}.tmp && mv {0}.tmp {0}\nexit 1", published.display()));
    let session = root.join("target/ridx/sessions/1-1");
    build_session_index_with(&failing, &root, &session, &[(root.join("a/src/lib.rs"), "pub fn a( {\n".into())]).unwrap();
    assert_eq!(read(session.join(VARIANT).join("a-1.ridx")), "A", "the shard of the publication the session started from");
    assert!(!session.join(VARIANT).join("a-1.ridx.published").exists());
}

/// Review of ca0c1ec7: an absolute or `..` variant directory made the session's copy the published directory.
#[test]
fn test_a_manifest_naming_another_directory_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    for bad in [root.join("target/ridx").join(VARIANT).display().to_string(), "../../x".into(), "variants/..".into()] {
        published(&root, &[A], &bad);
        let r = build_session_index_with(&driver(&root, "d", ""), &root, &root.join("target/ridx/sessions/1-1"), &[(root.join("a/src/lib.rs"), "x".into())]);
        assert!(r.is_err(), "{bad} refused");
        assert_eq!(read(root.join("target/ridx").join(VARIANT).join("a-1.ridx")), "A", "{bad}: published shard intact");
    }
}

/// Review of ca0c1ec7: a `RIDX_OUT_DIR` recorded in the sidecar (a build script can set any variable) sent the compile
/// into the published index.
#[test]
fn test_a_recorded_environment_cannot_redirect_the_session() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let published_dir = root.join("target/ridx").join(VARIANT).display().to_string();
    let a = UnitSpec { extra_env: &[("RIDX_OUT_DIR", published_dir.as_str()), ("RIDX_BUFFERS", "/nonexistent")], ..A };
    published(&root, &[a], VARIANT);
    let session = root.join("target/ridx/sessions/1-1");
    build_session_index_with(&driver(&root, "d", ""), &root, &session, &[(root.join("a/src/lib.rs"), "x".into())]).unwrap();
    assert_eq!(read(root.join("target/ridx").join(VARIANT).join("a-1.ridx")), "A");
    assert_eq!(read(session.join(VARIANT).join("a-1.ridx")), "NEW");
}

/// Review of ca0c1ec7: a file included with `#[path]` by two packages is compiled in both, and an edited library is
/// read by an edited consumer from the session's metadata, compiled first.
#[test]
fn test_every_owner_is_compiled_and_reads_the_session_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let a = UnitSpec { own: Some(&["a/src/lib.rs"]), ..A };
    let b = UnitSpec { own: Some(&["b/src/lib.rs", "a/src/lib.rs"]), ..B };
    let a_it = UnitSpec { own: Some(&["a/tests/it.rs"]), ..A_IT };
    published(&root, &[a, b, a_it], VARIANT);
    let session = root.join("target/ridx/sessions/1-1");
    build_session_index_with(&driver(&root, "d", ""), &root, &session, &[(root.join("a/src/lib.rs"), "x".into())]).unwrap();
    let calls = read(session.join(VARIANT).join("calls"));
    let lines: Vec<&str> = calls.lines().collect();
    assert_eq!(lines.len(), 2, "a and b own the file, a_it only reads it: {calls}");
    assert!(lines[0].starts_with("a ") && lines[1].starts_with("b "), "the library first: {calls}");
    let fresh = session.join("direct/deps/liba.rmeta").display().to_string();
    assert!(lines[1].contains(&format!("a={fresh}")), "b reads a's session metadata: {calls}");
}

/// Review of 11310378: a consumer binding an edited library under another name (`renamed=liba.rmeta`) still read the
/// saved metadata.
#[test]
fn test_a_renamed_dependency_reads_the_session_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let a = UnitSpec { own: Some(&["a/src/lib.rs"]), ..A };
    let b = UnitSpec { own: Some(&["b/src/lib.rs", "a/src/lib.rs"]), externs: &["renamed=a"], ..B };
    published(&root, &[a, b], VARIANT);
    let session = root.join("target/ridx/sessions/1-1");
    build_session_index_with(&driver(&root, "d", ""), &root, &session, &[(root.join("a/src/lib.rs"), "x".into())]).unwrap();
    let calls = read(session.join(VARIANT).join("calls"));
    let fresh = session.join("direct/deps/liba.rmeta").display().to_string();
    let b_line = calls.lines().find(|l| l.starts_with("b ")).unwrap_or_default();
    assert!(b_line.contains(&format!("renamed={fresh}")), "{calls}");
}

/// Review of a21afbc0: a proc macro is bound as a dylib, which a session (metadata only) never writes; its consumer
/// was rebound to a session file that does not exist.
#[test]
fn test_a_proc_macro_consumer_keeps_the_saved_dylib() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let a = UnitSpec { own: Some(&["a/src/lib.rs"]), ..A };
    let b = UnitSpec { own: Some(&["b/src/lib.rs", "a/src/lib.rs"]), externs: &["a=a.so"], ..B };
    published(&root, &[a, b], VARIANT);
    let session = root.join("target/ridx/sessions/1-1");
    build_session_index_with(&driver(&root, "d", ""), &root, &session, &[(root.join("a/src/lib.rs"), "x".into())]).unwrap();
    let calls = read(session.join(VARIANT).join("calls"));
    let saved = root.join("target/ridx_cargo/liba.so").display().to_string();
    let b_line = calls.lines().find(|l| l.starts_with("b ")).unwrap_or_default();
    assert!(b_line.contains(&format!("a={saved}")), "{calls}");
}

/// Review of 503071cb: one binding naming an rlib and a dylib had both rebound when only the rlib had a replacement.
#[test]
fn test_each_bound_file_is_rebound_on_its_own() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let a = UnitSpec { own: Some(&["a/src/lib.rs"]), ..A };
    let b = UnitSpec { own: Some(&["b/src/lib.rs", "a/src/lib.rs"]), externs: &["a", "a=a.so"], ..B };
    published(&root, &[a, b], VARIANT);
    let session = root.join("target/ridx/sessions/1-1");
    build_session_index_with(&driver(&root, "d", ""), &root, &session, &[(root.join("a/src/lib.rs"), "x".into())]).unwrap();
    let calls = read(session.join(VARIANT).join("calls"));
    let b_line = calls.lines().find(|l| l.starts_with("b ")).unwrap_or_default();
    let fresh = session.join("direct/deps/liba.rmeta").display().to_string();
    let saved_dylib = root.join("target/ridx_cargo/liba.so").display().to_string();
    assert!(b_line.contains(&format!("a={fresh}")) && b_line.contains(&format!("a={saved_dylib}")), "{calls}");
}

/// What the compiler reports on the buffers' text comes back with the session, its diagnostics only (prod-code,
/// m=20120: errors typed and never saved went unreported).
#[test]
fn test_the_compilers_diagnostics_come_back_with_the_session() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    published(&root, &[A], VARIANT);
    let report = r#"printf '%s\n' '{"$message_type":"diagnostic","message":"mismatched types","spans":[]}' 'warning: not json' '{"$message_type":"artifact","artifact":"x"}' >&2"#;
    let out = build_session_index_with(&driver(&root, "d", report), &root, &root.join("target/ridx/sessions/1-1"), &[(root.join("a/src/lib.rs"), "x".into())]).unwrap();
    assert_eq!(out.messages.len(), 1, "{:?}", out.messages);
    assert_eq!(out.messages[0]["message"], "mismatched types");
    assert!(read(out.dir.join(VARIANT).join("calls")).contains("--error-format=json"), "asked for JSON");
}

#[test]
fn test_the_recorded_format_gives_way_to_json() {
    let args: Vec<String> = ["rustc", "--error-format", "human", "--json=diagnostic-rendered-ansi,artifacts", "--color", "always", "--color=never", "--edition=2021", "lib.rs"]
        .map(String::from)
        .into();
    assert_eq!(json_diagnostics(args), ["rustc", "--edition=2021", "lib.rs", "--error-format=json"]);
}
