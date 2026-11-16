/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Primitive types (`usize`, `str`, `f64`, …). No item declares them, so the index keeps no definition and a click on
//! `usize` in Zed did nothing. As in rust-analyzer, a definition goes to their documentation module in `core`
//! (`mod prim_usize {}` in `core/src/primitive_docs.rs`) and hover shows its docs.

use crate::lsp::protocol::path_to_uri;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

const PRIMITIVES: &[&str] = &["bool", "char", "str", "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128", "isize", "f16", "f32", "f64", "f128"];

/// Where a primitive's documentation module is and what it says.
#[derive(Clone)]
pub(crate) struct PrimitiveDoc {
    file: PathBuf,
    /// 0-based line of `mod prim_<name> {}`, and the columns of `prim_<name>` on it.
    line: u32,
    start: u32,
    end: u32,
    docs: String,
}

/// The primitive type named at the 1-based character column `col` of `line`: not a field or method (`x.u8`), a path
/// segment (`m::u8`), a macro or a call.
pub(crate) fn primitive_at(line: &str, col: u32) -> Option<&'static str> {
    let chars: Vec<char> = line.chars().collect();
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let at = (col.saturating_sub(1) as usize).min(chars.len().checked_sub(1)?);
    if !is_word(chars[at]) {
        return None;
    }
    let (mut s, mut e) = (at, at + 1);
    while s > 0 && is_word(chars[s - 1]) {
        s -= 1;
    }
    while e < chars.len() && is_word(chars[e]) {
        e += 1;
    }
    let word: String = chars[s..e].iter().collect();
    let before = chars[..s].iter().rev().find(|c| !c.is_whitespace());
    let path_segment = s >= 2 && chars[s - 2] == ':' && chars[s - 1] == ':';
    let rest: String = chars[e..].iter().collect::<String>().trim_start().to_string();
    // A field (`u8: u8`, `M { u8: 1 }`) is followed by one `:`, a type's associated item (`f32::MAX`) by two.
    let field = rest.starts_with(':') && !rest.starts_with("::");
    // Review of 19d427d9: a raw identifier (`r#str`) names no primitive, nor does a word in a string or comment.
    if matches!(before, Some('.' | '#')) || path_segment || field || rest.starts_with(['!', '(']) || in_string_or_comment(&chars, s) {
        return None;
    }
    PRIMITIVES.iter().find(|p| **p == word).copied()
}

/// Whether character `at` of a line lies in a string literal, a `//` comment or a block comment's continuation line.
fn in_string_or_comment(chars: &[char], at: usize) -> bool {
    // A block comment's continuation line (` * text`, ` */`), not a pointer type or a dereference (`*const u8`, `*p`)
    // (review of be9cc081).
    let first = chars.iter().position(|c| !c.is_whitespace()).unwrap_or(0);
    if chars.get(first) == Some(&'*') && chars.get(first + 1).is_none_or(|c| c.is_whitespace()) {
        return true;
    }
    // A line opening with `*/` ends a comment begun above; what follows it is code.
    let (mut in_str, mut in_block) = (false, chars[first..].starts_with(&['*', '/']));
    let mut k = 0;
    while k < at {
        let next = chars.get(k + 1).copied();
        match chars[k] {
            '*' if in_block && next == Some('/') => (in_block, k) = (false, k + 1),
            _ if in_block => {}
            '\\' if in_str => k += 1,
            '"' => in_str = !in_str,
            '/' if !in_str && next == Some('/') => return true,
            '/' if !in_str && next == Some('*') => (in_block, k) = (true, k + 1),
            // A char literal (`'"'`, `'\''`) holds no string; a lifetime (`'a`) is left as it is.
            '\'' if !in_str && next == Some('\\') => k = (k + 2..chars.len()).find(|&j| chars[j] == '\'').unwrap_or(k),
            '\'' if !in_str && chars.get(k + 2) == Some(&'\'') => k += 2,
            _ => {}
        }
        k += 1;
    }
    in_str || in_block
}

/// The documentation of primitive `name` in `primitive_docs.rs` (`text`): the `///` lines above `mod prim_<name> {}`,
/// past its attributes.
fn parse_primitive_doc(text: &str, name: &str) -> Option<(u32, u32, u32, String)> {
    let lines: Vec<&str> = text.lines().collect();
    let module = format!("mod prim_{name} {{}}");
    let at = lines.iter().position(|l| l.trim() == module)?;
    let start = lines[at].find("prim_")? as u32;
    let mut docs = Vec::new();
    for l in lines[..at].iter().rev().map(|l| l.trim()) {
        if let Some(d) = l.strip_prefix("///") {
            docs.push(d.strip_prefix(' ').unwrap_or(d));
        } else if l.starts_with("#[rustc_doc_primitive") || !(l.starts_with("#[") || l.starts_with("//")) {
            // The docs start below the primitive's own attribute; a blank line or code ends them as well. A plain
            // comment inside them does not (review of 19d427d9: `f32`'s `// Sorted alphabetically` cut its docs).
            break;
        }
    }
    docs.reverse();
    Some((at as u32, start, start + "prim_".len() as u32 + name.len() as u32, docs.join("\n")))
}

/// `name`'s documentation module in the standard library sources the index was built from, read once per name.
pub(crate) fn primitive_doc(name: &str) -> Option<PrimitiveDoc> {
    static CACHE: OnceLock<Mutex<HashMap<String, Option<PrimitiveDoc>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(d) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(name) {
        return d.clone();
    }
    let doc = super::type_definition_parse::sysroot_rust_src().and_then(|root| {
        let file = root.join("library/core/src/primitive_docs.rs");
        let text = std::fs::read_to_string(&file).ok()?;
        let (line, start, end, docs) = parse_primitive_doc(&text, name)?;
        Some(PrimitiveDoc { file, line, start, end, docs })
    });
    cache.lock().unwrap_or_else(|e| e.into_inner()).insert(name.to_string(), doc.clone());
    doc
}

