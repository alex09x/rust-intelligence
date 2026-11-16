/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! A reindex in which a crate does not build keeps that crate in the index: an invalidated crate whose dependency's
//! build script fails (a `cmake` missing from the `PATH` the editor started the server with) keeps its last good
//! shards instead of dropping out, and with it every hover and definition in its files.

use super::*;
use crate::indexer::driver::find_driver_binary;
use crate::indexer::orchestrate::try_run_indexing;
use crate::indexer::{IndexBuilder, IndexOptions};
use ridx_format::IndexReader;
use std::path::Path;

fn shard_defs(variant: &Path, krate: &str) -> Option<Vec<String>> {
    let shard = fs::read_dir(variant)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|x| x == "ridx") && p.file_stem().and_then(|s| s.to_str()).is_some_and(|s| s.rsplit_once('-').is_some_and(|(c, _)| c == krate)))?;
    let r = IndexReader::open(&shard).ok()?;
    Some(r.definitions().iter().map(|d| r.get_str(d.name_offset()).to_string()).collect())
}

fn write(ws: &Path, rel: &str, text: &str) {
    let p = ws.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}

#[test]
fn test_a_crate_that_cannot_be_rebuilt_keeps_its_last_good_index() {
    let _lock = lock_indexer_test();
    let Some(driver) = find_driver_binary() else { return };
    let dir = new_test_dir("ridx_failed_build");
    let (ws, cas) = (dir.0.join("ws"), dir.0.join("cas"));
    write(&ws, "Cargo.toml", "[workspace]\nmembers = [\"app\", \"native\", \"pm\"]\nresolver = \"2\"\n");
    // `native` builds through a script, which fails while `native/fail` exists.
    write(&ws, "native/Cargo.toml", "[package]\nname = \"native\"\nversion = \"0.1.0\"\nedition = \"2021\"\n");
    write(
        &ws,
        "native/build.rs",
        "fn main() {\n    println!(\"cargo:rerun-if-changed=fail\");\n    if std::path::Path::new(\"fail\").exists() {\n        panic!(\"is `cmake` not installed?\");\n    }\n}\n",
    );
    write(&ws, "native/src/lib.rs", "pub fn native_api() -> u32 { 1 }\n");
    // A proc-macro crate: an edit of it is indexed through Cargo, never on the direct path.
    write(&ws, "pm/Cargo.toml", "[package]\nname = \"pm\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[lib]\nproc-macro = true\n");
    write(&ws, "pm/src/lib.rs", "use proc_macro::TokenStream;\n#[proc_macro]\npub fn noop(_: TokenStream) -> TokenStream { TokenStream::new() }\n");
    write(
        &ws,
        "app/Cargo.toml",
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\nnative = { path = \"../native\" }\npm = { path = \"../pm\" }\n",
    );
    write(&ws, "app/src/lib.rs", "pm::noop!();\npub fn app_api() -> u32 { native::native_api() }\n");
    let opts = || IndexOptions {
        path: ws.clone(),
        out_dir_opt: None,
        package: None,
        release: false,
        features: None,
        no_default_features: false,
        all_features: false,
        target: None,
        cache_dir: Some(cas.clone()),
        driver_bin: Some(driver.clone()),
        capture_child_output: true,
        builder: Some(IndexBuilder::Cargo),
    };
    // The first run writes `Cargo.lock`, which names the variant: the second one stands for the workspace as it is.
    try_run_indexing(opts()).expect("lockfile run");
    let first = try_run_indexing(opts()).expect("first index");
    assert!(shard_defs(&first.variant_out_dir, "app").expect("app indexed").iter().any(|d| d == "app_api"));

    // `pm` is edited, which invalidates `app`; `native` no longer builds, so neither does `app`.
    write(&ws, "native/fail", "");
    write(&ws, "pm/src/lib.rs", "use proc_macro::TokenStream;\n#[proc_macro]\npub fn noop(_: TokenStream) -> TokenStream { TokenStream::new() }\npub(crate) fn edited() {}\n");
    let second = try_run_indexing(opts()).expect("a crate that did not build does not fail the run");
    assert_eq!(second.variant_out_dir, first.variant_out_dir, "the same variant is reindexed in place");
    let app = shard_defs(&second.variant_out_dir, "app").expect("app keeps its last good shard");
    assert!(app.iter().any(|d| d == "app_api"), "{app:?}");
    assert!(shard_defs(&second.variant_out_dir, "pm").is_some_and(|d| d.iter().any(|d| d == "edited")), "pm built anew");
    assert!(!second.variant_out_dir.join(crate::indexer::feature_seed::STALE_DIR).exists(), "the set-aside shards are cleaned up");

    // Once `native` builds again, so does `app`.
    fs::remove_file(ws.join("native/fail")).unwrap();
    write(&ws, "app/src/lib.rs", "pm::noop!();\npub fn app_api() -> u32 { native::native_api() }\npub fn typed_since() {}\n");
    let third = try_run_indexing(opts()).expect("third index");
    let app = shard_defs(&third.variant_out_dir, "app").expect("app indexed");
    assert!(app.iter().any(|d| d == "typed_since"), "{app:?}");
}
