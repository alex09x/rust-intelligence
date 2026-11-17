/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Found with rust-analyzer's goto-definition fixtures run against ridx on real Cargo workspaces
//! (`bench/ra_fixtures.py`): these spots answered nothing, or what a `for` loop desugars to.

use super::*;

/// The target of a definition asked at the `$0` of `marked` (removed), as the text it covers and its line.
fn at(marked: &str) -> Option<String> {
    let offset = marked.find("$0").unwrap();
    let text = marked.replacen("$0", "", 1);
    match definition(Path::new("/x/lib.rs"), &text, offset)? {
        Target::Here(r) => Some(format!("{}@{}", &text[r.clone()], position(&text, r.start).0)),
        Target::File(p) => Some(format!("file {}", p.display())),
        Target::Macro { name, use_sites, qualifier } => Some(format!("macro {name} {}{}", use_sites.len(), qualifier.map(|q| format!(" in {q}")).unwrap_or_default())),
        Target::Member { anchor, member, .. } => Some(format!("member {member} of {}", text[anchor..].split(|c: char| !c.is_alphanumeric()).next().unwrap())),
        Target::Nothing => Some("nothing".into()),
    }
}

#[test]
fn test_control_flow_keywords_go_to_their_construct() {
    let f = "fn outer() -> Option<u8> {\n    for i in 0..3 {\n        if i == 1 { break; }\n        continue;\n    }\n    let v = Some(1)?;\n    return v;\n}\n";
    let mark = |needle: &str, k: usize| {
        let i = f.find(needle).unwrap() + k;
        format!("{}$0{}", &f[..i], &f[i..])
    };
    assert_eq!(at(&mark("break", 4)).as_deref(), Some("for@1"), "break: its loop, not what `for` desugars to");
    assert_eq!(at(&mark("continue", 8)).as_deref(), Some("for@1"), "a caret just past `continue`");
    assert_eq!(at(&mark("?;", 0)).as_deref(), Some("fn@0"), "`?` returns from the function");
    assert_eq!(at(&mark("return", 2)).as_deref(), Some("fn@0"));
    let f = "fn outer() {\n    let c = || { re$0turn 1; };\n    fn inner() { return; }\n}\n";
    assert_eq!(at(f).as_deref(), Some("|@1"), "from a closure, its closure");
    let f = "fn f() {\n    'outer: loop {\n        while true { break$0 'outer; }\n    }\n    'b: { break 'b; }\n}\n";
    assert_eq!(at(f).as_deref(), Some("loop@1"), "a labelled break: the labelled loop");
    assert_eq!(at(&f.replace("break$0 'outer", "break 'outer").replace("'b; }", "'b$0; }")).as_deref(), Some("'b@4"), "a labelled block: its label");
    let f = "fn f(x: u8) -> u8 {\n    if x == 0 { 1 } else i$0f x == 1 { 2 } else { 3 }\n}\n";
    assert_eq!(at(f).as_deref(), Some("if@1"), "`else if`: the chain's first `if`");
    assert_eq!(at("fn f() { match 1 { _ =$0> {} } }\n").as_deref(), Some("=>@0"));
    assert_eq!(at("fn f() { le$0t x = 1; }\n").as_deref(), Some("nothing"), "a keyword with nothing to go to: not the index");
}

#[test]
fn test_lifetimes_labels_and_self_param() {
    assert_eq!(at("fn f<'long>(a: &'lo$0ng str) -> &'long str { a }\n").as_deref(), Some("'long@0"));
    assert_eq!(at("struct S;\nimpl S {\n    fn get(&se$0lf) {}\n}\n").as_deref(), Some("self@2"));
    assert_eq!(at("fn f() {\n    'scan: for _ in 0..1 { continue 'sc$0an; }\n}\n").as_deref(), Some("'scan@1"), "on the label: the label");
    assert_eq!(at("fn f() {\n    'scan: for _ in 0..1 { conti$0nue 'scan; }\n}\n").as_deref(), Some("for@1"), "on the keyword: the loop");
    let m = "macro_rules! again { ($s:stmt) => { $s } }\nfn f() {\n    'next: loop { again!(continue 'ne$0xt); }\n}\n";
    assert_eq!(at(m).as_deref(), Some("'next@2"), "a label inside a macro's arguments");
}

