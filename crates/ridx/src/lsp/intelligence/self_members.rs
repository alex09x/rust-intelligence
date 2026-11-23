/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! The members of `self` read from the document's own text: the fields of `struct T { … }` and the methods taking
//! `self` of each `impl … T { … }`, `T` being the type of the `impl` around the cursor. Completion after `self.`
//! answered nothing in a file no index holds (a crate not indexed yet, a file of no module), and listed the methods
//! but no field in a file indexed with other text (prod-code, issue #3). They join the items the index gave.

use super::self_completion::{find_enclosing_scope, parse_header, EnclosingScope};
use ridx_syntax::Token;
use serde_json::{json, Value};

/// LSP `CompletionItemKind`s.
const METHOD: u8 = 2;
const FIELD: u8 = 5;

/// Adds to `response.items` the members of `self` the document declares that start with the name typed, when the
/// cursor (`before`, the line's text up to it, on the 0-based `line`) follows `self.`.
pub(crate) fn add_buffer_members(response: &mut Value, text: &str, line: u32, before: &str) {
    let typed_from = before.trim_end_matches(|c: char| c.is_alphanumeric() || c == '_').len();
    let Some(receiver) = before[..typed_from].strip_suffix('.') else { return };
    let Some(head) = receiver.strip_suffix("self") else { return };
    if head.ends_with(|c: char| c.is_alphanumeric() || c == '_' || c == '.' || c == ':') {
        return;
    }
    let Some(EnclosingScope::Impl { type_name, .. }) = find_enclosing_scope(text, line + 1, before) else { return };
    let ty = type_name.rsplit("::").next().unwrap_or(&type_name);
    let typed = &before[typed_from..];
    let known: Vec<String> = response["items"].as_array().into_iter().flatten().filter_map(|i| i["label"].as_str()).map(|l| l.split('(').next().unwrap_or(l).to_string()).collect();
    let mut added: Vec<Value> = members(text, ty)
        .into_iter()
        .filter(|(name, _)| name.starts_with(typed) && !known.contains(name))
        .map(|(name, kind)| json!({ "label": name, "kind": kind, "detail": ty }))
        .collect();
    if added.is_empty() {
        return;
    }
    match response["items"].as_array_mut() {
        Some(items) => items.append(&mut added),
        None => response["items"] = Value::Array(added),
    }
}

/// The fields and methods with a `self` receiver that `text` declares for the type named `ty`, in order, each once.
fn members(text: &str, ty: &str) -> Vec<(String, u8)> {
    let toks: Vec<Token> = ridx_syntax::lex(text).into_iter().filter(|t| !t.is_trivia()).collect();
    let at = |i: usize| toks.get(i).map_or("", |t| t.text(text));
    let mut out: Vec<(String, u8)> = Vec::new();
    let mut push = |name: &str, kind: u8| {
        if !out.iter().any(|(n, _)| n == name) {
            out.push((name.to_string(), kind));
        }
    };
    for i in 0..toks.len() {
        if at(i) == "struct" && at(i + 1) == ty {
            if let Some(open) = (i + 2..toks.len()).find(|&j| matches!(at(j), "{" | ";" | "(")).filter(|&j| at(j) == "{") {
                for name in fields(&toks, text, open) {
                    push(name, FIELD);
                }
            }
        } else if at(i) == "impl" {
            let Some(open) = (i + 1..toks.len()).find(|&j| matches!(at(j), "{" | ";")).filter(|&j| at(j) == "{") else { continue };
            let header = &text[toks[i].range().start as usize..toks[open].range().start as usize];
            if matches!(parse_header(header), EnclosingScope::Impl { type_name, .. } if type_name.rsplit("::").next() == Some(ty)) {
                for name in methods(&toks, text, open) {
                    push(name, METHOD);
                }
            }
        }
    }
    out
}

