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
use crate::lsp::intelligence::completion_edits::mark_decl;
use crate::lsp::intelligence::completion_edits::tests::{accept, run, with_snippets};
use std::path::Path;

const DOC: &str = "struct S;\nimpl S {\n    pub fn push_str(&mut self, string: &str) {}\n    pub fn with(&self, _n: usize) -> Self { S }\n}\nfn f<T>(_: std::collections::HashMap<u8, u8>, (a, b): (u8, u8), _: T, _: &S, _: HTTPServer) {}\n";

/// An item for the function declared on `line` of [`DOC`].
fn declared(label: &str, kind: u32, detail: &str, line: u32) -> Value {
    let mut it = json!({ "label": label, "kind": kind, "detail": detail });
    mark_decl(&mut it, Path::new("/w/src/lib.rs"), line);
    it
}

fn set(options: Value, commands: &[&str]) {
    set_client(&json!({
        "capabilities": { "textDocument": { "completion": { "completionItem": { "snippetSupport": true } } }, "experimental": { "commands": { "commands": commands } } },
        "initializationOptions": options,
    }));
}

#[test]
fn test_arguments_are_named_placeholders() {
    with_snippets(true);
    let items = run("    s.pu|", json!([declared("push_str", 2, "fn(&mut self, &str)", 3), declared("with", 2, "fn(&self, usize) -> S", 4)]), Some(DOC));
    assert_eq!(items[0]["textEdit"]["newText"], "push_str(${1:string})$0");
    assert_eq!(accept("    s.pu|", &items[0]), "    s.push_str(${1:string})$0");
    assert_eq!(items[1]["textEdit"]["newText"], "with(${1:n})$0", "a leading `_` is dropped");
    assert!(items.iter().all(|it| it["data"]["ridxDecl"]["line"].is_u64()), "the declaration stays marked for completionItem/resolve");
    assert!(items[0].get("command").is_none(), "the client did not announce rust-analyzer's commands");
}

#[test]
fn test_path_calls_take_the_receiver_first() {
    with_snippets(true);
    let items = run("    let x = S::wi|", json!([declared("with", 2, "fn(&self, usize) -> S", 4)]), Some(DOC));
    assert_eq!(items[0]["textEdit"]["newText"], "with(${1:&self}, ${2:n})$0");
}

#[test]
fn test_pattern_parameters_are_named_by_their_type() {
    with_snippets(true);
    let items = run("    |", json!([declared("f", 3, "fn(HashMap<u8, u8>, (u8, u8), T, &S, HTTPServer)", 6)]), Some(DOC));
    assert_eq!(items[0]["textEdit"]["newText"], "f(${1:hash_map}, ${2:_}, ${3:_}, ${4:_}, ${5:http_server})$0");
}

#[test]
fn test_a_declaration_that_does_not_match_the_signature_is_not_used() {
    with_snippets(true);
    let items = run("    s.pu|", json!([declared("push_str", 2, "fn(&mut self, &str, u8)", 3), json!({ "label": "push", "kind": 2, "detail": "fn(&mut self, T)" })]), Some(DOC));
    assert_eq!(items[0]["textEdit"]["newText"], "push_str($0)");
    assert_eq!(items[1]["textEdit"]["newText"], "push($0)", "no declaration found");
}

#[test]
fn test_callable_snippets_option_and_parameter_hints_command() {
    let item = || json!([declared("push_str", 2, "fn(&mut self, &str)", 3), json!({ "label": "len", "kind": 2, "detail": "fn(&self) -> usize" })]);
    set(json!({ "completion": { "callable": { "snippets": "add_parentheses" } } }), &["rust-analyzer.triggerParameterHints"]);
    let items = run("    s.pu|", item(), Some(DOC));
    assert_eq!(items[0]["textEdit"]["newText"], "push_str($0)");
    assert_eq!(items[0]["command"]["command"], "rust-analyzer.triggerParameterHints");
    assert_eq!(items[1]["textEdit"]["newText"], "len()$0");
    assert!(items[1].get("command").is_none(), "nothing to hint for a call taking nothing");
    set(json!({ "completion": { "callable": { "snippets": "none" } } }), &[]);
    let items = run("    s.pu|", item(), Some(DOC));
    assert!(items.iter().all(|it| it.get("textEdit").is_none()), "{items:?}");
    set(json!({}), &[]);
    assert_eq!(client().callable, Callable::FillArguments);
}

#[test]
fn test_snake_case_type_names() {
    assert_eq!(to_lower_snake_case("HashMap"), "hash_map");
    assert_eq!(to_lower_snake_case("HTTPServer"), "http_server");
    assert_eq!(to_lower_snake_case("Vec2D"), "vec2_d");
    assert_eq!(adt_name("&Foo", &[]), None);
    assert_eq!(adt_name("T", &["T".into()]), None);
    assert_eq!(adt_name("u8", &[]), None);
    assert_eq!(adt_name("std::path::PathBuf", &[]).as_deref(), Some("PathBuf"));
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
