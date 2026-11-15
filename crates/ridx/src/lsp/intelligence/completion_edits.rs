/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! What accepting a completion item writes, the way rust-analyzer writes it: a function or method is inserted as a
//! call (`len()$0`, `push_str(${1:string})$0`, see [`completion_call`]), a postfix snippet rewrites its receiver
//! (`s.dbg` → `dbg!(s)`). The list builders decide which items exist, marking where each function is declared;
//! this pass gives each item its edit. Edits stay on the cursor's line: the main edit replaces the word being
//! typed, and the receiver a postfix snippet consumes is removed by an additional edit.
//!
//! [`completion_call`]: super::completion_call

use super::completion_call::{call_edit, client};
use super::fn_params::{params_in, params_on_disk, FnParams};
use serde_json::{json, Value};
use std::path::Path;

const METHOD_KIND: u64 = 2;
const FUNCTION_KIND: u64 = 3;
const SNIPPET_KIND: u64 = 15;
pub(crate) const DECL_KEY: &str = "ridxDecl";

/// Marks `item` as declared from the 1-based `line` of `file`: a function's parameters name the placeholders of its
/// call ([`apply`]), and `completionItem/resolve` reads the declaration's doc comment from there.
pub(crate) fn mark_decl(item: &mut Value, file: &Path, line: u32) {
    if let Some(file) = file.to_str() {
        item["data"] = json!({ DECL_KEY: { "file": file, "line": line } });
    }
}

/// Where `item` is declared, by its mark.
pub(crate) fn item_decl(item: &Value) -> Option<(String, u32)> {
    let decl = item.get("data")?.get(DECL_KEY)?;
    Some((decl["file"].as_str()?.to_string(), decl["line"].as_u64()? as u32))
}

/// The cursor's line split around the cursor, with what the edits need to know about it.
pub struct EditSite<'a> {
    /// The line up to the cursor.
    pub before: &'a str,
    /// The line from the cursor on.
    pub after: &'a str,
    pub line_0based: u32,
    /// The cursor's UTF-16 column.
    pub col_utf16: u32,
    /// Whether the cursor is inside a `use` item, where a function is named, not called.
    pub in_use_item: bool,
    /// The whole document, for the imports a postfix snippet needs and the declarations it holds.
    pub doc_text: &'a str,
    /// The document's path.
    pub file: &'a Path,
}

/// The parameters of the function declared at `decl`: from the open document when it is declared there.
fn decl_params(site: &EditSite<'_>, decl: &(String, u32), name: &str) -> Option<FnParams> {
    let file = Path::new(&decl.0);
    if file == site.file {
        params_in(site.doc_text, decl.1, name)
    } else {
        params_on_disk(file, decl.1, name)
    }
}

/// Gives every item of a completion response (`{items: [...]}` or a bare list) the edit accepting it makes.
pub fn apply(response: &mut Value, site: &EditSite<'_>) {
    let snippets = client().snippets;
    let items = match response {
        Value::Array(items) => items,
        Value::Object(obj) => match obj.get_mut("items") {
            Some(Value::Array(items)) => items,
            _ => return,
        },
        _ => return,
    };
    let word = word_at(site);
    let receiver = receiver_before(site.before, word.prefix_chars);
    let head: String = site.before.chars().take(site.before.chars().count() - word.prefix_chars).collect();
    let method_call = head.ends_with('.') && !head.ends_with("..");
    for it in items.iter_mut() {
        let decl = item_decl(it);
        if it.get("textEdit").is_some() {
            continue;
        }
        match it["kind"].as_u64() {
            Some(METHOD_KIND | FUNCTION_KIND) if !site.in_use_item && !word.followed_by_call => {
                let name = it["insertText"].as_str().or_else(|| it["label"].as_str()).unwrap_or_default();
                let name = name.rsplit('.').next().unwrap_or(name).to_string();
                let params = decl.as_ref().and_then(|d| decl_params(site, d, &name));
                call_edit(it, site, &word, method_call, params.as_ref());
            }
            Some(SNIPPET_KIND) => {
                if let Some(recv) = receiver.as_ref() {
                    postfix_edit(it, site, &word, recv, snippets);
                }
            }
            _ => {}
        }
    }
}

/// The word under the cursor: its UTF-16 range on the line and whether a call already follows it.
pub(super) struct Word {
    start_utf16: u32,
    end_utf16: u32,
    prefix_chars: usize,
    followed_by_call: bool,
}

