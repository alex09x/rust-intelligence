/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! A function's parameters as its declaration in source writes them (the standard library's from `rust-src`):
//! what parameter-name hints and the argument placeholders of a completed call are made of. Signatures the index
//! and the compiler hand out carry parameter types only.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

/// One parameter of a declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Param {
    /// The receiver (`self`, `&mut self`, `self: Pin<&mut Self>`), written as `text`.
    pub is_self: bool,
    /// The pattern before `:` (`x`, `mut buf`, `(a, b)`, `_`), or the whole receiver.
    pub pat: String,
    /// The type after `:`, empty for a receiver without one.
    pub ty: String,
}

impl Param {
    /// The name the pattern binds, when it is a single binding (`mut x` → `x`).
    pub fn binding(&self) -> Option<&str> {
        if self.is_self {
            return Some("self");
        }
        let pat = self.pat.trim_start_matches("ref ").trim_start_matches("mut ").trim();
        (!pat.is_empty() && pat != "_" && pat.chars().all(|c| c.is_alphanumeric() || c == '_')).then_some(pat)
    }
}

/// A declaration's generic parameter names and its parameters.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct FnParams {
    pub generics: Vec<String>,
    pub params: Vec<Param>,
}

type Key = (PathBuf, u32, String);
type Stamp = (Option<SystemTime>, u64);
type Cache = Mutex<HashMap<Key, (Stamp, Option<FnParams>)>>;

fn cache() -> &'static Cache {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// The parameters of the function `name` declared from the 1-based `line` of `file` on disk, cached until the file
/// changes.
pub(crate) fn params_on_disk(file: &Path, line: u32, name: &str) -> Option<FnParams> {
    let meta = std::fs::metadata(file).ok()?;
    let stamp = (meta.modified().ok(), meta.len());
    let key = (file.to_path_buf(), line, name.to_string());
    if let Some((s, p)) = cache().lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        if *s == stamp {
            return p.clone();
        }
    }
    let parsed = std::fs::read_to_string(file).ok().and_then(|text| params_in(&text, line, name));
    let mut c = cache().lock().unwrap_or_else(|e| e.into_inner());
    if c.len() >= 8192 {
        c.clear();
    }
    c.insert(key, (stamp, parsed.clone()));
    parsed
}

/// The 1-based line of `text` declaring `name` nearest to `line`: `line` itself when it declares it (an index line is
/// exact for the text it was built from), else the closest declaring line within 400 lines (the file changed since
/// it was indexed).
pub(crate) fn decl_line_near(text: &str, line: u32, name: &str) -> Option<u32> {
    let lines: Vec<&str> = text.lines().collect();
    let at = line.saturating_sub(1) as usize;
    let declares = |i: usize| lines.get(i).is_some_and(|l| declares_name(l, name));
    if declares(at) {
        return Some(line);
    }
    (1..=400usize).find_map(|d| [at.checked_sub(d), Some(at + d)].into_iter().flatten().find(|&i| declares(i))).map(|i| i as u32 + 1)
}

/// Whether `line` declares `name`: an item (`fn name`, `struct name`, …) or a field or variant (`name: T`, `Name(..)`).
fn declares_name(line: &str, name: &str) -> bool {
    let bounded = |rest: &str| !rest.starts_with(|c: char| c.is_alphanumeric() || c == '_');
    let t = line.trim_start();
    let item = ["fn", "struct", "enum", "union", "trait", "type", "const", "static", "mod"].iter().any(|kw| {
        let needle = format!("{kw} {name}");
        t.match_indices(&needle).any(|(i, _)| (i == 0 || !t[..i].ends_with(|c: char| c.is_alphanumeric() || c == '_')) && bounded(&t[i + needle.len()..]))
    });
    let member = t.strip_prefix("pub(crate) ").or_else(|| t.strip_prefix("pub ")).unwrap_or(t).strip_prefix(name).is_some_and(|rest| {
        let rest = rest.trim_start();
        bounded(rest) && (rest.starts_with(':') && !rest.starts_with("::") || rest.starts_with('(') || rest.starts_with('{') || rest.starts_with(',') || rest.is_empty())
    });
    item || member
}

