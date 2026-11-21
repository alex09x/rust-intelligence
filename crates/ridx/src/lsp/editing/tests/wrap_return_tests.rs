/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! `Wrap return type in Option` / `in Result`: where offered, what it writes, and that the result compiles.

use super::*;
use crate::lsp::editing::tests::code_action_assists_tests::assert_rustc_compiles;

/// `doc` with the wrap at the first `$0` applied, nothing calling it from another file, in a 2018 package.
fn wrapped(doc: &str, wrap: Wrap) -> Option<String> {
    let offset = doc.find("$0").unwrap();
    let doc = doc.replacen("$0", "", 1);
    let mut out = doc.clone();
    for (r, text) in wrap_edits(&doc, offset, wrap, true, &|_, _| false)?.into_iter().rev() {
        out.replace_range(r, &text);
    }
    Some(out)
}

/// prod-code, m=20226: rust-analyzer offers both in `pub fn twice(x: u32) -> u32`.
#[test]
fn test_the_return_type_and_every_returned_value_are_wrapped() {
    let doc = "pub fn twice(x: u32) -> u$032 {\n    x * 2\n}\n";
    let got = wrapped(doc, Wrap::Option).unwrap();
    assert_eq!(got, "pub fn twice(x: u32) -> ::core::option::Option<u32> {\n    ::core::option::Option::Some(x * 2)\n}\n");
    assert_rustc_compiles(&got, "Wrap return type in Option");
    let got = wrapped(doc, Wrap::Result).unwrap();
    assert_eq!(got, "pub fn twice(x: u32) -> ::core::result::Result<u32, ()> {\n    ::core::result::Result::Ok(x * 2)\n}\n");
    assert_rustc_compiles(&got, "Wrap return type in Result");

    let doc = "pub fn $0pick(a: bool) -> &'static str {\n    if a {\n        return \"a\";\n    }\n    match a {\n        true => \"t\",\n        false => \"f\",\n    }\n}\n";
    let got = wrapped(doc, Wrap::Option).unwrap();
    assert!(got.contains("return ::core::option::Option::Some(\"a\");") && got.contains("::core::option::Option::Some(match a {"), "{got}");
    assert_rustc_compiles(&got, "Wrap return type in Option");

    let doc = "pub struct S;\nimpl S {\n    pub fn $0get(&self) -> u8 {\n        let _ = self;\n        1\n    }\n}\n";
    assert_rustc_compiles(&wrapped(doc, Wrap::Result).unwrap(), "Wrap return type in Result");
    // Review of 36349abf: an `Option` of the user's, a glob import, no prelude: the paths are the `core` crate's.
    let doc = "pub struct Option<T>(pub T);\nmod p {\n    pub type Result<T> = core::result::Result<T, ()>;\n}\nuse p::*;\npub fn $0f() -> u8 {\n    1\n}\n";
    assert_rustc_compiles(&wrapped(doc, Wrap::Option).unwrap(), "Wrap return type in Option");
    assert_rustc_compiles(&wrapped(doc, Wrap::Result).unwrap(), "Wrap return type in Result");
    assert_rustc_compiles(&wrapped("#![no_implicit_prelude]\npub fn $0f() -> u8 {\n    1\n}\n", Wrap::Option).unwrap(), "Wrap return type in Option");
}

