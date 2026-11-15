/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use super::runnable_lenses;
use crate::lsp::presentation::lens_runnable_hints::{FnSite, RunnableHints};
use serde_json::Value;

fn package(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"demo-pkg\"\nversion = \"0.1.0\"\n").unwrap();
    for (rel, text) in files {
        let p = dir.path().join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, text).unwrap();
    }
    dir
}

fn lenses(dir: &tempfile::TempDir, rel: &str) -> Vec<Value> {
    let file = dir.path().join(rel);
    let text = std::fs::read_to_string(&file).unwrap();
    runnable_lenses(file.to_str().unwrap(), &text, true, true, &RunnableHints::default())
}

fn summary(l: &Value) -> (String, u64, String, Vec<String>) {
    let arg = &l["command"]["arguments"][0];
    let cargo = arg["args"]["cargoArgs"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
    (l["command"]["title"].as_str().unwrap().to_string(), l["range"]["start"]["line"].as_u64().unwrap(), arg["label"].as_str().unwrap().to_string(), cargo)
}

#[test]
fn library_tests_modules_and_doctests() {
    let lib = concat!(
        "/// Adds.\n",
        "///\n",
        "/// ```\n",
        "/// assert_eq!(demo_pkg::add(1, 1), 2);\n",
        "/// ```\n",
        "pub fn add(a: u32, b: u32) -> u32 { a + b }\n",
        "#[cfg(test)]\n",
        "mod tests {\n",
        "    #[test]\n",
        "    fn adds() { assert_eq!(super::add(1, 1), 2); }\n",
        "    #[tokio::test]\n",
        "    async fn adds_async() {}\n",
        "    fn helper() {}\n",
        "}\n",
    );
    let dir = package(&[("src/lib.rs", lib)]);
    let got: Vec<_> = lenses(&dir, "src/lib.rs").iter().map(summary).collect();
    let lib_args = |extra: &[&str]| -> Vec<String> { ["test", "--package", "demo-pkg", "--lib"].iter().chain(extra).map(|s| s.to_string()).collect() };
    // The file's only tests sit in one `mod tests`: that module runs, the file gets no lens of its own (rust-analyzer
    // runs a module with a test of its own or several test submodules).
    assert_eq!(got[0], ("▶︎ Run Doctest".into(), 0, "doctest add".into(), vec!["test".into(), "--doc".into(), "--package".into(), "demo-pkg".into()]));
    assert_eq!(got[1].0, "⚙︎ Debug");
    assert_eq!(got[2], ("▶︎ Run Tests".into(), 7, "test-mod tests".into(), lib_args(&[])));
    assert_eq!(got[4], ("▶︎ Run Test".into(), 9, "test tests::adds".into(), lib_args(&[])));
    assert_eq!(got[6], ("▶︎ Run Test".into(), 11, "test tests::adds_async".into(), lib_args(&[])));
    assert_eq!(got.len(), 8, "`helper` is no runnable: {got:?}");
    // A test of the file's own makes the file a test module.
    let own = package(&[("src/lib.rs", "#[test]\nfn top() {}\n")]);
    let got: Vec<_> = lenses(&own, "src/lib.rs").iter().map(summary).collect();
    assert_eq!((got[0].0.as_str(), got[0].1, got[0].2.as_str()), ("▶︎ Run Tests", 0, "test-mod "));
}

#[test]
fn a_declared_test_module_runs_from_its_mod_item() {
    // `mod a;` whose file holds a test: the lens sits on the declaration's name; one such module makes the
    // declaring file no test module of its own.
    let dir = package(&[("src/lib.rs", "pub fn f() {}\nmod a;\n"), ("src/a.rs", "#[test]\nfn t() {}\n")]);
    let got: Vec<_> = lenses(&dir, "src/lib.rs").iter().map(summary).collect();
    assert_eq!(got.iter().map(|g| (g.0.as_str(), g.1, g.2.as_str())).collect::<Vec<_>>(), [("▶︎ Run Tests", 1, "test-mod a"), ("⚙︎ Debug", 1, "test-mod a")]);
}

#[test]
fn example_main_and_integration_test_targets() {
    let dir = package(&[
        ("examples/walk.rs", "use std::env;\nfn main() {\n    let _ = env::args();\n}\n"),
        ("tests/cli.rs", "#[test]\nfn runs() {}\n"),
        ("src/lib.rs", "pub fn main() {}\n"),
    ]);
    let ex: Vec<_> = lenses(&dir, "examples/walk.rs").iter().map(summary).collect();
    assert_eq!(ex[0], ("▶︎ Run ".into(), 1, "run walk".into(), ["run", "--package", "demo-pkg", "--example", "walk"].map(String::from).to_vec()));
    let it: Vec<_> = lenses(&dir, "tests/cli.rs").iter().map(summary).collect();
    assert_eq!(it[2], ("▶︎ Run Test".into(), 1, "test runs".into(), ["test", "--package", "demo-pkg", "--test", "cli"].map(String::from).to_vec()));
    assert!(lenses(&dir, "src/lib.rs").is_empty(), "a library `main` is an ordinary function");
}

#[test]
fn no_lenses_without_client_commands_or_package() {
    let dir = package(&[("src/lib.rs", "#[test]\nfn t() {}\n")]);
    let file = dir.path().join("src/lib.rs");
    let none = RunnableHints::default();
    assert!(runnable_lenses(file.to_str().unwrap(), "#[test]\nfn t() {}\n", false, false, &none).is_empty());
    assert!(runnable_lenses("/nonexistent/dir/x.rs", "#[test]\nfn t() {}\n", true, true, &none).is_empty());
}

#[test]
fn method_doctests_and_files_outside_the_target_layout() {
    let lib = "pub struct Ring;\nimpl<T> From<T> for Ring {\n    /// ```\n    /// let _ = 1;\n    /// ```\n    pub fn from(_: T) -> Self { Ring }\n}\n";
    let fixture = "#[test]\nfn looks_like_a_test() {}\n";
    let dir = package(&[("src/lib.rs", lib), ("test_data/case.rs", fixture)]);
    let got: Vec<_> = lenses(&dir, "src/lib.rs").iter().map(summary).collect();
    assert_eq!((got[0].0.as_str(), got[0].1, got[0].2.as_str()), ("▶︎ Run Doctest", 2, "doctest Ring::from"));
    // Test data outside Cargo's layout is not compiled: only the index could make it runnable.
    assert!(lenses(&dir, "test_data/case.rs").is_empty());
}

#[test]
fn index_hints_add_macro_generated_tests_and_main() {
    // `case!(empty_text)` expands to a `#[test] fn empty_text`; only the index knows it is a test.
    let lib = "macro_rules! case { ($n:ident) => { #[test] fn $n() {} }; }\nmod tests {\n    case!(empty_text);\n}\n";
    let dir = package(&[("src/lib.rs", lib), ("benches/speed.rs", "criterion_main!(benches);\n")]);
    let mut hints = RunnableHints::default();
    hints.tests.insert(2, vec![FnSite { col16: 10, len16: 10, name: "empty_text".into() }]);
    let file = dir.path().join("src/lib.rs");
    let got: Vec<_> = runnable_lenses(file.to_str().unwrap(), lib, true, false, &hints).iter().map(summary).collect();
    let titles: Vec<_> = got.iter().map(|g| (g.0.as_str(), g.1, g.2.as_str())).collect();
    assert_eq!(titles, [("▶︎ Run Tests", 1, "test-mod tests"), ("▶︎ Run Test", 2, "test tests::empty_text")]);

    let mut bench_hints = RunnableHints::default();
    bench_hints.mains.insert(0, FnSite { col16: 0, len16: 15, name: "main".into() });
    let bench = dir.path().join("benches/speed.rs");
    let got: Vec<_> = runnable_lenses(bench.to_str().unwrap(), "criterion_main!(benches);\n", true, false, &bench_hints).iter().map(summary).collect();
    assert_eq!(got, [("▶︎ Run ".into(), 0, "run speed".into(), ["bench", "--package", "demo-pkg", "--bench", "speed"].map(String::from).to_vec())]);
}

#[test]
fn experimental_runnables_list_the_file_and_its_package() {
    let dir = package(&[("src/main.rs", "fn main() {}\n#[test]\nfn works() {}\n")]);
    let file = dir.path().join("src/main.rs");
    let uri = crate::lsp::protocol::path_to_uri(file.to_str().unwrap());
    let ov = crate::overlay::OverlayManager::new();
    let ask = |position: Value| {
        crate::lsp::presentation::runnables::handle_runnables(&serde_json::json!({ "params": { "textDocument": { "uri": uri }, "position": position } }), None, &ov)
    };
    let all = ask(Value::Null);
    let labels: Vec<&str> = all.as_array().unwrap().iter().map(|r| r["label"].as_str().unwrap()).collect();
    assert_eq!(labels, ["test-mod ", "run demo-pkg", "test works", "cargo check -p demo-pkg --all-targets", "cargo run -p demo-pkg", "cargo test -p demo-pkg --all-targets"]);
    assert_eq!(all[2]["args"]["executableArgs"], serde_json::json!(["works", "--exact", "--nocapture"]));
    assert!(all[3]["location"].is_null(), "a package runnable has no location");
    let at_test = ask(serde_json::json!({ "line": 2, "character": 4 }));
    let labels: Vec<&str> = at_test.as_array().unwrap().iter().map(|r| r["label"].as_str().unwrap()).collect();
    assert_eq!(labels, ["test-mod ", "test works"], "the file module spans the whole file");
}