#[test]
fn test_self_and_the_items_of_a_trait_impl() {
    // `Self` is the index's first (it records the type itself): the syntax answers only when the index has none.
    assert_eq!(at("struct Grid<T>(T);\nimpl<T> Grid<T> {\n    fn new(t: T) -> Se$0lf { Grid(t) }\n}\n"), None, "review of 2bca7861: not before the index");
    let self_at = |m: &str| {
        let t = m.replacen("$0", "", 1);
        let (l, c) = position(&t, m.find("$0").unwrap());
        self_type("/x/lib.rs", &t, l, c).map(|v| {
            let s = &v["range"]["start"];
            let line = t.lines().nth(s["line"].as_u64().unwrap() as usize).unwrap();
            let c = s["character"].as_u64().unwrap() as usize;
            format!("{}@{}", line[c..].split(|ch: char| !ch.is_alphanumeric()).next().unwrap(), s["line"])
        })
    };
    assert_eq!(self_at("struct Grid<T>(T);\nimpl<T> Grid<T> {\n    fn new(t: T) -> Se$0lf { Grid(t) }\n}\n").as_deref(), Some("Grid@1"), "the impl's type");
    assert_eq!(self_at("trait Make { fn make() -> Self; }\nimpl Make for crate::Grid {\n    fn make() -> Self$0 { todo!() }\n}\n").as_deref(), Some("Grid@1"), "not the trait");
    assert_eq!(self_at("trait Make { fn make() -> Se$0lf; }\n").as_deref(), Some("Make@0"));
    assert_eq!(self_at("impl<T: B> Make for <T as B>::Out {\n    fn make() -> Se$0lf {}\n}\n").as_deref(), Some("Out@0"), "a qualified path's last segment");
    assert_eq!(self_at("impl<const N: usize> Make for [u8; N] {\n    fn make() -> Se$0lf {}\n}\n"), None, "an array type has no name: not `N`");
    assert_eq!(at("impl Make for Grid {\n    const SI$0ZE: u8 = 1;\n}\n").as_deref(), Some("member SIZE of Make"), "the trait's declaration");
    assert_eq!(at("impl Grid {\n    fn ma$0ke() {}\n}\n"), None, "an inherent item is its own definition");
}

#[test]
fn test_a_shorthand_field_names_its_record() {
    let s = |m: &str| {
        shorthand(&m.replacen("$0", "", 1), m.find("$0").unwrap()).map(|(a, f)| (m.replacen("$0", "", 1)[a..].split(|c: char| !c.is_alphanumeric()).next().unwrap().to_string(), f))
    };
    assert_eq!(s("fn f(x: u8) { let _ = m::Pt::<u8> { x$0 }; }"), Some(("Pt".into(), "x".into())));
    assert_eq!(s("fn f(p: E) { if let E::Move { dx$0, .. } = p {} }"), Some(("Move".into(), "dx".into())));
    assert_eq!(s("fn f(x: u8) { let _ = Pt { x$0: x }; }"), None, "`field: value` is no shorthand");
}

