/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Doc comments turned into the Markdown an editor renders, the way rustdoc and rust-analyzer do: Rust code blocks
//! tagged `rust` with their hidden `# ` lines removed, and intra-doc links (`[`Circle`]`, `[text][crate::m]`,
//! `[x]: path`) pointed at the documentation page of the item they name.

use std::collections::HashSet;

/// Code-block attributes rustdoc treats as Rust (`edition2021`, `no_run`, …).
fn is_rust_block(info: &str) -> bool {
    info.split([',', ' ', '\t']).map(str::trim).filter(|t| !t.is_empty()).all(|t| {
        matches!(t, "rust" | "ignore" | "should_panic" | "no_run" | "compile_fail" | "test_harness" | "standalone_crate" | "allow_fail")
            || t.starts_with("edition")
            || t.starts_with("ignore-")
            || t.starts_with('{')
            || (t.len() > 1 && t.starts_with('E') && t[1..].bytes().all(|c| c.is_ascii_digit()))
    })
}

/// Rendered Markdown for the doc text `doc`; `resolve` maps an intra-doc link target (`Circle`, `crate::shapes`,
/// `Shape::area`, a `#section` anchor or a relative `.html` link) to a URL. Reference definitions move to the end,
/// as rust-analyzer's Markdown printer emits them.
pub(crate) fn render(doc: &str, resolve: &dyn Fn(&str) -> Option<String>) -> String {
    let mut definitions: Vec<(String, String)> = Vec::new();
    let mut body = Vec::new();
    let mut in_fence: Option<&str> = None;
    for line in doc.lines() {
        let trimmed = line.trim_start();
        let marker = ["```", "~~~"].into_iter().find(|m| trimmed.starts_with(m));
        match (in_fence, marker) {
            (None, Some(m)) => in_fence = Some(m),
            (Some(m), Some(close)) if m == close && trimmed[close.len()..].trim().is_empty() => in_fence = None,
            (None, None) => {
                if let Some((label, target)) = reference_definition(line) {
                    definitions.push((label.to_string(), target.to_string()));
                    continue;
                }
            }
            _ => {}
        }
        body.push(line);
    }
    let defined: HashSet<String> = definitions.iter().map(|(l, _)| l.clone()).collect();
    let resolve_definition = |label: &str| -> Option<String> {
        let target = &definitions.iter().find(|(l, _)| l == label)?.1;
        if is_url(target) {
            Some(target.clone())
        } else {
            resolve(target)
        }
    };
    let mut out: Vec<String> = Vec::new();
    let mut fence: Option<(String, bool)> = None;
    for line in body {
        let trimmed = line.trim_start();
        let marker = if trimmed.starts_with("```") {
            Some("```")
        } else if trimmed.starts_with("~~~") {
            Some("~~~")
        } else {
            None
        };
        match (&fence, marker) {
            (None, Some(m)) => {
                let info = trimmed[m.len()..].trim();
                let rust = is_rust_block(info);
                out.push(if rust { format!("{m}rust") } else { trimmed.to_string() });
                fence = Some((m.to_string(), rust));
            }
            (Some((m, _)), Some(close)) if close == m && trimmed[close.len()..].trim().is_empty() => {
                out.push(trimmed.to_string());
                fence = None;
            }
            (Some((_, true)), _) => {
                let code = line.trim_start();
                if code == "#" || code.starts_with("# ") {
                    continue;
                }
                out.push(match code.strip_prefix("##") {
                    Some(rest) => format!("{}#{rest}", &line[..line.len() - code.len()]),
                    None => line.to_string(),
                });
            }
            (Some(_), _) => out.push(line.to_string()),
            (None, None) => out.push(normalize_emphasis(&rewrite_links(line, &defined, resolve, &resolve_definition))),
        }
    }
    while out.last().is_some_and(|l| l.trim().is_empty()) {
        out.pop();
    }
    let mut collapsed: Vec<String> = Vec::with_capacity(out.len());
    for line in out {
        if line.trim().is_empty() && collapsed.last().is_some_and(|l| l.trim().is_empty()) {
            continue;
        }
        collapsed.push(line);
    }
    if !definitions.is_empty() {
        collapsed.push(String::new());
        for (label, target) in &definitions {
            let shown = if is_url(target) { Some(target.clone()) } else { resolve(target) };
            collapsed.push(format!("[{label}]: {}", shown.unwrap_or_else(|| target.clone())));
        }
    }
    collapsed.join("\n")
}

/// Whether `target` is a finished URL that needs no resolving.
fn is_url(target: &str) -> bool {
    target.contains("://") || target.starts_with("mailto:")
}

