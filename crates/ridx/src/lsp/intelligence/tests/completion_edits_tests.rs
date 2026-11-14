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

fn with_snippets(on: bool) {
    set_snippet_support(&json!({ "textDocument": { "completion": { "completionItem": { "snippetSupport": on } } } }));
}

/// Applies the edits for the cursor at `|` of `line` (line 3 of `doc`, or of the line alone) to `items`.
fn run(line: &str, items: Value, doc: Option<&str>) -> Vec<Value> {
    let cursor = line.find('|').unwrap();
    let text = line.replace('|', "");
    let (before, after) = text.split_at(cursor);
    let site = EditSite { before, after, line_0based: 3, col_utf16: before.encode_utf16().count() as u32, in_use_item: in_use_item(&text, cursor), doc_text: doc.unwrap_or(&text) };
    let mut response = json!({ "isIncomplete": false, "items": items });
    apply(&mut response, &site);
    response["items"].as_array().unwrap().clone()
}

/// The line `line` becomes once `item`'s edits are applied (all edits sit on that line).
fn accept(line: &str, item: &Value) -> String {
    let text: Vec<u16> = line.replace('|', "").encode_utf16().collect();
    let mut edits: Vec<&Value> = vec![&item["textEdit"]];
    edits.extend(item["additionalTextEdits"].as_array().into_iter().flatten().filter(|e| e["range"]["start"]["line"] == 3));
    edits.sort_by_key(|e| std::cmp::Reverse(e["range"]["start"]["character"].as_u64().unwrap()));
    let mut out = text;
    for e in edits {
        let (s, t) = (e["range"]["start"]["character"].as_u64().unwrap() as usize, e["range"]["end"]["character"].as_u64().unwrap() as usize);
        let new: Vec<u16> = e["newText"].as_str().unwrap().encode_utf16().collect();
        out.splice(s..t, new);
    }
    String::from_utf16(&out).unwrap()
}

#[test]
fn test_methods_insert_calls() {
    with_snippets(true);
    let items = run(
        "    s.as_r|",
        json!([
            { "label": "as_ref", "kind": 2, "detail": "for<'a> fn(&'a Self) -> &'a T", "filterText": "as_ref", "insertText": "as_ref", "labelDetails": { "detail": "(as AsRef)" } },
            { "label": "push_str", "kind": 2, "detail": "fn(&mut self, &str)" },
            { "label": "len", "kind": 2, "detail": "fn(&self) -> usize" },
            { "label": "and_then", "kind": 2, "detail": "fn(self, F) -> std::result::Result<U, E> (in core::result::Result::<T, E>::and_then)" },
            { "label": "capacity", "kind": 5, "detail": "usize" },
        ]),
        None,
    );
    assert_eq!(items[0]["textEdit"]["newText"], "as_ref()$0");
    assert_eq!(items[0]["insertTextFormat"], 2);
    assert!(items[0].get("insertText").is_none());
    assert_eq!(accept("    s.as_r|", &items[0]), "    s.as_ref()$0");
    assert_eq!(items[1]["textEdit"]["newText"], "push_str($0)");
    assert_eq!(items[2]["textEdit"]["newText"], "len()$0");
    assert_eq!(items[3]["textEdit"]["newText"], "and_then($0)");
    assert!(items[4].get("textEdit").is_none(), "a field is not called");
}

#[test]
fn test_function_paths_count_every_parameter() {
    with_snippets(true);
    let items = run(
        "    let p = Builder::ne|",
        json!([
            { "label": "new", "kind": 3, "detail": "pub fn new() -> Self" },
            { "label": "with", "kind": 2, "detail": "pub fn with(&self, n: usize) -> Self" },
            { "label": "from_iter", "kind": 3, "detail": "fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self" },
        ]),
        None,
    );
    assert_eq!(items[0]["textEdit"]["newText"], "new()$0");
    assert_eq!(items[1]["textEdit"]["newText"], "with($0)", "`Type::method` passes the receiver as an argument");
    assert_eq!(items[2]["textEdit"]["newText"], "from_iter($0)");
}

#[test]
fn test_no_parentheses_where_a_call_follows_or_in_use_items() {
    with_snippets(true);
    let items = run("    v.pu|(x)", json!([{ "label": "push", "kind": 2, "detail": "fn(&mut self, T)" }]), None);
    assert!(items[0].get("textEdit").is_none());
    let items = run("use crate::util::lo|", json!([{ "label": "load", "kind": 3, "detail": "pub fn load() -> u8" }]), None);
    assert!(items[0].get("textEdit").is_none());
    assert!(in_use_item("fn a() {}\npub(crate) use std::{\n    fs::read, io::", 41));
    assert!(!in_use_item("fn a() {\n    let x = 1;\n    re", 30));
}