/// The tokens between `{` at `open` and its `}` (brackets left out), each with its depth inside it (0 for the body's
/// own tokens).
fn body(toks: &[Token], text: &str, open: usize) -> Vec<(usize, usize)> {
    let (mut depth, mut out) = (0usize, Vec::new());
    for (j, t) in toks.iter().enumerate().skip(open + 1) {
        match t.text(text) {
            "{" | "(" | "[" => depth += 1,
            "}" | ")" | "]" if depth == 0 => break,
            "}" | ")" | "]" => depth -= 1,
            _ => out.push((j, depth)),
        }
    }
    out
}

/// The names of a `{ … }` struct body's fields: a name followed by a single `:` at the body's own depth.
fn fields<'s>(toks: &[Token], text: &'s str, open: usize) -> Vec<&'s str> {
    let at = |i: usize| toks.get(i).map_or("", |t| t.text(text));
    let path_sep = |i: usize| at(i) == ":" && at(i + 1) == ":" && toks[i].range().end == toks[i + 1].range().start;
    let start = |i: usize| matches!(at(i - 1), "{" | "," | ")" | "]" | "pub");
    body(toks, text, open).into_iter().filter(|&(j, d)| d == 0 && at(j + 1) == ":" && !path_sep(j + 1) && start(j) && is_name(at(j))).map(|(j, _)| at(j)).collect()
}

/// The names of the functions of an `impl` body whose first parameter is `self` (`self`, `&self`, `&'a mut self`,
/// `mut self`, `self: Box<Self>`).
fn methods<'s>(toks: &[Token], text: &'s str, open: usize) -> Vec<&'s str> {
    let at = |i: usize| toks.get(i).map_or("", |t| t.text(text));
    let takes_self = |paren: usize| (paren + 1..toks.len()).take_while(|&k| !matches!(at(k), "," | ")")).any(|k| at(k) == "self");
    body(toks, text, open)
        .into_iter()
        .filter(|&(j, d)| d == 0 && at(j) == "fn" && is_name(at(j + 1)))
        .filter(|&(j, _)| (j + 2..toks.len()).find(|&k| at(k) == "(").is_some_and(takes_self))
        .map(|(j, _)| at(j + 1))
        .collect()
}

fn is_name(s: &str) -> bool {
    s.starts_with(|c: char| c.is_alphabetic() || c == '_') && s.chars().all(|c| c.is_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    const COUNTER: &str = "pub struct Counter<T> {\n    pub total: u32,\n    #[allow(dead_code)]\n    pub(crate) map: std::collections::HashMap<u8, fn(a: u8)>,\n}\n\nimpl<T> Counter<T> {\n    pub fn add(&mut self, amount: u32) -> u32 {\n        self.total += amount;\n        self.\n    }\n    fn new() -> Self { todo!() }\n}\n\nimpl<T> Clone for Counter<T> {\n    fn clone(&self) -> Self { todo!() }\n}\n\nstruct Other { total_other: u8 }\nimpl Other { fn other(&self) {} }\n";

    fn labels(text: &str, line: u32, before: &str, items: Value) -> Vec<String> {
        let mut response = json!({ "isIncomplete": false, "items": items });
        add_buffer_members(&mut response, text, line, before);
        response["items"].as_array().unwrap().iter().map(|i| i["label"].as_str().unwrap().to_string()).collect()
    }

    #[test]
    fn test_the_fields_and_methods_of_selfs_type_in_the_document() {
        assert_eq!(members(COUNTER, "Counter"), [("total".into(), FIELD), ("map".into(), FIELD), ("add".into(), METHOD), ("clone".into(), METHOD)]);
        assert_eq!(labels(COUNTER, 8, "        self.", json!([])), ["total", "map", "add", "clone"]);
        assert_eq!(labels(COUNTER, 8, "        self.to", json!([])), ["total"], "the name typed");
        assert_eq!(labels(COUNTER, 9, "        self.", json!([{ "label": "add(…)" }])), ["add(…)", "total", "map", "clone"], "once");
        assert_eq!(labels(COUNTER, 8, "        other.self.", json!([])), Vec::<String>::new(), "not `self` itself");
        assert_eq!(members(COUNTER, "Other"), [("total_other".into(), FIELD), ("other".into(), METHOD)]);
        assert_eq!(labels(COUNTER, 8, "        x.", json!([])), Vec::<String>::new(), "another receiver");
    }
}
