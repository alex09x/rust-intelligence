/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! LSP folding ranges in rust-analyzer's shape: every multi-line `{}` / `()` / `[]` group (blocks, item and field
//! lists, argument and parameter lists, arrays, macro token trees), whole functions, `where` clauses, whole
//! `if … else` chains, multi-line statements, and runs of comment lines (`comment`) and `use` items (`imports`).

use crate::lsp::editing::action_parse::mask_comments_and_strings;
use crate::lsp::protocol::uri_to_path;
use crate::overlay::OverlayManager;
use serde_json::{json, Value};

struct Folder<'a> {
    src: &'a str,
    m: &'a [u8],
    line_starts: Vec<usize>,
    out: Vec<(usize, usize, Option<&'static str>)>,
}

fn is_ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

impl<'a> Folder<'a> {
    fn line(&self, offset: usize) -> usize {
        self.line_starts.partition_point(|&s| s <= offset) - 1
    }

    fn push(&mut self, start: usize, end: usize, kind: Option<&'static str>) {
        if end > start && self.line(start) < self.line(end) {
            self.out.push((start, end, kind));
        }
    }

    fn word_at(&self, i: usize) -> &'a str {
        if i >= self.m.len() || !is_ident(self.m[i]) || (i > 0 && is_ident(self.m[i - 1])) {
            return "";
        }
        let mut j = i;
        while j < self.m.len() && is_ident(self.m[j]) {
            j += 1;
        }
        &self.src[i..j]
    }

    fn next_significant(&self, mut i: usize) -> usize {
        while i < self.m.len() && self.m[i].is_ascii_whitespace() {
            i += 1;
        }
        i
    }

    /// Matching closer of the bracket at `open`, or `None` when unbalanced.
    fn close_of(&self, open: usize) -> Option<usize> {
        let mut stack = Vec::new();
        for (j, &c) in self.m.iter().enumerate().skip(open) {
            match c {
                b'{' | b'(' | b'[' => stack.push(c),
                b'}' | b')' | b']' => {
                    stack.pop()?;
                    if stack.is_empty() {
                        return Some(j);
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// Start of the outer attributes (`#[…]`) directly before `at`, or `at` when there are none.
    fn attrs_start(&self, at: usize) -> usize {
        let m = self.m;
        let mut start = at;
        loop {
            let mut p = start;
            while p > 0 && m[p - 1].is_ascii_whitespace() {
                p -= 1;
            }
            if p == 0 || m[p - 1] != b']' {
                return start;
            }
            let mut depth = 0i32;
            let mut q = p;
            while q > 0 {
                q -= 1;
                match m[q] {
                    b']' => depth += 1,
                    b'[' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
            }
            if depth != 0 || q == 0 || m[q - 1] != b'#' {
                return start;
            }
            start = q - 1;
        }
    }

    /// Bracket groups, functions, where clauses, if-else chains and statements.
    fn structure(&mut self) {
        let mut opens: Vec<usize> = Vec::new();
        let m = self.m;
        for (i, &c) in m.iter().enumerate() {
            match c {
                b'{' | b'(' | b'[' => opens.push(i),
                b'}' | b')' | b']' => {
                    if let Some(o) = opens.pop() {
                        // A block under attributes (`#[cfg(x)] { … }`) folds from its first attribute.
                        let start = if c == b'}' { self.attrs_start(o) } else { o };
                        self.push(start, i, None);
                        if c == b'}' {
                            self.statements(o + 1, i);
                        }
                    }
                }
                _ => {}
            }
        }
        self.statements(0, m.len());
        let mut i = 0;
        while i < m.len() {
            match self.word_at(i) {
                "fn" => {
                    let body = (i..m.len()).find(|&p| matches!(m[p], b'{' | b';'));
                    if let Some(b) = body.filter(|&b| m[b] == b'{') {
                        if let Some(end) = self.close_of(b) {
                            self.push(i, end, None);
                        }
                    }
                }
                "where" => {
                    let end = (i..m.len()).find(|&p| matches!(m[p], b'{' | b';')).unwrap_or(m.len());
                    let last = self.src[i..end].trim_end().len() + i;
                    self.push(i, last, None);
                }
                "if" if i == 0 || !(m[i - 1] == b' ' && self.src[..i].trim_end().ends_with("else")) => self.if_chain(i),
                _ => {}
            }
            i += 1;
        }
    }

    /// A whole `if … else …` chain from its `if` to the end of its last block.
    fn if_chain(&mut self, start: usize) {
        let m = self.m;
        let mut p = start;
        loop {
            let Some(open) = (p..m.len()).find(|&q| m[q] == b'{') else { return };
            let Some(close) = self.close_of(open) else { return };
            let next = self.next_significant(close + 1);
            if self.word_at(next) == "else" {
                p = next + 4;
                continue;
            }
            self.push(start, close, None);
            return;
        }
    }

    /// Multi-line statements ending in `;` at the top level of `[from, to)`: `let` bindings and expression
    /// statements such as builder chains, not items (those fold as their own groups).
    fn statements(&mut self, from: usize, to: usize) {
        let m = self.m;
        let mut depth = 0i32;
        let mut stmt_start: Option<usize> = None;
        let mut p = from;
        while p < to {
            let c = m[p];
            match c {
                b'{' | b'(' | b'[' => depth += 1,
                b'}' | b')' | b']' => {
                    depth -= 1;
                    if depth == 0 && c == b'}' {
                        // A block-like expression (`if`, `match`, a nested block) ends a statement without `;`.
                        stmt_start = None;
                    }
                }
                b';' if depth == 0 => {
                    if let Some(s) = stmt_start.take() {
                        let w = self.word_at(s);
                        if !matches!(w, "use" | "mod" | "extern" | "type" | "const" | "static" | "pub" | "fn" | "struct" | "enum" | "impl" | "trait") {
                            self.push(s, p, None);
                        }
                    }
                }
                _ if depth == 0 && stmt_start.is_none() && !c.is_ascii_whitespace() => {
                    if c == b'#' {
                        // An attribute is no statement start.
                    } else {
                        stmt_start = Some(p);
                    }
                }
                _ => {}
            }
            p += 1;
        }
    }

    /// Runs of two or more adjacent lines of one group: comments of one flavour (`comment`), `use` items (`imports`),
    /// body-less `mod x;` declarations, and `const` / `static` items of one visibility. Also `// region` …
    /// `// endregion` marker pairs.
    fn runs(&mut self) {
        let lines: Vec<&str> = self.src.lines().collect();
        let mut regions: Vec<usize> = Vec::new();
        let mut i = 0;
        while i < lines.len() {
            match region_marker(lines[i]) {
                Some(true) => {
                    regions.push(i);
                    i += 1;
                    continue;
                }
                Some(false) => {
                    if let Some(open) = regions.pop() {
                        let start = self.line_starts[open] + indent(lines[open]);
                        let end = self.line_starts[i] + lines[i].trim_end().len();
                        self.push(start, end, Some("region"));
                    }
                    i += 1;
                    continue;
                }
                None => {}
            }
            let Some((kind, key)) = line_group(lines[i]) else {
                i += 1;
                continue;
            };
            let mut j = i;
            loop {
                // An item may span lines up to its `;`.
                if kind != Some("comment") {
                    while j + 1 < lines.len() && !lines[j].trim_end().ends_with(';') {
                        j += 1;
                    }
                }
                let next = lines.get(j + 1).copied().unwrap_or("");
                if region_marker(next).is_some() || line_group(next) != Some((kind, key)) {
                    break;
                }
                j += 1;
            }
            if j > i {
                let start = self.line_starts[i] + indent(lines[i]);
                let end = self.line_starts[j] + lines[j].trim_end().len();
                self.push(start, end, kind);
            }
            i = j + 1;
        }
    }

    fn to_json(&self) -> Value {
        // One range per line span and kind (a function whose header fits one line folds once, as in rust-analyzer),
        // in closing order: inner ranges before the ranges that enclose them.
        let mut seen = std::collections::HashSet::new();
        let mut items: Vec<(usize, usize, Option<&str>)> = self.out.iter().copied().filter(|&(s, e, k)| seen.insert((self.line(s), self.line(e), k))).collect();
        items.sort_by_key(|&(s, e, _)| (self.line(e), std::cmp::Reverse(self.line(s))));
        let col = |off: usize| self.src[self.line_starts[self.line(off)]..off].encode_utf16().count();
        json!(items
            .iter()
            .map(|&(s, e, k)| {
                let mut v = json!({ "startLine": self.line(s), "startCharacter": col(s), "endLine": self.line(e), "endCharacter": col(e) });
                v["kind"] = json!(k.unwrap_or("region"));
                v
            })
            .collect::<Vec<_>>())
    }
}

fn indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// `Some(true)` for a `// region` marker line, `Some(false)` for `// endregion`.
fn region_marker(line: &str) -> Option<bool> {
    let rest = line.trim_start().strip_prefix("//")?.trim_start();
    let rest = rest.strip_prefix('#').unwrap_or(rest);
    let word_end = |s: &str, w: &str| s.strip_prefix(w).is_some_and(|r| !r.starts_with(|c: char| c.is_alphanumeric() || c == '_'));
    if word_end(rest, "region") {
        Some(true)
    } else if word_end(rest, "endregion") {
        Some(false)
    } else {
        None
    }
}

/// The run a line belongs to: `(fold kind, group key)`.
fn line_group(line: &str) -> Option<(Option<&'static str>, &'static str)> {
    let t = line.trim_start();
    if let Some(rest) = t.strip_prefix("//") {
        let flavour = if rest.starts_with('/') && !rest.starts_with("//") {
            "///"
        } else if rest.starts_with('!') {
            "//!"
        } else {
            "//"
        };
        return Some((Some("comment"), flavour));
    }
    let (vis, item) = match t.strip_prefix("pub") {
        Some(r) if r.starts_with('(') => r.find(')').map_or(("", t), |e| (&t[..3 + e + 1], r[e + 1..].trim_start())),
        Some(r) if r.starts_with(' ') => ("pub", r.trim_start()),
        _ => ("", t),
    };
    let first = item.split(|c: char| !(c.is_alphanumeric() || c == '_')).next().unwrap_or("");
    match first {
        "use" => Some((Some("imports"), "use")),
        "mod" if vis.is_empty() && item.trim_end().ends_with(';') => Some((None, "mod")),
        "const" if !item.starts_with("const fn") && !item.starts_with("const unsafe") => Some((None, const_key(vis, "const"))),
        "static" => Some((None, const_key(vis, "static"))),
        _ => None,
    }
}

/// Group key for `const` / `static` runs: the item keyword and its visibility.
fn const_key(vis: &str, kw: &'static str) -> &'static str {
    match (kw, vis) {
        ("const", "") => "const",
        ("const", "pub") => "pub const",
        ("const", _) => "pub(..) const",
        (_, "") => "static",
        (_, "pub") => "pub static",
        _ => "pub(..) static",
    }
}

fn collect_folding_ranges(content: &str) -> Value {
    let masked = mask_comments_and_strings(content);
    let mut line_starts = vec![0];
    line_starts.extend(content.match_indices('\n').map(|(i, _)| i + 1));
    let mut f = Folder { src: content, m: masked.as_bytes(), line_starts, out: Vec::new() };
    f.structure();
    f.runs();
    f.to_json()
}

#[cfg(test)]
#[path = "folding_tests.rs"]
mod tests;

/// Handles `textDocument/foldingRange` requests from the open buffer, or the file on disk when closed.
pub fn handle_folding_range(msg: &Value, overlay_mgr: &OverlayManager) -> Value {
    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
    let file = uri_to_path(uri);
    if let Some(overlay) = overlay_mgr.get(&file) {
        return collect_folding_ranges(&overlay.text);
    }
    std::fs::read_to_string(&file).map_or_else(|_| json!([]), |content| collect_folding_ranges(&content))
}