#[test]
fn test_it_is_not_offered_where_the_result_would_not_compile() {
    for doc in [
        // A caller.
        "pub fn $0twice(x: u32) -> u32 {\n    x * 2\n}\npub fn four() -> u32 {\n    twice(2)\n}\n",
        // A signature a trait fixes.
        "pub trait T {\n    fn $0f(&self) -> u32;\n}\npub struct S;\nimpl T for S {\n    fn f(&self) -> u32 {\n        1\n    }\n}\n",
        "pub trait T {\n    fn f(&self) -> u32;\n}\npub struct S;\nimpl T for S {\n    fn $0f(&self) -> u32 {\n        1\n    }\n}\n",
        // Already wrapped, no return type, never returns, inside the body.
        "pub fn $0f() -> Option<u32> {\n    None\n}\n",
        "pub fn $0f() {\n}\n",
        "pub fn $0f() -> ! {\n    panic!()\n}\n",
        "pub fn f() -> u32 {\n    $01\n}\n",
        // Review of 63f24322: a caller in raw spelling, a fall-through, `?` through an alias, a return in a macro's
        // arguments, a user's own `Option`.
        "pub fn $0foo() -> u8 {\n    1\n}\npub fn caller() -> u8 {\n    r#foo()\n}\n",
        "pub fn $0f() -> () {\n    let _ = 1;\n}\n",
        "pub type R = Result<u8, ()>;\npub fn $0f(r: R) -> R {\n    let v = r?;\n    Ok(v)\n}\n",
        "pub fn $0f(c: bool) -> u8 {\n    println!(\"{}\", if c { return 1 } else { 2 });\n    3\n}\n",
        // Review of 36349abf: a unit alias falling through, a macro whose expansion returns, a name beyond ASCII.
        "pub type Unit = ();\npub fn $0f() -> Unit {\n    let _ = 1;\n}\n",
        "macro_rules! early {\n    () => {\n        return 1\n    };\n}\npub fn $0f() -> u8 {\n    early!();\n    2\n}\n",
        "pub fn $0ca\u{0301}fe() -> u8 {\n    1\n}\n",
        // Review of 03933e37: any macro (the file's own `println` may return), a crate named in the extern prelude
        // (`r#core` too), `main`, an attribute.
        "macro_rules! println {\n    ($e:expr) => {\n        return $e\n    };\n}\npub fn $0f() -> u8 {\n    println!(1);\n    2\n}\n",
        "extern crate self as r#core;\npub fn $0f() -> u8 {\n    1\n}\n",
        "fn $0main() -> std::process::ExitCode {\n    std::process::ExitCode::SUCCESS\n}\n",
        "#[inline]\npub fn $0f() -> u8 {\n    1\n}\n",
    ] {
        assert_eq!(wrapped(doc, Wrap::Option), None, "{doc}");
    }
}

/// Review of 63f24322: a call typed in another open buffer, not saved, keeps the function's signature.
#[test]
fn test_a_caller_in_another_open_buffer_keeps_the_signature() {
    let dir = tempfile::tempdir().unwrap();
    let mut w = ridx_format::IndexWriter::new("w", "0.1.0");
    std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"w\"\nversion = \"0.1.0\"\nedition = \"2021\"\n").unwrap();
    let lib = dir.path().join("src/lib.rs");
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(&lib, "pub fn f() -> u8 {\n    1\n}\n").unwrap();
    w.add_definition(ridx_format::RawDefinition {
        def_id: 1,
        kind: ridx_format::DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        file: lib.to_string_lossy().to_string(),
        name: "f".to_string(),
        start_line: 1,
        start_col: 8,
        end_line: 1,
        end_col: 9,
        path: "f".to_string(),
        signature: None,
    });
    let shard = dir.path().join("w.ridx");
    w.write_to(&mut std::fs::File::create(&shard).unwrap()).unwrap();
    let ws = ridx_format::WorkspaceIndex::open(&shard).unwrap();
    let doc = "pub fn f() -> u8 {\n    1\n}\n";
    let uri = format!("file://{}", lib.display());
    let range = json!({ "start": { "line": 0, "character": 14 }, "end": { "line": 0, "character": 14 } });
    let mut ov = crate::overlay::OverlayManager::new();
    assert_eq!(wrap_return_actions(doc, &uri, &range, None, &ws, &ov).len(), 2, "nothing calls it");
    let other = dir.path().join("src/g.rs");
    ov.open_document(&format!("file://{}", other.display()), &other.to_string_lossy(), 1, "pub fn g() -> u8 {\n    crate::f()\n}\n".to_string());
    assert!(wrap_return_actions(doc, &uri, &range, None, &ws, &ov).is_empty(), "a call not saved yet");
}

