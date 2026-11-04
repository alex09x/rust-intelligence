/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Hierarchical `DocumentSymbol[]` in rust-analyzer's shape: items with their members as children (fields, variants,
//! impl and trait items, module items, items declared inside function bodies), `impl` blocks named as written
//! (`impl Trait for Ty<'a>`, the impl's own generics and where clause left out) and selected on their self type,
//! associated functions with a `self` parameter as methods. Local bindings are not symbols.

use crate::lsp::editing::action_parse::mask_comments_and_strings;
use serde_json::{json, Value};

const MODULE: u32 = 2;
const METHOD: u32 = 6;
const FIELD: u32 = 8;
const ENUM: u32 = 10;
const INTERFACE: u32 = 11;
const FUNCTION: u32 = 12;
const VARIABLE: u32 = 13;
const CONSTANT: u32 = 14;
const OBJECT: u32 = 19;
const ENUM_MEMBER: u32 = 22;
const STRUCT: u32 = 23;
const TYPE_PARAMETER: u32 = 26;

#[derive(Clone, Copy, PartialEq)]
enum Ctx {
    Module,
    Impl,
    Trait,
    FnBody,
}

#[derive(Default)]
struct Sym {
    name: String,
    kind: u32,
    /// rust-analyzer's detail: a function's signature without its name (`fn(&self) -> u32`), a field's or constant's type.
    detail: Option<String>,
    range: (usize, usize),
    selection: (usize, usize),
    children: Vec<Sym>,
}

struct Parser<'a> {
    src: &'a str,
    m: &'a [u8],
    line_starts: Vec<usize>,
}

fn is_ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80
}

impl<'a> Parser<'a> {
    fn skip_ws(&self, mut i: usize, end: usize) -> usize {
        while i < end && self.m[i].is_ascii_whitespace() {
            i += 1;
        }
        i
    }

    /// The identifier starting at `i` (a raw identifier keeps its `r#`), and the offset just past it.
    fn ident(&self, i: usize, end: usize) -> Option<(&'a str, usize)> {
        let start = i;
        let mut j = i;
        if j + 1 < end && &self.m[j..j + 2] == b"r#" {
            j += 2;
        }
        let name_start = j;
        while j < end && is_ident(self.m[j]) {
            j += 1;
        }
        (j > name_start && !self.m[name_start].is_ascii_digit()).then(|| (&self.src[start..j], j))
    }

    /// Offset just past the group opened at `i` (`(`, `[`, `{` or `<`), or `end` when unbalanced.
    fn skip_group(&self, i: usize, end: usize) -> usize {
        let (open, close) = match self.m[i] {
            b'(' => (b'(', b')'),
            b'[' => (b'[', b']'),
            b'{' => (b'{', b'}'),
            _ => (b'<', b'>'),
        };
        let mut depth = 0i32;
        let mut j = i;
        while j < end {
            let c = self.m[j];
            if c == open {
                depth += 1;
            } else if c == close && !(close == b'>' && j > 0 && self.m[j - 1] == b'-') {
                depth -= 1;
                if depth == 0 {
                    return j + 1;
                }
            }
            j += 1;
        }
        end
    }

    /// First `{` or `;` at bracket depth 0 from `i`, skipping `(..)`, `[..]` and `<..>` groups.
    fn find_body(&self, mut i: usize, end: usize) -> usize {
        while i < end {
            match self.m[i] {
                b'{' | b';' => return i,
                b'(' | b'[' => i = self.skip_group(i, end),
                b'<' if i + 1 < end && self.m[i + 1] != b'=' && !(i > 0 && self.m[i - 1] == b'<') => i = self.skip_group(i, end),
                _ => i += 1,
            }
        }
        end
    }

    /// Offset past an item ending at `body`: the matching `}` of a `{`, or the `;` itself.
    fn item_end(&self, body: usize, end: usize) -> usize {
        if body < end && self.m[body] == b'{' {
            self.skip_group(body, end)
        } else {
            (body + 1).min(end)
        }
    }

    fn collapse(&self, a: usize, b: usize) -> String {
        self.src[a..b].split_whitespace().collect::<Vec<_>>().join(" ")
    }