/// The parameters of the function `name` declared from the 1-based `line` of `text`.
pub(crate) fn params_in(text: &str, line: u32, name: &str) -> Option<FnParams> {
    let line = decl_line_near(text, line, name).unwrap_or(line);
    let line_start = text.split_inclusive('\n').take(line.saturating_sub(1) as usize).map(str::len).sum::<usize>();
    let mut end = (line_start + 4000).min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let window = text.get(line_start..end)?;
    let needle = format!("fn {name}");
    let fn_at = window.match_indices(&needle).map(|(i, _)| i).find(|&i| !window[i + needle.len()..].starts_with(|c: char| c.is_alphanumeric() || c == '_'))?;
    let mut after = window[fn_at + needle.len()..].trim_start();
    let mut generics = Vec::new();
    if after.starts_with('<') {
        let close = generics_end(after)?;
        generics = generic_names(&after[1..close]);
        after = &after[close + 1..];
    }
    let open = after.find('(')?;
    let masked = crate::lsp::editing::action_parse::mask_comments_and_strings(&after[open..]);
    let close = matching_paren(masked.as_bytes())?;
    let params = top_level_split(&masked, 1, close).into_iter().map(|(s, e)| strip_attrs(after[open + s..open + e].trim())).filter(|p| !p.is_empty()).map(param).collect();
    Some(FnParams { generics, params })
}

/// The signature of the function `name` declared from the 1-based `line` of `text` as rust-analyzer labels a
/// signature: `fn name<generics>(params as written) -> Ret`, without visibility, attributes, `where` clause or body.
pub(crate) fn signature_in(text: &str, line: u32, name: &str) -> Option<String> {
    let line = decl_line_near(text, line, name).unwrap_or(line);
    let line_start = text.split_inclusive('\n').take(line.saturating_sub(1) as usize).map(str::len).sum::<usize>();
    let mut end = (line_start + 4000).min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let window = text.get(line_start..end)?;
    let needle = format!("fn {name}");
    let fn_at = window.match_indices(&needle).map(|(i, _)| i).find(|&i| !window[i + needle.len()..].starts_with(|c: char| c.is_alphanumeric() || c == '_'))?;
    let decl = &window[fn_at..];
    let masked = crate::lsp::editing::action_parse::mask_comments_and_strings(decl);
    let mb = masked.as_bytes();
    let mut i = needle.len();
    while mb.get(i).is_some_and(u8::is_ascii_whitespace) {
        i += 1;
    }
    let generics = if mb.get(i) == Some(&b'<') { generics_end(&masked[i..]).map(|e| (i, i + e + 1)) } else { None };
    let open = masked[generics.map_or(i, |g| g.1)..].find('(')? + generics.map_or(i, |g| g.1);
    let close = open + matching_paren(&mb[open..])?;
    let params: Vec<String> = top_level_split(&masked, open + 1, close)
        .into_iter()
        .map(|(s, e)| {
            let code: String = decl[s..e].lines().map(|l| l.split_once("//").map_or(l, |(c, _)| c)).collect::<Vec<_>>().join(" ");
            strip_attrs(code.trim()).split_whitespace().collect::<Vec<_>>().join(" ")
        })
        .filter(|p| !p.is_empty())
        .collect();
    // The return type runs to the body, the `;` of a declaration or the `where` clause, outside brackets.
    let mut depth = 0i32;
    let mut ret_end = masked.len();
    for (k, &c) in mb.iter().enumerate().skip(close + 1) {
        match c {
            b'(' | b'[' | b'<' => depth += 1,
            b'>' if mb[k - 1] == b'-' => {}
            b')' | b']' | b'>' => depth -= 1,
            b'{' | b';' if depth == 0 => {
                ret_end = k;
                break;
            }
            b'w' if depth == 0
                && masked[k..].starts_with("where")
                && mb[k - 1].is_ascii_whitespace()
                && !mb.get(k + 5).is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_') =>
            {
                ret_end = k;
                break;
            }
            _ => {}
        }
    }
    let ret = decl[close + 1..ret_end].split_whitespace().collect::<Vec<_>>().join(" ");
    let generics = generics.map(|(s, e)| decl[s..e].split_whitespace().collect::<Vec<_>>().join(" ")).unwrap_or_default();
    Some(format!("fn {name}{generics}({}){}{ret}", params.join(", "), if ret.is_empty() { "" } else { " " }))
}