#[test]
fn test_without_snippet_support_only_empty_calls_get_parentheses() {
    with_snippets(false);
    let items = run("    s.l|", json!([{ "label": "len", "kind": 2, "detail": "fn(&self) -> usize" }, { "label": "push", "kind": 2, "detail": "fn(&mut self, T)" }]), None);
    assert_eq!(items[0]["textEdit"]["newText"], "len()");
    assert!(items[0].get("insertTextFormat").is_none());
    assert!(items[1].get("textEdit").is_none());
    with_snippets(true);
}

#[test]
fn test_postfix_snippets_rewrite_the_receiver() {
    with_snippets(true);
    let line = "    let r = conf.symbol.replace(\"-\", \"\").db|";
    let items = run(
        line,
        json!([
            { "label": "dbg", "kind": 15, "detail": "dbg!(expr)" },
            { "label": "ok", "kind": 15, "detail": "Ok(expr)" },
            { "label": "ref", "kind": 15, "detail": "&expr" },
        ]),
        None,
    );
    assert_eq!(accept(line, &items[0]), "    let r = dbg!(conf.symbol.replace(\"-\", \"\"))");
    assert_eq!(accept(line, &items[1]), "    let r = Ok(conf.symbol.replace(\"-\", \"\"))");
    assert_eq!(accept(line, &items[2]), "    let r = &conf.symbol.replace(\"-\", \"\")");
}

#[test]
fn test_postfix_blocks_and_bindings() {
    with_snippets(true);
    let line = "        opt.if|";
    let items = run(
        line,
        json!([
            { "label": "ifl", "kind": 15, "detail": "if let Some {}" },
            { "label": "let", "kind": 15, "detail": "let" },
            { "label": "match", "kind": 15, "detail": "match expr {}" },
        ]),
        None,
    );
    assert_eq!(accept(line, &items[0]), "        if let Some(${1:_}) = opt {\n            $0\n        }");
    assert_eq!(accept(line, &items[1]), "        let $1 = opt;$0");
    assert_eq!(accept(line, &items[2]), "        match opt {\n            ${1:_} => {$0}\n        }");
    with_snippets(false);
    let items = run(line, json!([{ "label": "ifl", "kind": 15, "detail": "if let Ok {}" }]), None);
    assert_eq!(accept(line, &items[0]), "        if let Ok(_) = opt {\n            \n        }");
    with_snippets(true);
}

#[test]
fn test_arc_postfix_imports_arc() {
    with_snippets(true);
    let doc = "use std::fmt;\n\nfn f() {\n    state.ar\n}\n";
    let items = run("    state.ar|", json!([{ "label": "arc", "kind": 15, "detail": "Arc::new(expr)" }]), Some(doc));
    assert_eq!(accept("    state.ar|", &items[0]), "    Arc::new(state)");
    let edits = items[0]["additionalTextEdits"].as_array().unwrap();
    assert!(edits.iter().any(|e| e["newText"] == "use std::sync::Arc;\n"), "{edits:?}");
}

#[test]
fn test_receiver_scan() {
    assert_eq!(receiver_before("    a.b().c[0].", 0), Some(Receiver { start_byte: 4, dot_byte: 14 }));
    assert_eq!(receiver_before("    x?.", 0), Some(Receiver { start_byte: 4, dot_byte: 6 }));
    assert_eq!(receiver_before("    vec![1, 2].it", 2), Some(Receiver { start_byte: 4, dot_byte: 14 }));
    assert_eq!(receiver_before("    Foo::bar(1).", 0), Some(Receiver { start_byte: 4, dot_byte: 15 }));
    assert_eq!(receiver_before("    0..", 0), None);
    assert_eq!(receiver_before("    .", 0), None);
    assert_eq!(receiver_before("    \"абв\".", 0), Some(Receiver { start_byte: 4, dot_byte: 12 }));
}

#[test]
fn test_call_arg_count() {
    assert_eq!(call_arg_count("fn(&self) -> usize", true), Some(0));
    assert_eq!(call_arg_count("fn(&mut self, f: impl Fn(u8, u8) -> u8)", true), Some(1));
    assert_eq!(call_arg_count("for<'a> fn(&'a Self) -> &'a T", true), Some(0));
    assert_eq!(call_arg_count("fn(self: Pin<&mut Self>, cx: &mut Context<'_>)", true), Some(1));
    assert_eq!(call_arg_count("pub fn new(a: HashMap<K, V>, b: [u8; 2]) -> Self", false), Some(2));
    assert_eq!(call_arg_count("pub const fn len(&self) -> usize", false), Some(1));
    assert_eq!(call_arg_count("usize", true), None);
    assert_eq!(call_arg_count("fnord", true), None);
}