    /// Items between `start` and `end` in context `ctx`.
    fn items(&self, start: usize, end: usize, ctx: Ctx) -> Vec<Sym> {
        let mut out = Vec::new();
        let mut i = start;
        let mut stmt_start = true;
        while i < end {
            i = self.skip_ws(i, end);
            if i >= end {
                break;
            }
            let c = self.m[i];
            if c == b'#' && i + 1 < end && (self.m[i + 1] == b'[' || self.m[i + 1] == b'!') {
                let open = if self.m[i + 1] == b'!' { i + 2 } else { i + 1 };
                i = if open < end && self.m[open] == b'[' { self.skip_group(open, end) } else { open };
                continue;
            }
            if stmt_start && is_ident(c) && (i == 0 || self.m[i - 1] != b'\'') {
                if let Some((sym, next)) = self.item_at(i, end, ctx) {
                    out.extend(sym);
                    i = next;
                    stmt_start = true;
                    continue;
                }
            }
            if ctx == Ctx::FnBody {
                // Statements: items only start right after `{`, `}` or `;`.
                stmt_start = matches!(c, b'{' | b'}' | b';');
                i += 1;
                continue;
            }
            // Module, impl and trait bodies: skip what is no item (a macro call `m!{..}` / `m!(..);`, a stray token).
            stmt_start = true;
            match c {
                b'{' | b'(' | b'[' => i = self.skip_group(i, end),
                _ if is_ident(c) => i = self.ident(i, end).map_or(i + 1, |(_, n)| n),
                _ => i += 1,
            }
        }
        out
    }

    /// The item starting at `i`: `Some((symbols, offset past the item))`, or `None` when no item starts there.
    fn item_at(&self, i: usize, end: usize, ctx: Ctx) -> Option<(Option<Sym>, usize)> {
        let (mut word, mut next) = self.ident(i, end)?;
        if word == "pub" {
            let mut p = self.skip_ws(next, end);
            if p < end && self.m[p] == b'(' {
                p = self.skip_group(p, end);
            }
            (word, next) = self.ident(self.skip_ws(p, end), end)?;
        }
        while matches!(word, "default" | "async" | "unsafe" | "extern" | "const" | "safe") {
            let k = self.skip_ws(next, end);
            // `extern "C"` has its ABI string masked to blanks; `const NAME` and `extern crate` are items themselves.
            let Some((w, n)) = self.ident(k, end) else { break };
            if word == "const" && !matches!(w, "fn" | "unsafe" | "async" | "extern") {
                break;
            }
            if word == "extern" && w == "crate" {
                break;
            }
            (word, next) = (w, n);
        }
        let item_start = i;
        match word {
            "fn" => {
                let k = self.skip_ws(next, end);
                let (name, after) = self.ident(k, end)?;
                let body = self.find_body(after, end);
                let close = self.item_end(body, end);
                let has_self = self.fn_has_self(after, body);
                let kind = if matches!(ctx, Ctx::Impl | Ctx::Trait) && has_self { METHOD } else { FUNCTION };
                let children = if body < end && self.m[body] == b'{' { self.items(body + 1, close - 1, Ctx::FnBody) } else { Vec::new() };
                // rust-analyzer's detail is the signature after the name: generics, parameters and return type.
                let detail = Some(format!("fn{}", self.collapse(after, self.where_start(after, body))));
                Some((Some(Sym { name: name.to_string(), kind, detail, range: (item_start, close), selection: (k, after), children }), close))
            }
            "struct" | "union" | "enum" | "trait" | "mod" => {
                let k = self.skip_ws(next, end);
                let (name, after) = self.ident(k, end)?;
                let body = self.find_body(after, end);
                let mut close = self.item_end(body, end);
                let has_braces = body < end && self.m[body] == b'{';
                let (kind, children) = match word {
                    "enum" => (ENUM, if has_braces { self.variants(body + 1, close - 1) } else { Vec::new() }),
                    "trait" => (INTERFACE, if has_braces { self.items(body + 1, close - 1, Ctx::Trait) } else { Vec::new() }),
                    "mod" => (MODULE, if has_braces { self.items(body + 1, close - 1, Ctx::Module) } else { Vec::new() }),
                    _ => {
                        // A tuple struct ends at its `;` after `(..)`; named fields are the children of `{..}`.
                        if !has_braces && body < end && self.m[body] == b';' {
                            close = body + 1;
                        }
                        (STRUCT, if has_braces { self.fields(body + 1, close - 1) } else { Vec::new() })
                    }
                };
                Some((Some(Sym { name: name.to_string(), kind, range: (item_start, close), selection: (k, after), children, ..Default::default() }), close))
            }
            "impl" => {
                let mut k = self.skip_ws(next, end);
                if k < end && self.m[k] == b'<' {
                    k = self.skip_ws(self.skip_group(k, end), end);
                }
                let body = self.find_body(k, end);
                let close = self.item_end(body, end);
                let header_end = self.where_start(k, body);
                let header = self.collapse(k, header_end);
                let self_start = self.self_type_start(k, header_end);
                let name = format!("impl {header}");
                let children = if body < end && self.m[body] == b'{' { self.items(body + 1, close - 1, Ctx::Impl) } else { Vec::new() };
                let sel_end = self.src[..header_end].trim_end().len();
                Some((Some(Sym { name, kind: OBJECT, range: (item_start, close), selection: (self_start, sel_end), children, ..Default::default() }), close))
            }
            "const" | "static" | "type" => {
                let k = self.skip_ws(next, end);
                let k = if word == "static" { self.ident(k, end).filter(|(w, _)| *w == "mut").map_or(k, |(_, n)| self.skip_ws(n, end)) } else { k };
                let (name, after) = self.ident(k, end).or_else(|| (k < end && self.m[k] == b'_').then(|| ("_", k + 1)))?;
                let mut e = after;
                while e < end && self.m[e] != b';' {
                    e = if matches!(self.m[e], b'{' | b'(' | b'[') { self.skip_group(e, end) } else { e + 1 };
                }
                let close = (e + 1).min(end);
                let kind = match word {
                    "const" => CONSTANT,
                    "static" => VARIABLE,
                    _ => TYPE_PARAMETER,
                };
                let detail = if word == "type" {
                    // A type alias shows its aliased type: `type Err = ColorError;` is `ColorError`.
                    let eq = (after..e).find(|&p| self.m[p] == b'=');
                    eq.map(|p| self.collapse(p + 1, e)).filter(|t| !t.is_empty())
                } else {
                    self.typed_detail(after, e)
                };
                let sym = (name != "_").then(|| Sym { name: name.to_string(), kind, detail, range: (item_start, close), selection: (k, after), ..Default::default() });
                Some((sym, close))
            }
            "macro_rules" => {
                let k = self.skip_ws(next, end);
                if k >= end || self.m[k] != b'!' {
                    return None;
                }
                let k = self.skip_ws(k + 1, end);
                let (name, after) = self.ident(k, end)?;
                let b = self.skip_ws(after, end);
                let close = if b < end && matches!(self.m[b], b'{' | b'(' | b'[') { self.skip_group(b, end) } else { after };
                Some((Some(Sym { name: name.to_string(), kind: FUNCTION, range: (item_start, close), selection: (k, after), ..Default::default() }), close))
            }
            "use" | "extern" => {
                // A foreign block `extern "C" { .. }` ends at its brace; `use ..;` and `extern crate ..;` at the `;`.
                let b = self.skip_ws(next, end);
                if word == "extern" && b < end && self.m[b] == b'{' {
                    return Some((None, self.skip_group(b, end)));
                }
                let mut e = next;
                while e < end && self.m[e] != b';' {
                    e = if self.m[e] == b'{' { self.skip_group(e, end) } else { e + 1 };
                }
                Some((None, (e + 1).min(end)))
            }
            _ => None,
        }
    }