fn param(p: &str) -> Param {
    let core = p.trim_start_matches('&').trim_start();
    let core = if core.starts_with('\'') { core.split_once(' ').map_or(core, |(_, r)| r) } else { core };
    let core = core.trim_start_matches("mut ").trim_start();
    let is_self = core == "self" || core.starts_with("self:") || core.starts_with("self ");
    if is_self {
        let ty = p.split_once(':').map_or("", |(_, t)| t).trim();
        return Param { is_self, pat: p.to_string(), ty: ty.to_string() };
    }
    match type_colon(p) {
        Some(i) => Param { is_self, pat: p[..i].trim().to_string(), ty: p[i + 1..].trim().to_string() },
        None => Param { is_self, pat: p.to_string(), ty: String::new() },
    }
}

/// The `:` between a parameter's pattern and its type: the first one outside brackets that is not part of `::`.
fn type_colon(p: &str) -> Option<usize> {
    let b = p.as_bytes();
    let mut depth = 0i32;
    for (i, &c) in b.iter().enumerate() {
        match c {
            b'(' | b'[' | b'{' | b'<' => depth += 1,
            b')' | b']' | b'}' | b'>' => depth -= 1,
            b':' if depth == 0 && b.get(i + 1) != Some(&b':') && (i == 0 || b[i - 1] != b':') => return Some(i),
            _ => {}
        }
    }
    None
}

/// `p` without its leading `#[attr]`s.
fn strip_attrs(mut p: &str) -> &str {
    while p.starts_with("#[") {
        let mut depth = 0i32;
        let Some(close) = p.char_indices().find_map(|(i, c)| {
            match c {
                '[' => depth += 1,
                ']' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i);
                    }
                }
                _ => {}
            }
            None
        }) else {
            return "";
        };
        p = p[close + 1..].trim_start();
    }
    p
}

/// The byte index of the `>` closing the generics `text` opens with.
fn generics_end(text: &str) -> Option<usize> {
    let mut depth = 0i32;
    let mut prev = ' ';
    for (i, c) in text.char_indices() {
        match c {
            '<' => depth += 1,
            '>' if prev != '-' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        prev = c;
    }
    None
}

/// The type and const parameter names of a generics list (`'a, T: Clone, const N: usize` → `T`, `N`).
fn generic_names(list: &str) -> Vec<String> {
    top_level_split(list, 0, list.len())
        .into_iter()
        .filter_map(|(s, e)| {
            let g = list[s..e].trim();
            let g = g.strip_prefix("const ").unwrap_or(g).trim_start();
            let name: String = g.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
            (!name.is_empty() && !g.starts_with('\'')).then_some(name)
        })
        .collect()
}

fn matching_paren(b: &[u8]) -> Option<usize> {
    let mut depth = 0i32;
    for (k, &c) in b.iter().enumerate() {
        match c {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                depth -= 1;
                if depth == 0 {
                    return (c == b')').then_some(k);
                }
            }
            _ => {}
        }
    }
    None
}