/// The location of the primitive named at `col` of `line`, or `None`.
pub(crate) fn definition(line: &str, col: u32) -> Option<Value> {
    let d = primitive_doc(primitive_at(line, col)?)?;
    Some(json!({
        "uri": path_to_uri(&d.file.to_string_lossy()),
        "range": { "start": { "line": d.line, "character": d.start }, "end": { "line": d.line, "character": d.end } }
    }))
}

/// The hover of the primitive named at `col` of `line`, rust-analyzer's: its name, then its documentation.
pub(crate) fn hover(line: &str, col: u32) -> Option<Value> {
    let name = primitive_at(line, col)?;
    let docs = primitive_doc(name).map(|d| d.docs).filter(|d| !d.is_empty());
    let value = match docs {
        Some(d) => format!("```rust\n{name}\n```\n\n---\n\n{d}"),
        None => format!("```rust\n{name}\n```"),
    };
    Some(json!({ "contents": { "kind": "markdown", "value": value } }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_a_primitive_is_told_from_names_spelled_alike() {
        let line = "    pub samples: usize, x: Vec<u8>, s: &str, n: x.u8, m::u8, f32::MAX, u8!(1), u8(2), usizes";
        let col = |needle: &str, k: usize| line.find(needle).unwrap() as u32 + 1 + k as u32;
        assert_eq!(primitive_at(line, col("usize,", 2)), Some("usize"));
        assert_eq!(primitive_at(line, col("u8>", 0)), Some("u8"));
        assert_eq!(primitive_at(line, col("str,", 1)), Some("str"));
        assert_eq!(primitive_at(line, col("f32::", 0)), Some("f32"), "an associated item's type");
        assert_eq!(primitive_at(line, col("u8,", 0)), None, "a field");
        assert_eq!(primitive_at(line, col("m::u8", 3)), None, "a path segment");
        assert_eq!(primitive_at(line, col("u8!", 0)), None, "a macro");
        assert_eq!(primitive_at(line, col("u8(2", 0)), None, "a call");
        assert_eq!(primitive_at(line, col("usizes", 1)), None);
        assert_eq!(primitive_at(line, col("samples", 1)), None);

        // Review of 19d427d9.
        let line = r#"    pub u8: u8, M { u8: 1 }, r#str, x.r#char, "usize", s.f("a\"bool"), y // a usize"#;
        let col = |needle: &str, k: usize| line.find(needle).unwrap() as u32 + 1 + k as u32;
        assert_eq!(primitive_at(line, col("u8: u8", 0)), None, "a field's declaration");
        assert_eq!(primitive_at(line, col("u8: u8", 4)), Some("u8"), "its type");
        assert_eq!(primitive_at(line, col("u8: 1", 0)), None, "a field in a literal");
        assert_eq!(primitive_at(line, col("str,", 0)), None, "a raw identifier");
        assert_eq!(primitive_at(line, col("char", 0)), None);
        assert_eq!(primitive_at(line, col("usize\"", 0)), None, "a string");
        assert_eq!(primitive_at(line, col("bool", 0)), None, "a string past an escaped quote");
        assert_eq!(primitive_at(line, line.rfind("usize").unwrap() as u32 + 1), None, "a comment");
        assert_eq!(primitive_at(" * the usize of a block comment", 10), None);
        assert_eq!(primitive_at("let s: &str = \"\";", 9), Some("str"), "a string after it is no matter");

        // Review of be9cc081.
        let first = |line: &str, needle: &str| primitive_at(line, line.find(needle).unwrap() as u32 + 1);
        assert_eq!(first("    *const u8,", "u8"), Some("u8"), "a pointer type");
        assert_eq!(first("    *p as usize", "usize"), Some("usize"), "a dereference");
        assert_eq!(first("/* tag */ pub n: usize", "usize"), Some("usize"), "past a closed block comment");
        assert_eq!(first("let a = 1; /* the usize */", "usize"), None, "inside one");
        assert_eq!(first(" */ usize", "usize"), Some("usize"), "code after a block comment's end");
        assert_eq!(first(" * see usize", "usize"), None, "a block comment's continuation line");
        assert_eq!(first("let q = '\"'; let n: usize = 1;", "usize"), Some("usize"), "a char literal holds no string");
        assert_eq!(first("let q = '\\''; let n: usize = 1;", "usize"), Some("usize"));
        assert_eq!(first("fn f<'a>(x: &'a str) {}", "str"), Some("str"), "a lifetime");
    }

    #[test]
    fn test_primitive_docs_are_read_from_core() {
        let text = "#[rustc_doc_primitive = \"isize\"]\n//\n/// The signed one.\n#[stable(feature = \"rust1\", since = \"1.0.0\")]\nmod prim_isize {}\n\n\
                    #[rustc_doc_primitive = \"usize\"]\n//\n/// The pointer-sized unsigned integer type.\n///\n/// The size of this primitive.\n#[stable(feature = \"rust1\", since = \"1.0.0\")]\nmod prim_usize {}\n";
        let (line, start, end, docs) = parse_primitive_doc(text, "usize").unwrap();
        assert_eq!((line, start, end), (12, 4, 14), "`prim_usize` on its line");
        assert_eq!(docs, "The pointer-sized unsigned integer type.\n\nThe size of this primitive.");
        assert_eq!(parse_primitive_doc(text, "isize").unwrap().3, "The signed one.");
        assert!(parse_primitive_doc(text, "u8").is_none());
    }
}