    /// The type after `name:` up to a top-level `=` or the end: `u32` for `x: u32` and for `const X: u32 = 1`.
    fn typed_detail(&self, after_name: usize, end: usize) -> Option<String> {
        let colon = self.skip_ws(after_name, end);
        if colon >= end || self.m[colon] != b':' {
            return None;
        }
        let mut p = colon + 1;
        while p < end && self.m[p] != b'=' {
            p = if matches!(self.m[p], b'(' | b'[' | b'{') || (self.m[p] == b'<') { self.skip_group(p, end) } else { p + 1 };
        }
        let ty = self.collapse(colon + 1, p.min(end));
        (!ty.is_empty()).then_some(ty)
    }

    fn fn_has_self(&self, after_name: usize, body: usize) -> bool {
        let Some(open) = (after_name..body).find(|&p| self.m[p] == b'(') else { return false };
        let mut p = self.skip_ws(open + 1, body);
        loop {
            if p < body && self.m[p] == b'&' {
                p = self.skip_ws(p + 1, body);
            } else if p < body && self.m[p] == b'\'' {
                p = self.ident(p + 1, body).map_or(p + 1, |(_, n)| self.skip_ws(n, body));
            } else if let Some(("mut", n)) = self.ident(p, body) {
                p = self.skip_ws(n, body);
            } else {
                break;
            }
        }
        matches!(self.ident(p, body), Some(("self", _)))
    }