#[test]
fn test_a_macro_call_goes_to_its_macro_rules() {
    let f = "macro_rules! twice { ($e:expr) => { $e * 2 } }\nfn f() -> u8 {\n    macro_rules! twice { () => { 4 } }\n    twi$0ce!()\n}\n";
    assert_eq!(at(f).as_deref(), Some("twice@2"), "the nearest one above the call");
    assert_eq!(at("use dep::shout;\nfn f() { sho$0ut!(); }\n").as_deref(), Some("macro shout 1"), "imported: the index resolves the `use`");
    assert_eq!(at("fn f() { dep::m::sho$0ut!(); }\nmacro_rules! shout { () => {} }\n").as_deref(), Some("macro shout 0 in dep::m"), "a path: not the local one");
    assert_eq!(at("#[macro_use(sho$0ut)]\nextern crate dep;\n").as_deref(), Some("macro shout 0"));
    assert_eq!(at("fn f() { twi$0ce(); }\n"), None, "a function call: the index decides");
    // Review of 2bca7861.
    let f = "use dep::foo;\nfn caller() { fo$0o!(); }\nmod private { macro_rules! foo { () => {} } }\n";
    assert_eq!(at(f).as_deref(), Some("macro foo 1"), "one in a sibling `mod` is out of scope");
    assert_eq!(at("fn f() { fo$0o!(); }\nmacro_rules! foo { () => {} }\n").as_deref(), Some("macro foo 0"), "one below the call is not in textual scope");
    assert_eq!(at("#[macro_u$0se(shout)]\nextern crate dep;\n"), None, "`macro_use` itself");
    assert_eq!(at("#[cfg_attr(un$0ix, macro_use)]\nextern crate dep;\n"), None, "another word of the attribute");
}

#[test]
fn test_a_module_declaration_goes_to_its_file() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir_all(src.join("net/proto")).unwrap();
    std::fs::write(src.join("net.rs"), "mod proto;\n").unwrap();
    std::fs::write(src.join("net/proto/mod.rs"), "").unwrap();
    std::fs::write(src.join("util.rs"), "").unwrap();
    let lib = src.join("lib.rs");
    let text = "mod net;\npub mod util;\n";
    assert_eq!(definition(&lib, text, text.find("util").unwrap() + 4), Some(Target::File(src.join("util.rs"))), "a caret just past the name");
    assert_eq!(definition(&src.join("net.rs"), "mod proto;\n", 5), Some(Target::File(src.join("net/proto/mod.rs"))), "below a non-root file");
    assert_eq!(module_file(&lib, "absent"), None);
}

#[test]
fn test_lsp_positions_round_trip() {
    let t = "a\n€é x\n";
    assert_eq!(byte_offset(t, 1, 3), Some(t.find('x').unwrap()), "`€` and `é` are one UTF-16 unit each");
    assert_eq!(position(t, t.find('x').unwrap()), (1, 3));
    assert_eq!(byte_offset(t, 1, 99), Some(t.len() - 1), "past the line's end: its end");
}

/// In Zed on BTCR `println!` went to `anstream`'s `println!`, a dependency's macro the code never imports.
#[test]
fn test_an_unimported_macro_is_the_standard_librarys_before_a_dependencys() {
    let mut crates = [("anstream", false), ("std", false), ("btcr", true), ("core", false)];
    crates.sort_by_key(|&(c, own)| macro_rank(own, c));
    assert_eq!(crates.iter().map(|c| c.0).collect::<Vec<_>>(), ["btcr", "std", "core", "anstream"]);
}

/// In Zed on BTCR `include!` went to `BTCR/library/core/src/macros/mod.rs`: a standard library symbol is recorded
/// relative to the sysroot's sources, and the symbol search joined it to the workspace's root.
#[test]
fn test_a_standard_library_symbol_is_in_the_sysroot() {
    let sym = |file: &'static str, root: &'static Path| ridx_format::workspace::WorkspaceSymbol {
        crate_name: "core",
        name: "include",
        path: "core::include",
        kind: ridx_format::DefKind::Macro,
        file,
        span: Default::default(),
        signature: None,
        is_macro_expansion: false,
        source_root: Some(root),
        unit_key: None,
    };
    let ws_root = Path::new("/work/BTCR");
    let std_file = symbol_file(&sym("library/core/src/macros/mod.rs", ws_root));
    assert!(!std_file.starts_with(ws_root), "{std_file:?}");
    assert!(std_file.ends_with("library/core/src/macros/mod.rs"));
    assert_eq!(symbol_file(&sym("src/lib.rs", ws_root)), ws_root.join("src/lib.rs"), "the workspace's own");
}
