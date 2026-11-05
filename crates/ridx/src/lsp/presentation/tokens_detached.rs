/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Tokens for a file no crate includes (a trybuild `ui` test, a stray script). rust-analyzer analyses no such file
//! semantically: it types definition sites by their syntax and leaves uses, imports and attribute arguments
//! unresolved. These tokens do the same, from the document-symbol parse.

use super::tokens::{
    RawToken, MOD_DEFINITION, TOKEN_TYPE_ENUM, TOKEN_TYPE_ENUM_MEMBER, TOKEN_TYPE_FUNCTION, TOKEN_TYPE_INTERFACE, TOKEN_TYPE_MACRO, TOKEN_TYPE_METHOD, TOKEN_TYPE_NAMESPACE,
    TOKEN_TYPE_PROPERTY, TOKEN_TYPE_STRUCT, TOKEN_TYPE_TYPE, TOKEN_TYPE_VARIABLE,
};
use serde_json::Value;

/// Token type for an LSP `SymbolKind`; `None` for kinds with no definition-site token (impl blocks).
fn symbol_kind_token(kind: u64) -> Option<u32> {
    Some(match kind {
        2 => TOKEN_TYPE_NAMESPACE,
        5 | 23 => TOKEN_TYPE_STRUCT,
        6 => TOKEN_TYPE_METHOD,
        8 => TOKEN_TYPE_PROPERTY,
        10 => TOKEN_TYPE_ENUM,
        11 => TOKEN_TYPE_INTERFACE,
        12 => TOKEN_TYPE_FUNCTION,
        13 | 14 => TOKEN_TYPE_VARIABLE,
        22 => TOKEN_TYPE_ENUM_MEMBER,
        26 => TOKEN_TYPE_TYPE,
        _ => return None,
    })
}

fn collect(symbols: &Value, lines: &[&str], out: &mut Vec<RawToken>) {
    for s in symbols.as_array().into_iter().flatten() {
        let sel = &s["selectionRange"];
        let (line, col, end_line, end_col) = (sel["start"]["line"].as_u64(), sel["start"]["character"].as_u64(), sel["end"]["line"].as_u64(), sel["end"]["character"].as_u64());
        if let (Some(line), Some(col), Some(end_line), Some(end_col), Some(mut tt)) = (line, col, end_line, end_col, symbol_kind_token(s["kind"].as_u64().unwrap_or(0))) {
            // `macro_rules! name` is listed as a function; its name is a macro.
            if tt == TOKEN_TYPE_FUNCTION && lines.get(line as usize).is_some_and(|l| l.contains("macro_rules!")) {
                tt = TOKEN_TYPE_MACRO;
            }
            if end_line == line && end_col > col {
                let tm = if matches!(tt, TOKEN_TYPE_FUNCTION | TOKEN_TYPE_METHOD) { MOD_DEFINITION } else { 0 };
                out.push(RawToken { line: line as u32, col: col as u32, length: (end_col - col) as u32, token_type: tt, token_modifiers: tm });
            }
        }
        collect(&s["children"], lines, out);
    }
}

/// Whether an unindexed file lies outside every crate by Cargo's layout: not its package's build script, not under its
/// package's `src/` (where cfg-disabled modules of the crate live), and not an auto-discovered target root
/// (`examples/x.rs`, `examples/x/main.rs`, likewise `tests/` and `benches/`).
pub(super) fn outside_every_crate(file: &str) -> bool {
    let path = std::path::Path::new(file);
    let Some(pkg) = path.ancestors().skip(1).find(|d| d.join("Cargo.toml").is_file()) else {
        return true;
    };
    let Ok(rel) = path.strip_prefix(pkg) else { return true };
    // A package that declares its own `[workspace]` inside another workspace (a `fuzz/` crate) is a separate
    // workspace the editor's workspace does not load.
    let manifest = std::fs::read_to_string(pkg.join("Cargo.toml")).unwrap_or_default();
    let declares_workspace = |m: &str| m.lines().any(|l| l.trim() == "[workspace]");
    if declares_workspace(&manifest) && pkg.ancestors().skip(1).any(|d| std::fs::read_to_string(d.join("Cargo.toml")).is_ok_and(|m| declares_workspace(&m))) {
        return true;
    }
    let parts: Vec<&str> = rel.iter().filter_map(|p| p.to_str()).collect();
    let target_root = match parts.as_slice() {
        [dir, _] => matches!(*dir, "examples" | "tests" | "benches"),
        [dir, _, "main.rs"] => matches!(*dir, "examples" | "tests" | "benches"),
        _ => false,
    };
    if rel == std::path::Path::new("build.rs") || rel.starts_with("src") || target_root {
        return false;
    }
    // A target the manifest declares (`[[example]] path = "examples/tutorial/01.rs"`).
    let rel_str = parts.join("/");
    !manifest.lines().any(|l| l.split_once('=').is_some_and(|(k, v)| k.trim() == "path" && v.trim().trim_matches(|c| c == '"' || c == '\'') == rel_str))
}