    /// Start of a `where` clause between `a` and `b`, or `b`.
    fn where_start(&self, a: usize, b: usize) -> usize {
        let mut p = a;
        while p < b {
            if self.m[p] == b'<' || self.m[p] == b'(' {
                p = self.skip_group(p, b);
                continue;
            }
            if let Some((w, n)) = self.ident(p, b) {
                if w == "where" {
                    return p;
                }
                p = n;
            } else {
                p += 1;
            }
        }
        b
    }

    /// Start of the self type in an impl header: after ` for ` at bracket depth 0, else the header start.
    fn self_type_start(&self, a: usize, b: usize) -> usize {
        let mut p = a;
        let mut found = a;
        while p < b {
            if self.m[p] == b'<' || self.m[p] == b'(' {
                p = self.skip_group(p, b);
                continue;
            }
            if let Some((w, n)) = self.ident(p, b) {
                if w == "for" && p > a && self.m[p - 1].is_ascii_whitespace() {
                    found = self.skip_ws(n, b);
                }
                p = n;
            } else {
                p += 1;
            }
        }
        if found == a && a < b && self.m[a] == b'!' {
            return self.skip_ws(a + 1, b);
        }
        found
    }

    /// Top-level comma-separated entries of a body, each with attributes and visibility skipped.
    fn entries(&self, a: usize, b: usize) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        let mut start = a;
        let mut p = a;
        while p < b {
            match self.m[p] {
                b'(' | b'[' | b'{' | b'<' => p = self.skip_group(p, b),
                b',' => {
                    out.push((start, p));
                    p += 1;
                    start = p;
                }
                _ => p += 1,
            }
        }
        out.push((start, b));
        out.into_iter()
            .map(|(s, e)| {
                let mut s = self.skip_ws(s, e);
                while s + 1 < e && self.m[s] == b'#' && self.m[s + 1] == b'[' {
                    s = self.skip_ws(self.skip_group(s + 1, e), e);
                }
                if let Some(("pub", n)) = self.ident(s, e) {
                    s = self.skip_ws(n, e);
                    if s < e && self.m[s] == b'(' {
                        s = self.skip_ws(self.skip_group(s, e), e);
                    }
                }
                (s, e)
            })
            .filter(|(s, e)| s < e)
            .collect()
    }

    fn fields(&self, a: usize, b: usize) -> Vec<Sym> {
        self.entries(a, b)
            .into_iter()
            .filter_map(|(s, e)| {
                let (name, after) = self.ident(s, e)?;
                Some(Sym { name: name.to_string(), kind: FIELD, detail: self.typed_detail(after, e), range: (s, e), selection: (s, after), ..Default::default() })
            })
            .collect()
    }

    fn variants(&self, a: usize, b: usize) -> Vec<Sym> {
        self.entries(a, b)
            .into_iter()
            .filter_map(|(s, e)| {
                let (name, after) = self.ident(s, e)?;
                let p = self.skip_ws(after, e);
                let children = if p < e && self.m[p] == b'{' { self.fields(p + 1, self.skip_group(p, e) - 1) } else { Vec::new() };
                Some(Sym { name: name.to_string(), kind: ENUM_MEMBER, range: (s, e), selection: (s, after), children, ..Default::default() })
            })
            .collect()
    }

    fn position(&self, offset: usize) -> Value {
        let line = self.line_starts.partition_point(|&s| s <= offset) - 1;
        let col = self.src[self.line_starts[line]..offset].encode_utf16().count();
        json!({ "line": line, "character": col })
    }

    fn to_json(&self, s: &Sym) -> Value {
        let mut v = json!({
            "name": s.name,
            "kind": s.kind,
            "range": { "start": self.position(s.range.0), "end": self.position(s.range.1) },
            "selectionRange": { "start": self.position(s.selection.0), "end": self.position(s.selection.1) },
            "children": s.children.iter().map(|c| self.to_json(c)).collect::<Vec<_>>()
        });
        if let Some(d) = &s.detail {
            v["detail"] = json!(d);
        }
        v
    }
}

/// `DocumentSymbol[]` of a Rust source text.
pub fn document_symbols(text: &str) -> Value {
    let masked = mask_comments_and_strings(text);
    let mut line_starts = vec![0];
    line_starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
    let p = Parser { src: text, m: masked.as_bytes(), line_starts };
    let syms = p.items(0, text.len(), Ctx::Module);
    json!(syms.iter().map(|s| p.to_json(s)).collect::<Vec<_>>())
}

#[cfg(test)]
#[path = "document_symbol_tree_tests.rs"]
mod tests;