pub(super) fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn word_at(site: &EditSite<'_>) -> Word {
    let prefix: Vec<char> = site.before.chars().rev().take_while(|c| is_ident(*c)).collect();
    let suffix: String = site.after.chars().take_while(|c| is_ident(*c)).collect();
    let rest = site.after[suffix.len()..].trim_start();
    let prefix_utf16: u32 = prefix.iter().map(|c| c.len_utf16() as u32).sum();
    let suffix_utf16: u32 = suffix.chars().map(|c| c.len_utf16() as u32).sum();
    Word {
        start_utf16: site.col_utf16.saturating_sub(prefix_utf16),
        end_utf16: site.col_utf16 + suffix_utf16,
        prefix_chars: prefix.len(),
        followed_by_call: rest.starts_with('(') || rest.starts_with('!') || rest.starts_with("::<"),
    }
}

fn range(line: u32, start: u32, end: u32) -> Value {
    json!({ "start": { "line": line, "character": start }, "end": { "line": line, "character": end } })
}

pub(super) fn set_main_edit(it: &mut Value, site: &EditSite<'_>, word: &Word, text: String, snippets: bool) {
    if let Some(obj) = it.as_object_mut() {
        obj.remove("insertText");
    }
    it["textEdit"] = json!({ "range": range(site.line_0based, word.start_utf16, word.end_utf16), "newText": text });
    if snippets {
        it["insertTextFormat"] = json!(2);
    }
    if it.get("filterText").is_none() {
        let label = it["label"].clone();
        it["filterText"] = label;
    }
}

/// The receiver expression ending at the `.` before the word being typed, as the byte range of `before` it spans
/// (through the dot). Only receivers on the cursor's line are rewritten.
#[derive(Debug, PartialEq)]
pub(crate) struct Receiver {
    pub start_byte: usize,
    pub dot_byte: usize,
}

pub(crate) fn receiver_before(before: &str, prefix_chars: usize) -> Option<Receiver> {
    let head: String = before.chars().take(before.chars().count() - prefix_chars).collect();
    let dot_byte = head.len().checked_sub(1).filter(|&i| head.as_bytes()[i] == b'.')?;
    if dot_byte > 0 && head.as_bytes()[dot_byte - 1] == b'.' {
        return None;
    }
    let chars: Vec<(usize, char)> = head[..dot_byte].char_indices().collect();
    let mut i = chars.len();
    let mut start = dot_byte;
    while i > 0 {
        let (b, c) = chars[i - 1];
        let inner_dot = c == '.' && i >= 2 && chars[i - 2].1 != '.' && start < dot_byte;
        if is_ident(c) || c == '?' || c == '!' || inner_dot {
            i -= 1;
            start = b;
        } else if c == ':' && i >= 2 && chars[i - 2].1 == ':' {
            i -= 2;
            start = chars[i].0;
        } else if matches!(c, ')' | ']' | '}' | '"' | '\'') {
            let open = matching_open(&chars, i - 1)?;
            i = open;
            start = chars[open].0;
        } else {
            break;
        }
    }
    (start < dot_byte).then_some(Receiver { start_byte: start, dot_byte })
}

/// The index of the bracket or quote opening the one closing at `close`.
fn matching_open(chars: &[(usize, char)], close: usize) -> Option<usize> {
    let c = chars[close].1;
    if c == '"' || c == '\'' {
        return (0..close).rev().find(|&j| chars[j].1 == c && (j == 0 || chars[j - 1].1 != '\\'));
    }
    let open = match c {
        ')' => '(',
        ']' => '[',
        _ => '{',
    };
    let mut depth = 0;
    for j in (0..=close).rev() {
        match chars[j].1 {
            x if x == c => depth += 1,
            x if x == open => {
                depth -= 1;
                if depth == 0 {
                    return Some(j);
                }
            }
            _ => {}
        }
    }
    None
}

fn postfix_edit(it: &mut Value, site: &EditSite<'_>, word: &Word, recv: &Receiver, snippets: bool) {
    let label = it["label"].as_str().unwrap_or_default().to_string();
    let detail = it["detail"].as_str().unwrap_or_default().to_string();
    let receiver_text = &site.before[recv.start_byte..recv.dot_byte];
    let indent: String = site.before.chars().take_while(|c| c.is_whitespace()).collect();
    let Some(template) = postfix_template(&label, &detail, &indent) else { return };
    let text = if snippets {
        let escaped = receiver_text.replace('\\', "\\\\").replace('$', "\\$").replace('}', "\\}");
        template.replace("{R}", &escaped)
    } else {
        strip_placeholders(&template).replace("{R}", receiver_text)
    };
    let start_utf16 = site.before[..recv.start_byte].encode_utf16().count() as u32;
    let mut extra = vec![json!({ "range": range(site.line_0based, start_utf16, word.start_utf16), "newText": "" })];
    let import = match label.as_str() {
        "arc" => Some("std::sync::Arc"),
        "rc" => Some("std::rc::Rc"),
        _ => None,
    };
    if let Some(edit) = import.and_then(|path| super::auto_import::compute_auto_import_edit(site.doc_text, path)) {
        extra.push(edit);
    }
    set_main_edit(it, site, word, text, snippets);
    it["additionalTextEdits"] = Value::Array(extra);
}

