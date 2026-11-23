/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! A name completion from elsewhere in the workspace comes with a `use` that compiles. Typing `-> HashSe` in BTCR
//! listed impl blocks (`use <AHashSet<T, S> as Clone>;`), associated functions (`use HashSetExt::new;`), paths without
//! their crate (`use hash_set::AHashSet;`) and another module's private import (`use archive_version::HashSet;`).

use super::failed_build_tests::{index_opts, write, write_native_and_app};
use super::lsp_server::Server;
use super::*;
use crate::indexer::driver::find_driver_binary;
use crate::indexer::orchestrate::try_run_indexing;
use serde_json::Value;

/// `(label, import)` of each item: the `use` its additional edit writes, if any.
fn offered(items: &[Value]) -> Vec<(String, Option<String>)> {
    items
        .iter()
        .map(|i| {
            let import = i["additionalTextEdits"].as_array().and_then(|e| e.first()).and_then(|e| e["newText"].as_str()).map(|t| t.trim().to_string());
            (i["label"].as_str().unwrap_or_default().to_string(), import)
        })
        .collect()
}

#[test]
fn test_name_completion_imports_only_what_a_use_reaches() {
    let _lock = lock_indexer_test();
    let Some(driver) = find_driver_binary() else { return };
    let dir = new_test_dir("ridx_completion_import");
    let (ws, cas) = (dir.0.join("ws"), dir.0.join("cas"));
    // `Reexported` is imported inside `mod tests` only: a function outside it still needs its own `use`.
    let app = "use std::collections::HashMap;\nmod util;\npub fn uses_map(m: HashMap<u8, u8>) -> usize { util::helper(Default::default()); m.len() + native::native_api() as usize }\n#[cfg(test)]\nmod tests {\n    #[allow(unused_imports)]\n    use native::Reexported;\n}\n";
    write_native_and_app(&ws, app);
    // Another module of the crate, with a private import of `HashSet` and a private function.
    write(&ws, "app/src/util.rs", "use std::collections::HashSet;\npub fn helper(_: HashSet<u8>) {}\n#[allow(dead_code)]\nfn private_helper() {}\n");
    write(
        &ws,
        "native/src/lib.rs",
        "pub fn native_api() -> u32 { 1 }\nmod hidden {\n    pub struct Reexported;\n    pub struct NotExported;\n}\npub mod shown {\n    pub struct Shown;\n    struct Private;\n    impl Clone for Shown { fn clone(&self) -> Self { Shown } }\n}\npub use hidden::Reexported;\n",
    );
    try_run_indexing(index_opts(&ws, &cas, &driver)).expect("index");
    let mut server = Server::start(&ws);
    let lib = ws.join("app/src/lib.rs");

    let mut at = |typed: &str| {
        let text = format!("{app}pub fn typed() -> {typed}");
        let uri = server.open_with(&lib, &text);
        let line = text.lines().count() as u32 - 1;
        offered(&server.completion(&uri, line, text.lines().last().unwrap().len() as u32))
    };

    let hash_set = at("HashSe");
    assert!(hash_set.iter().all(|(l, _)| l.chars().all(|c| c.is_alphanumeric() || c == '_')), "names only, no impl blocks: {hash_set:?}");
    assert!(hash_set.contains(&("HashSet".into(), Some("use std::collections::HashSet;".into()))), "std's public path: {hash_set:?}");
    let first_import = hash_set.iter().find_map(|(_, u)| u.clone());
    assert_eq!(first_import.as_deref(), Some("use std::collections::HashSet;"), "std first among imports: {hash_set:?}");
    assert!(hash_set.iter().all(|(_, u)| u.as_deref().is_none_or(|u| !u.contains("util::HashSet"))), "not another module's private import: {hash_set:?}");

    let map = at("HashMa");
    assert!(map.iter().any(|(l, u)| l == "HashMap" && u.is_none()), "already imported: no second use: {map:?}");

    let re = at("Reexp");
    assert!(re.contains(&("Reexported".into(), Some("use native::Reexported;".into()))), "the crate root's re-export: {re:?}");
    assert!(at("NotExpo").iter().all(|(l, _)| l != "NotExported"), "in a private module, re-exported nowhere");
    assert!(at("Show").contains(&("Shown".into(), Some("use native::shown::Shown;".into()))));
    assert!(at("Priva").iter().all(|(l, _)| l != "Private"), "not pub");
    assert!(at("helpe").contains(&("helper".into(), Some("use crate::util::helper;".into()))), "the crate's own pub fn");
    assert!(at("private_he").iter().all(|(l, _)| l != "private_helper"), "the crate's own private fn");

    // Typed inside `mod tests`: its own `use` counts, and a new one goes there, at its indentation.
    let in_tests = app.replace("    use native::Reexported;\n}\n", "    use native::Reexported;\n    fn t() { let _ = Show }\n}\n");
    let uri = server.open_with(&lib, &in_tests);
    let line = in_tests.lines().position(|l| l.contains("fn t()")).unwrap() as u32;
    let col = in_tests.lines().nth(line as usize).unwrap().find("Show").unwrap() as u32 + 4;
    let items = server.completion(&uri, line, col);
    let shown = items.iter().find(|i| i["label"] == "Shown").expect("Shown offered");
    let edit = &shown["additionalTextEdits"][0];
    assert_eq!(edit["newText"], "    use native::shown::Shown;\n", "indented as the module's items");
    assert_eq!(edit["range"]["start"]["line"], line, "right after `mod tests`' last use");
    let re_in = server.completion(&uri, line, col - 4);
    assert!(offered(&re_in).iter().all(|(l, u)| l != "Reexported" || u.is_none()), "imported by `mod tests` itself");

    // The package's binary uses its library as another crate: `app::util::helper` is not `crate::` from `main.rs`.
    write(&ws, "app/src/lib.rs", &app.replace("mod util;", "pub mod util;").replace("pub fn uses_map", "pub(crate) fn lib_secret() {}\npub fn uses_map"));
    let main = ws.join("app/src/main.rs");
    write(&ws, "app/src/main.rs", "mod cli;\nfn main() { cli::run(); }\n");
    write(&ws, "app/src/cli.rs", "pub fn run() {}\n");
    write(&ws, "app/src/bin/tool.rs", "pub fn tool_only() {}\nfn main() { tool_only(); }\n");
    try_run_indexing(index_opts(&ws, &cas, &driver)).expect("reindex");
    let mut server = Server::start(&ws);
    let mut in_file = |file: &std::path::Path, text: &str, typed: &str| {
        let text = format!("{text}fn typed() {{ {typed}");
        let uri = server.open_with(file, &text);
        let line = text.lines().count() as u32 - 1;
        offered(&server.completion(&uri, line, text.lines().last().unwrap().len() as u32))
    };
    let items = in_file(&main, "mod cli;\nfn main() { cli::run(); }\n", "helpe");
    assert!(items.contains(&("helper".into(), Some("use app::util::helper;".into()))), "through the library: {items:?}");
    let items = in_file(&ws.join("app/src/cli.rs"), "pub fn run() {}\n", "helpe");
    assert!(items.contains(&("helper".into(), Some("use app::util::helper;".into()))), "from a module of the binary too: {items:?}");
    let items = in_file(&main, "mod cli;\nfn main() { cli::run(); }\n", "lib_sec");
    assert!(items.iter().all(|(l, u)| l != "lib_secret" || u.is_none()), "the library's pub(crate) item is no import of the binary: {items:?}");
    // An item of another binary (`src/bin/tool.rs`) is seen from neither the main binary nor the library.
    let items = in_file(&main, "mod cli;\nfn main() { cli::run(); }\n", "tool_on");
    assert!(items.iter().all(|(l, u)| l != "tool_only" || u.is_none()), "from main.rs: {items:?}");
    let items = in_file(&lib, app, "tool_on");
    assert!(items.iter().all(|(l, u)| l != "tool_only" || u.is_none()), "from the library: {items:?}");
}