/// `::core` names the crate from the 2018 edition on: a 2015 package, or one whose edition is unknown, is not wrapped.
/// The edition is Cargo's (review of 03933e37: an `edition` in a metadata table, a workspace's inherited 2015).
#[test]
fn test_the_edition_is_read_from_the_manifest() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("src/lib.rs");
    assert!(!edition_2018(&file), "no manifest");
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(&file, "").unwrap();
    let manifest = dir.path().join("Cargo.toml");
    let package = "[package]\nname = \"w\"\nversion = \"0.1.0\"\n";
    for (text, yes) in [
        (String::new(), false),
        ("edition = \"2015\"\n".to_string(), false),
        ("edition = \"2021\" # now\n".to_string(), true),
        ("[package.metadata.x]\nedition = \"2021\"\n".to_string(), false),
        ("edition = \"2021\"\n[package.metadata.x]\nedition = \"2015\"\n".to_string(), true),
    ] {
        std::fs::write(&manifest, format!("{package}{text}")).unwrap();
        assert_eq!(edition_2018(&file), yes, "{text}");
    }
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("m/src")).unwrap();
    std::fs::write(root.path().join("m/src/lib.rs"), "").unwrap();
    std::fs::write(root.path().join("m/Cargo.toml"), "[package]\nname = \"m\"\nversion = \"0.1.0\"\nedition.workspace = true\n").unwrap();
    for (edition, yes) in [("2015", false), ("2021", true)] {
        std::fs::write(root.path().join("Cargo.toml"), format!("[workspace]\nmembers = [\"m\"]\n[workspace.package]\nedition = \"{edition}\"\n")).unwrap();
        assert_eq!(edition_2018(&root.path().join("m/src/lib.rs")), yes, "workspace {edition}");
    }
    let doc = "pub fn $0f() -> u8 {\n    1\n}\n";
    assert_eq!(wrap_edits(&doc.replacen("$0", "", 1), doc.find("$0").unwrap(), Wrap::Option, false, &|_, _| false), None);
}

/// Review of b854363f: an `extern crate .. as core` in the crate root holds for a child module too; an edition a
/// workspace outside the package's directories gives is read again when that workspace changes.
#[test]
fn test_the_crate_root_and_a_workspace_elsewhere_are_read() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"w\"\nversion = \"0.1.0\"\nedition = \"2021\"\n").unwrap();
    std::fs::write(root.join("src/lib.rs"), "pub mod child;\n").unwrap();
    let child = root.join("src/child.rs");
    let doc = "pub fn f() -> u8 {\n    1\n}\n";
    std::fs::write(&child, doc).unwrap();
    let mut w = ridx_format::IndexWriter::new("w", "0.1.0");
    w.add_definition(ridx_format::RawDefinition {
        def_id: 1,
        kind: ridx_format::DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        file: child.to_string_lossy().to_string(),
        name: "f".to_string(),
        start_line: 1,
        start_col: 8,
        end_line: 1,
        end_col: 9,
        path: "child::f".to_string(),
        signature: None,
    });
    let shard = root.join("w.ridx");
    w.write_to(&mut std::fs::File::create(&shard).unwrap()).unwrap();
    let ws = ridx_format::WorkspaceIndex::open(&shard).unwrap();
    let uri = format!("file://{}", child.display());
    let range = json!({ "start": { "line": 0, "character": 14 }, "end": { "line": 0, "character": 14 } });
    let ov = crate::overlay::OverlayManager::new();
    assert_eq!(wrap_return_actions(doc, &uri, &range, None, &ws, &ov).len(), 2);
    std::fs::write(root.join("src/lib.rs"), "extern crate self as r#core;\npub mod child;\n").unwrap();
    assert!(wrap_return_actions(doc, &uri, &range, None, &ws, &ov).is_empty(), "the crate root's alias");

    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().canonicalize().unwrap();
    std::fs::create_dir_all(base.join("m/src")).unwrap();
    std::fs::create_dir_all(base.join("control")).unwrap();
    std::fs::write(base.join("m/src/lib.rs"), "").unwrap();
    std::fs::write(base.join("m/Cargo.toml"), "[package]\nname = \"m\"\nversion = \"0.1.0\"\nworkspace = \"../control\"\nedition.workspace = true\n").unwrap();
    let control = |edition: &str| format!("[workspace]\nmembers = [\"../m\"]\n[workspace.package]\nedition = \"{edition}\"\n");
    std::fs::write(base.join("control/Cargo.toml"), control("2021")).unwrap();
    let file = base.join("m/src/lib.rs");
    assert!(edition_2018(&file), "2021 from the workspace elsewhere");
    std::fs::write(base.join("control/Cargo.toml"), control("2015")).unwrap();
    assert!(!edition_2018(&file), "read again once that workspace changes");
}