/// rust-analyzer's text for the postfix snippet `label` (its `detail` tells `Some` from `Ok`), `{R}` standing for the
/// receiver.
fn postfix_template(label: &str, detail: &str, indent: &str) -> Option<String> {
    let variant = if detail.contains("Ok") { "Ok" } else { "Some" };
    let block = |head: String| format!("{head} {{\n{indent}    $0\n{indent}}}");
    Some(match label {
        "arc" => "Arc::new({R})".into(),
        "box" => "Box::new({R})".into(),
        "call" => "${1}({R})".into(),
        "const" => "const { {R} }".into(),
        "dbg" => "dbg!({R})".into(),
        "dbgr" => "dbg!(&{R})".into(),
        "deref" => "*{R}".into(),
        "err" => "Err({R})".into(),
        "ok" => "Ok({R})".into(),
        "pinbox" => "Box::pin({R})".into(),
        "rc" => "Rc::new({R})".into(),
        "ref" => "&{R}".into(),
        "refm" => "&mut {R}".into(),
        "return" => "return {R}".into(),
        "some" => "Some({R})".into(),
        "unsafe" => "unsafe { {R} }".into(),
        "not" => "!{R}".into(),
        "break" => "break {R}".into(),
        "drop" => "drop({R})".into(),
        "let" if detail.starts_with("let Some") || detail.starts_with("let Ok") => format!("let {variant}(${{1:_}}) = {{R}}"),
        "let" => "let $1 = {R};$0".into(),
        "letm" => "let mut $1 = {R};$0".into(),
        "match" => format!("match {{R}} {{\n{indent}    ${{1:_}} => {{$0}}\n{indent}}}"),
        "if" => block("if {R}".into()),
        "ifl" => block(format!("if let {variant}(${{1:_}}) = {{R}}")),
        "while" if detail.starts_with("while let") => block(format!("while let {variant}(${{1:_}}) = {{R}}")),
        "while" => block("while {R}".into()),
        "lete" => format!("let {variant}(${{1:_}}) = {{R}} else {{\n{indent}    $0\n{indent}}};"),
        "for" => block("for ${1:ele} in {R}".into()),
        _ => return None,
    })
}

/// Snippet text as plain text: `${1:_}` becomes `_`, tab stops vanish.
fn strip_placeholders(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '$' {
            out.push(c);
            continue;
        }
        if chars.peek() == Some(&'{') {
            chars.next();
            while chars.peek().is_some_and(char::is_ascii_digit) {
                chars.next();
            }
            if chars.peek() == Some(&':') {
                chars.next();
            }
            for d in chars.by_ref() {
                if d == '}' {
                    break;
                }
                out.push(d);
            }
        } else {
            while chars.peek().is_some_and(char::is_ascii_digit) {
                chars.next();
            }
        }
    }
    out
}

/// Whether `offset` of `text` lies inside a `use` item: the statement it belongs to (after the last `;` or `}`)
/// starts with `use`, optionally behind a visibility and attributes.
pub fn in_use_item(text: &str, offset: usize) -> bool {
    let head = &text[..offset.min(text.len())];
    let start = head.rfind([';', '}']).map_or(0, |i| i + 1);
    let mut stmt = head[start..].trim_start();
    loop {
        if let Some(rest) = stmt.strip_prefix("//") {
            stmt = rest.split_once('\n').map_or("", |(_, r)| r).trim_start();
        } else if stmt.starts_with("#[") {
            stmt = stmt.split_once(']').map_or("", |(_, r)| r).trim_start();
        } else {
            break;
        }
    }
    let stmt = match stmt.strip_prefix("pub") {
        Some(rest) if rest.starts_with('(') => rest.split_once(')').map_or("", |(_, r)| r).trim_start(),
        Some(rest) => rest.trim_start(),
        None => stmt,
    };
    stmt.starts_with("use ") || stmt.starts_with("use\n")
}

#[cfg(test)]
#[path = "tests/completion_edits_tests.rs"]
pub(crate) mod tests;
