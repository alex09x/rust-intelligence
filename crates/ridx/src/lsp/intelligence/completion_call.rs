/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! A completed function or method inserted as a call exactly as rust-analyzer inserts it. Under its default
//! `completion.callable.snippets = "fill_arguments"`, every argument is a tab stop holding its parameter's name
//! (`push_str(${1:string})$0`), the receiver of a `Type::method` path call first (`with(${1:&self}, ${2:n})$0`);
//! `"add_parentheses"` gives `push_str($0)`, `"none"` no parentheses. A call taking nothing is `len()$0`. Without
//! snippet support nothing is added, and a client running rust-analyzer's commands gets its parameter hints shown.

use super::completion_edits::{is_ident, set_main_edit, EditSite, Word};
use super::fn_params::{FnParams, Param};
use serde_json::{json, Value};
use std::cell::Cell;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Callable {
    FillArguments,
    AddParentheses,
    None,
}

/// What the client declared at initialize that decides the edits.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Client {
    pub snippets: bool,
    pub trigger_parameter_hints: bool,
    pub callable: Callable,
}

thread_local! {
    // The server loop answers every request on the thread that handled `initialize`.
    static CLIENT: Cell<Client> = const { Cell::new(Client { snippets: false, trigger_parameter_hints: false, callable: Callable::FillArguments }) };
}

/// Records the client's `snippetSupport`, its `rust-analyzer.triggerParameterHints` command and the
/// `completion.callable.snippets` initialization option.
pub fn set_client(params: &Value) {
    let caps = &params["capabilities"];
    let commands = caps["experimental"]["commands"]["commands"].as_array();
    let callable = match params["initializationOptions"]["completion"]["callable"]["snippets"].as_str() {
        Some("add_parentheses") => Callable::AddParentheses,
        Some("none") => Callable::None,
        _ => Callable::FillArguments,
    };
    let client = Client {
        snippets: caps["textDocument"]["completion"]["completionItem"]["snippetSupport"].as_bool().unwrap_or(false),
        trigger_parameter_hints: commands.is_some_and(|a| a.iter().any(|v| v.as_str() == Some("rust-analyzer.triggerParameterHints"))),
        callable,
    };
    CLIENT.with(|c| c.set(client));
}

pub(crate) fn client() -> Client {
    CLIENT.with(Cell::get)
}

/// Gives a function or method item its call: `name()$0`, or `name(` its argument placeholders `)$0`. `params` is
/// the declaration read from source, when it was found.
pub(super) fn call_edit(it: &mut Value, site: &EditSite<'_>, word: &Word, method_call: bool, params: Option<&FnParams>) {
    let client = client();
    if !client.snippets || client.callable == Callable::None {
        return;
    }
    let detail = it["detail"].as_str().unwrap_or_default();
    let Some(args) = call_arg_count(detail, method_call) else { return };
    let name = it["insertText"].as_str().or_else(|| it["label"].as_str()).unwrap_or_default().to_string();
    if name.is_empty() || !name.chars().all(|c| is_ident(c) || c == '.' || c == '(' || c == ')') {
        return;
    }
    let filled = if client.callable == Callable::FillArguments { params.and_then(|p| argument_snippet(p, method_call, args)) } else { None };
    let text = match (args, filled) {
        (0, _) => format!("{name}()$0"),
        (_, Some(args)) => format!("{name}({args})$0"),
        (_, None) => format!("{name}($0)"),
    };
    if args > 0 && client.trigger_parameter_hints {
        it["command"] = json!({ "title": "triggerParameterHints", "command": "rust-analyzer.triggerParameterHints" });
    }
    set_main_edit(it, site, word, text, true);
}

/// `${1:a}, ${2:b}` for the declaration's parameters, the receiver first unless it is the method call's receiver.
/// `None` when the declaration does not take the `args` arguments the signature counts: it is another function.
fn argument_snippet(fp: &FnParams, method_call: bool, args: usize) -> Option<String> {
    let skip_receiver = method_call && fp.params.first().is_some_and(|p| p.is_self);
    let shown: Vec<&Param> = fp.params.iter().skip(usize::from(skip_receiver)).collect();
    if shown.len() != args {
        return None;
    }
    let parts: Vec<String> = shown.iter().enumerate().map(|(i, p)| format!("${{{}:{}}}", i + 1, escape(&placeholder(p, &fp.generics)))).collect();
    Some(parts.join(", "))
}

/// rust-analyzer's placeholder for a parameter: the receiver as written, the bound name without leading `_`, or for
/// a pattern the name of its type in snake case (`_: HashMap<K, V>` → `hash_map`), `_` when the type is no ADT.
fn placeholder(p: &Param, generics: &[String]) -> String {
    if p.is_self {
        return p.pat.clone();
    }
    if let Some(name) = p.binding() {
        return name.trim_start_matches('_').to_string();
    }
    adt_name(&p.ty, generics).map_or_else(|| "_".to_string(), |n| to_lower_snake_case(&n))
}