/// Completion right after `self.` (`self.|total += amount;`) offers the fields and methods, in an indexed file opened
/// as it is on disk (prod-code, issue #3: `items: []`).
#[test]
fn test_completion_after_self_dot_offers_fields_and_methods() {
    let _lock = lock_indexer_test();
    let Some(driver) = find_driver_binary() else { return };
    let dir = new_test_dir("ridx_completion_self");
    let (ws, cas) = (dir.0.join("ws"), dir.0.join("cas"));
    let app = "pub struct Counter {\n    total: u32,\n}\n\nimpl Counter {\n    pub fn add(&mut self, amount: u32) -> u32 {\n        self.total += amount;\n        self.total\n    }\n}\n";
    write_native_and_app(&ws, app);
    try_run_indexing(index_opts(&ws, &cas, &driver)).expect("index");
    let mut server = Server::start(&ws);
    let uri = server.open(&ws.join("app/src/lib.rs"));
    let expect = |server: &mut Server, uri: &str, case: &str| {
        for (line, col) in [(6, 13), (7, 13)] {
            let labels: Vec<String> = offered(&server.completion(uri, line, col)).into_iter().map(|(l, _)| l).collect();
            assert!(labels.iter().any(|l| l == "total"), "{case}: the field at {line}:{col}: {labels:?}");
            assert!(labels.iter().any(|l| l.starts_with("add")), "{case}: the method at {line}:{col}: {labels:?}");
        }
    };
    expect(&mut server, &uri, "as indexed");
    // The struct typed in a buffer of a file indexed with other text, and in a file the index has not seen.
    write(&ws, "app/src/lib.rs", "pub fn other() {}\n");
    try_run_indexing(index_opts(&ws, &cas, &driver)).expect("reindex");
    let mut server = Server::start(&ws);
    let uri = server.open_with(&ws.join("app/src/lib.rs"), app);
    expect(&mut server, &uri, "typed over another text");
    let uri = server.open_with(&ws.join("app/src/fresh.rs"), app);
    expect(&mut server, &uri, "a file not indexed");
}