/// Whether the file switches itself off with an inner `#![cfg(…)]`: when the index did not compile it, rust-analyzer
/// treats its code as inactive and resolves none of its names.
pub(super) fn gated_off_by_inner_cfg(text: &str) -> bool {
    let masked = crate::lsp::editing::action_parse::mask_comments_and_strings(text);
    masked
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .take_while(|l| l.starts_with("#!"))
        .any(|l| l.trim_start_matches("#!").trim_start().trim_start_matches('[').trim_start().starts_with("cfg("))
}

/// Local bindings a definition site introduces on one line: `let` patterns, closure parameters, and function
/// parameters (`name: Type` inside a `fn` signature).
fn local_binding_tokens(scan: &str, line: &str, line_idx: u32, out: &mut Vec<RawToken>) {
    let b = scan.as_bytes();
    let is_id = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut push = |s: usize, e: usize| {
        let col = super::tokens_scan::byte_to_utf16_col(line, s);
        out.push(RawToken { line: line_idx, col, length: super::tokens_scan::byte_to_utf16_col(line, e) - col, token_type: TOKEN_TYPE_VARIABLE, token_modifiers: 0 });
    };
    // Lowercase identifiers of a pattern in `[from, to)`, skipping `mut`/`ref`, paths and constructor names.
    let pattern = |from: usize, to: usize, push: &mut dyn FnMut(usize, usize)| {
        let mut i = from;
        while i < to {
            if is_id(b[i]) && (i == 0 || !is_id(b[i - 1])) {
                let s = i;
                while i < to && is_id(b[i]) {
                    i += 1;
                }
                let w = &scan[s..i];
                let next = scan[i..to].trim_start();
                let first = w.as_bytes()[0];
                if (first.is_ascii_lowercase() || first == b'_')
                    && w != "_"
                    && !matches!(w, "mut" | "ref")
                    && !next.starts_with("::")
                    && !next.starts_with('(')
                    && !next.starts_with('{')
                {
                    push(s, i);
                }
                continue;
            }
            i += 1;
        }
    };
    let mut push_dyn = |s: usize, e: usize| push(s, e);
    for (at, _) in scan.match_indices("let ") {
        if at > 0 && is_id(b[at - 1]) {
            continue;
        }
        let end = scan[at..].find(['=', ':', ';']).map_or(scan.len(), |e| at + e);
        pattern(at + 4, end, &mut push_dyn);
    }
    // Closure parameters: `|a, b: T|`, not `||` or a `|` operator after an operand.
    let mut i = 0;
    while let Some(p) = scan[i..].find('|').map(|p| i + p) {
        let before = scan[..p].trim_end();
        let opens = before.is_empty() || before.ends_with(['(', ',', '=', '{', ';']) || before.ends_with("move") || before.ends_with("async");
        let Some(close) = scan[p + 1..].find('|').map(|c| p + 1 + c) else { break };
        if opens && close > p + 1 {
            let mut start = p + 1;
            for part in scan[p + 1..close].split(',') {
                let name_end = part.find(':').unwrap_or(part.len());
                pattern(start, start + name_end, &mut push_dyn);
                start += part.len() + 1;
            }
        }
        i = close + 1;
    }
    // Function parameters on the `fn` line.
    if let Some(f) = scan.find("fn ") {
        if let Some(open) = scan[f..].find('(').map(|o| f + o) {
            let close = scan[open..].find(')').map_or(scan.len(), |c| open + c);
            let mut start = open + 1;
            for part in scan[open + 1..close].split(',') {
                if let Some(colon) = part.find(':').filter(|&c| !part[c..].starts_with("::")) {
                    pattern(start, start + colon, &mut push_dyn);
                }
                start += part.len() + 1;
            }
        }
    }
}

/// Definition-site tokens of a file outside every crate.
pub(super) fn detached_definition_tokens(text: &str, out: &mut Vec<RawToken>) {
    let lines: Vec<&str> = text.lines().collect();
    collect(&crate::lsp::symbols::document_symbol_tree::document_symbols(text), &lines, out);
    let masked = crate::lsp::editing::action_parse::mask_comments_and_strings(text);
    for (idx, (line, scan)) in lines.iter().zip(masked.lines()).enumerate() {
        if line.len() == scan.len() {
            local_binding_tokens(scan, line, idx as u32, out);
        }
    }
}