/// The name of the ADT `ty` names (`std::collections::HashMap<K, V>` → `HashMap`): `None` for references, tuples,
/// slices, `impl`/`dyn` types, primitives and the function's own type parameters.
fn adt_name(ty: &str, generics: &[String]) -> Option<String> {
    let head = ty.split('<').next().unwrap_or(ty).trim();
    if head.is_empty() || !head.chars().all(|c| is_ident(c) || c == ':') || head.starts_with("impl") || head.starts_with("dyn") {
        return None;
    }
    let last = head.rsplit("::").next()?;
    const PRIMITIVES: [&str; 17] = ["bool", "char", "str", "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128", "isize", "f32", "f64"];
    let primitive = PRIMITIVES.contains(&last) || last == "Self";
    (!primitive && !generics.iter().any(|g| g == last) && last.starts_with(|c: char| c.is_uppercase())).then(|| last.to_string())
}

fn to_lower_snake_case(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    let mut out = String::with_capacity(name.len() + 4);
    for (i, &c) in chars.iter().enumerate() {
        if c.is_uppercase() && i > 0 {
            let prev = chars[i - 1];
            let next_lower = chars.get(i + 1).is_some_and(|n| n.is_lowercase());
            if prev.is_lowercase() || prev.is_ascii_digit() || (prev.is_uppercase() && next_lower) {
                out.push('_');
            }
        }
        out.extend(c.to_lowercase());
    }
    out
}

fn escape(text: &str) -> String {
    text.replace('\\', "\\\\").replace('$', "\\$").replace('}', "\\}")
}

/// How many arguments a call of the function `detail` describes takes, without the receiver of a method call.
/// `None` when `detail` is not a function signature.
pub(crate) fn call_arg_count(detail: &str, method_call: bool) -> Option<usize> {
    let open = params_open(detail, find_fn_keyword(detail)? + 2)?;
    let mut depth = 0i32;
    let mut params: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut prev = ' ';
    for c in detail[open + 1..].chars() {
        match c {
            '(' | '<' | '[' | '{' => depth += 1,
            ')' if depth == 0 => {
                if !cur.trim().is_empty() {
                    params.push(cur.trim().to_string());
                }
                let receiver = params.first().is_some_and(|p| is_receiver(p));
                return Some(params.len() - usize::from(method_call && receiver));
            }
            '>' if prev == '-' => {}
            ')' | '>' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => {
                params.push(std::mem::take(&mut cur).trim().to_string());
                prev = c;
                continue;
            }
            _ => {}
        }
        cur.push(c);
        prev = c;
    }
    None
}

/// The byte offset of the `(` opening the parameter list after `fn` (ending at `from`): past the name and its
/// generics, whose bounds may hold parentheses of their own (`<I: Iterator<Item = (K, V)>>`).
fn params_open(detail: &str, from: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut prev = ' ';
    for (i, c) in detail[from..].char_indices() {
        match c {
            '(' if depth == 0 => return Some(from + i),
            '<' => depth += 1,
            '>' if prev != '-' => depth -= 1,
            c if depth == 0 && !(is_ident(c) || c.is_whitespace() || c == '\'') => return None,
            _ => {}
        }
        prev = c;
    }
    None
}

/// The byte offset of the `fn` that starts the signature (`fn(`, `pub fn name(`, `for<'a> fn(`).
fn find_fn_keyword(detail: &str) -> Option<usize> {
    let bytes = detail.as_bytes();
    let mut from = 0;
    while let Some(i) = detail[from..].find("fn").map(|i| i + from) {
        let before_ok = i == 0 || !is_ident(bytes[i - 1] as char);
        let after = bytes.get(i + 2).copied().map(char::from);
        if before_ok && matches!(after, Some('(' | ' ' | '<')) {
            return Some(i);
        }
        from = i + 2;
    }
    None
}

fn is_receiver(param: &str) -> bool {
    let p = param.trim_start_matches("mut ").trim();
    let ty = p.strip_prefix("self:").map(str::trim).unwrap_or(p);
    let core = ty.trim_start_matches('&').trim_start();
    let core = if core.starts_with('\'') { core.split_once(' ').map_or(core, |(_, r)| r) } else { core };
    let core = core.trim_start_matches("mut ").trim();
    p == "self" || p.starts_with("self:") || core == "self" || core == "Self" || core.starts_with("Self<")
}

#[cfg(test)]
#[path = "tests/completion_call_tests.rs"]
mod tests;