/// Byte ranges of the comma-separated entries of `m[from..to]` outside brackets and generics.
fn top_level_split(m: &str, from: usize, to: usize) -> Vec<(usize, usize)> {
    let b = m.as_bytes();
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = from;
    for k in from..to {
        match b[k] {
            b'(' | b'[' | b'{' | b'<' => depth += 1,
            b'>' if k > from && b[k - 1] == b'-' => {}
            b')' | b']' | b'}' | b'>' => depth -= 1,
            b',' if depth == 0 => {
                out.push((start, k));
                start = k + 1;
            }
            _ => {}
        }
    }
    if start < to {
        out.push((start, to));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_params_from_declarations() {
        let src = "impl S {\n    /// doc\n    pub fn push_str(&mut self, string: &str) {}\n    fn push_str_x(&self) {}\n}\n";
        let p = params_in(src, 3, "push_str").unwrap();
        assert_eq!(p.params.len(), 2);
        assert!(p.params[0].is_self && p.params[0].pat == "&mut self");
        assert_eq!((p.params[1].pat.as_str(), p.params[1].ty.as_str()), ("string", "&str"));
        assert_eq!(params_in(src, 1, "push_str_x").unwrap().params.len(), 1);

        let src = "fn from_iter<I: IntoIterator<Item = (K, V)>, const N: usize>(#[allow(x)] mut iter: I, (a, b): (u8, u8), _: Foo, f: impl Fn(u8) -> u8) {}";
        let p = params_in(src, 1, "from_iter").unwrap();
        assert_eq!(p.generics, ["I", "N"]);
        let shown: Vec<(&str, Option<&str>, &str)> = p.params.iter().map(|q| (q.pat.as_str(), q.binding(), q.ty.as_str())).collect();
        assert_eq!(shown, [("mut iter", Some("iter"), "I"), ("(a, b)", None, "(u8, u8)"), ("_", None, "Foo"), ("f", Some("f"), "impl Fn(u8) -> u8")]);

        let p = params_in("fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) {}", 1, "poll").unwrap();
        assert!(p.params[0].is_self && p.params[0].ty == "Pin<&mut Self>");
        let p = params_in("fn get<'a>(&'a self, k: &std::path::Path) {}", 1, "get").unwrap();
        assert!(p.params[0].is_self && p.params[1].ty == "&std::path::Path");
    }

    #[test]
    fn test_a_stale_index_line_finds_the_nearest_declaration() {
        let src =
            "struct S {\n    /// The id.\n    pub id: u32,\n    idx: u8,\n}\n// moved\n// moved\nimpl S {\n    pub(crate) fn id(&self) -> u32 { self.id }\n}\nfn identity() {}\n";
        assert_eq!(decl_line_near(src, 9, "id"), Some(9), "an exact line is kept");
        assert_eq!(decl_line_near(src, 7, "id"), Some(9), "the method two lines below the stale line");
        assert_eq!(decl_line_near(src, 2, "id"), Some(3), "the field, not `idx` or `self.id`");
        assert_eq!(decl_line_near(src, 1, "nothing"), None);
        assert_eq!(params_in(src, 6, "id").map(|p| p.params.len()), Some(1));
    }

    #[test]
    fn test_signature_labels_as_rust_analyzer() {
        let src = "/// Doc.\npub(crate) fn params_in(text: &str, line: u32, name: &str) -> Option<FnParams> {\n    None\n}\n";
        assert_eq!(signature_in(src, 2, "params_in").as_deref(), Some("fn params_in(text: &str, line: u32, name: &str) -> Option<FnParams>"));
        let src = "    pub fn push_str(\n        &mut self,\n        string: &str, // the tail\n    ) {}\n";
        assert_eq!(signature_in(src, 1, "push_str").as_deref(), Some("fn push_str(&mut self, string: &str)"));
        let src = "fn map<U, F>(self, f: F) -> Option<U>\nwhere\n    F: FnOnce(T) -> U,\n{\n";
        assert_eq!(signature_in(src, 1, "map").as_deref(), Some("fn map<U, F>(self, f: F) -> Option<U>"));
        let src = "fn call(&self, f: impl Fn(u8) -> Vec<u8>) -> Result<(), E>;\n";
        assert_eq!(signature_in(src, 1, "call").as_deref(), Some("fn call(&self, f: impl Fn(u8) -> Vec<u8>) -> Result<(), E>"));
    }
}
