/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use super::*;

fn crate_root(first: &str) -> bool {
    matches!(first, "std" | "core" | "alloc" | "crate")
}

/// `doc` (cursor at `|`) once the assist's edits are applied, or `None` when it is not offered.
fn run(doc: &str) -> Option<String> {
    let at = doc.find('|').unwrap();
    let text = doc.replace('|', "");
    let line0 = text[..at].matches('\n').count() as u32;
    let col = text[text[..at].rfind('\n').map_or(0, |i| i + 1)..at].encode_utf16().count() as u32;
    let mut edits = qualified_use_edits(&text, line0, col, &crate_root)?;
    let starts = line_starts_of(&text);
    let byte = |p: &Value| starts[p["line"].as_u64().unwrap() as usize] + p["character"].as_u64().unwrap() as usize;
    edits.sort_by_key(|e| std::cmp::Reverse(byte(&e["range"]["start"])));
    let mut out = text.clone();
    for e in edits {
        out.replace_range(byte(&e["range"]["start"])..byte(&e["range"]["end"]), e["newText"].as_str().unwrap());
    }
    Some(out)
}

const DOC: &str = "use std::path::Path;\n\nfn f(p: &Path) -> Option<String> {\n    let m = std::fs::metadata(p).ok()?;\n    let t = std::fs::read_to_string(p).ok()?;\n    let _ = m;\n    Some(t)\n}\n";

#[test]
fn test_first_segment_imports_the_whole_path() {
    let doc = DOC.replace("std::fs::read_to_string", "s|td::fs::read_to_string");
    let out = run(&doc).unwrap();
    assert!(out.contains("use std::fs::read_to_string;\n"), "{out}");
    assert!(out.contains("let t = read_to_string(p)"), "{out}");
    assert!(out.contains("std::fs::metadata(p)"), "another path keeps its qualifier: {out}");
}

#[test]
fn test_a_later_segment_imports_through_it_and_rewrites_every_use() {
    let doc = DOC.replace("std::fs::read_to_string", "std::f|s::read_to_string");
    let out = run(&doc).unwrap();
    assert!(out.contains("use std::fs;\n"), "{out}");
    assert!(out.contains("let m = fs::metadata(p)") && out.contains("let t = fs::read_to_string(p)"), "{out}");
}

#[test]
fn test_not_offered_where_the_result_may_not_compile() {
    // An associated item, a macro, a path not from a crate root, a use item.
    assert_eq!(run("fn f() -> String {\n    std::string::String::ne|w()\n}\n"), None);
    assert_eq!(run("fn f() {\n    std::print|ln!(\"x\");\n}\n"), None);
    assert_eq!(run("fn f() {\n    util::lo|ad();\n}\n"), None);
    assert_eq!(run("use std::f|s::read;\n"), None);
    assert_eq!(run("fn f() {\n    std::mem::size_of|::<u8>();\n}\n"), None);
    // The name already means something here.
    let own = format!("{DOC}fn read_to_string() {{}}\n").replace("std::fs::read_to_string", "s|td::fs::read_to_string");
    assert_eq!(run(&own), None);
    let local = DOC.replace("let m =", "let fs =").replace("let _ = m;", "let _ = fs;").replace("std::fs::read_to_string", "std::f|s::read_to_string");
    assert_eq!(run(&local), None);
}

#[test]
fn test_methods_of_the_same_name_do_not_block_it() {
    let doc = "fn f(mut r: impl std::io::Read, p: &str) -> String {\n    let mut s = String::new();\n    r.read_to_string(&mut s).ok();\n    std::fs::read_to_st|ring(p).unwrap_or(s)\n}\n";
    let out = run(doc).unwrap();
    assert!(out.contains("use std::fs::read_to_string;\n") && out.contains("    read_to_string(p).unwrap_or(s)") && out.contains("r.read_to_string(&mut s)"), "{out}");
}