/// `_text_` emphasis written as `*text*` and `__text__` as `**text**`, outside code spans, as rust-analyzer's
/// Markdown printer normalizes it.
fn normalize_emphasis(line: &str) -> String {
    if !line.contains('_') {
        return line.to_string();
    }
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::with_capacity(line.len());
    let mut i = 0;
    let word = |c: Option<&char>| c.is_some_and(|c| c.is_alphanumeric() || *c == '_');
    while i < chars.len() {
        if chars[i] == '`' {
            let close = chars[i + 1..].iter().position(|&c| c == '`').map_or(chars.len(), |p| i + 1 + p);
            out.extend(&chars[i..(close + 1).min(chars.len())]);
            i = close + 1;
            continue;
        }
        if chars[i] == '_' && !word(i.checked_sub(1).and_then(|p| chars.get(p))) {
            let n = if chars.get(i + 1) == Some(&'_') { 2 } else { 1 };
            let open_ok = chars.get(i + n).is_some_and(|c| !c.is_whitespace() && *c != '_');
            if open_ok {
                let close = (i + n..chars.len().saturating_sub(n - 1))
                    .find(|&j| chars[j..j + n].iter().all(|&c| c == '_') && !chars[j - 1].is_whitespace() && !word(chars.get(j + n)) && (n == 2 || chars.get(j + 1) != Some(&'_')));
                if let Some(close) = close.filter(|&c| c > i + n) {
                    let stars = "*".repeat(n);
                    out.push_str(&stars);
                    out.extend(&chars[i + n..close]);
                    out.push_str(&stars);
                    i = close + n;
                    continue;
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// `[label]: target` at the start of `line`.
fn reference_definition(line: &str) -> Option<(&str, &str)> {
    let t = line.trim_start();
    let rest = t.strip_prefix('[')?;
    let close = rest.find("]:")?;
    let label = &rest[..close];
    if label.is_empty() || label.contains('[') {
        return None;
    }
    let target = rest[close + 2..].trim();
    let target = target.split_whitespace().next()?;
    Some((label, target.trim_start_matches('<').trim_end_matches('>')))
}

/// `line` with its intra-doc links resolved: `[text](path)`, `[text][path]` and `[path]` whose target is an item
/// path become `[text](url)`. Links with a reference definition, URLs and unresolved names stay as written.
fn rewrite_links(line: &str, defined: &HashSet<String>, resolve: &dyn Fn(&str) -> Option<String>, resolve_definition: &dyn Fn(&str) -> Option<String>) -> String {
    let b = line.as_bytes();
    let mut out = String::with_capacity(line.len());
    let mut i = 0;
    let mut copied = 0;
    while i < b.len() {
        match b[i] {
            b'`' => {
                let ticks = b[i..].iter().take_while(|&&c| c == b'`').count();
                let fence = &line[i..i + ticks];
                i = line[i + ticks..].find(fence).map_or(b.len(), |e| i + ticks + e + ticks);
            }
            b'[' => {
                let Some(close) = link_text_end(line, i) else {
                    i += 1;
                    continue;
                };
                let text = &line[i + 1..close];
                let after = &line[close + 1..];
                let (url, consumed) = if let Some(rest) = after.strip_prefix('(') {
                    match rest.find(')') {
                        Some(e) if !is_url(&rest[..e]) && !rest[..e].contains(' ') => (resolve(&rest[..e]), e + 2),
                        _ => (None, 0),
                    }
                } else if let Some(rest) = after.strip_prefix('[') {
                    match rest.find(']') {
                        Some(e) => {
                            let label = if rest[..e].is_empty() { text } else { &rest[..e] };
                            (if defined.contains(label) { resolve_definition(label) } else { resolve(label) }, e + 2)
                        }
                        None => (None, 0),
                    }
                } else if !after.starts_with(':') && !defined.contains(text) {
                    (resolve(text), 0)
                } else {
                    (None, 0)
                };
                match url {
                    Some(url) => {
                        out.push_str(&line[copied..i]);
                        out.push_str(&format!("[{text}]({url})"));
                        i = close + 1 + consumed;
                        copied = i;
                    }
                    None => i = close + 1,
                }
            }
            _ => i += 1,
        }
    }
    out.push_str(&line[copied..]);
    out
}

/// The `]` closing the link text opened at `open`, allowing code spans and one level of nested brackets.
fn link_text_end(line: &str, open: usize) -> Option<usize> {
    let b = line.as_bytes();
    let mut depth = 0;
    let mut i = open;
    while i < b.len() {
        match b[i] {
            b'`' => i = line[i + 1..].find('`').map(|e| i + 1 + e)?,
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    return (i > open + 1).then_some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// The item path an intra-doc link names: backticks, disambiguators (`struct@`, `fn@`), call parentheses, a macro
/// bang and generic arguments removed. `None` when the target is no path (spaces, empty).
#[cfg(test)]
pub(crate) fn link_path(target: &str) -> Option<String> {
    link_target(target).map(|t| t.path)
}

/// What an intra-doc link names: the item path and the kind its disambiguator or decoration asks for.
pub(crate) struct LinkTarget {
    pub path: String,
    /// `macro@x` or `x!`.
    pub wants_macro: bool,
    /// `prim@x` or `primitive@x`.
    pub wants_primitive: bool,
}

pub(crate) fn link_target(target: &str) -> Option<LinkTarget> {
    let mut t = target.trim().trim_matches('`').trim();
    let mut disambiguator = "";
    if let Some((kind, rest)) = t.split_once('@') {
        if kind.chars().all(|c| c.is_ascii_lowercase()) {
            disambiguator = kind;
            t = rest;
        }
    }
    let wants_macro = disambiguator == "macro" || t.ends_with('!');
    let wants_primitive = matches!(disambiguator, "prim" | "primitive");
    let t = t.trim_end_matches("()").trim_end_matches('!');
    let mut out = String::with_capacity(t.len());
    let mut depth = 0;
    for c in t.chars() {
        match c {
            '<' => depth += 1,
            '>' => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    let ok = !out.is_empty() && out.chars().all(|c| c.is_alphanumeric() || c == '_' || c == ':') && !out.starts_with(':');
    ok.then_some(LinkTarget { path: out, wants_macro, wants_primitive })
}

/// Primitive types whose documentation page an unqualified link reaches (`[`char`]`, `[`usize`]`).
pub(crate) fn is_primitive(name: &str) -> bool {
    matches!(name, "bool" | "char" | "str" | "u8" | "u16" | "u32" | "u64" | "u128" | "usize" | "i8" | "i16" | "i32" | "i64" | "i128" | "isize" | "f16" | "f32" | "f64" | "f128")
}

#[cfg(test)]
#[path = "tests/hover_doc_tests.rs"]
mod tests;
