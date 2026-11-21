/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! What one source file says about possible uses of the names `ridx/safeDelete` would remove, read from its tokens
//! (and, for a file the index read, its syntax tree): enough to refuse, never to certify a use absent.

use ridx_syntax::{SyntaxKind as K, TokenKind as T};
use std::ops::Range;

/// A file's possible uses of the names.
pub(super) struct Scan {
    /// An identifier naming one (raw or not), any identifier beyond ASCII (it may be one in another normal form), or a
    /// string holding one as a word (a format capture `"{N}"`, a count `"{:>N$}"`, an escaped `"\u{7b}N}"`: reviews of
    /// c5dee55b and 53031a6e).
    pub names_one: bool,
    /// For a file the index read: a mention under `cfg` or `cfg_attr`, which another configuration of the same bytes
    /// compiles (a feature turned on in the manifest: review of c5dee55b).
    pub gated_mention: bool,
    /// The macros it calls, each with whether the call is under `cfg` (in a file the index read) and whether its
    /// arguments name one.
    pub calls: Vec<(String, bool, bool)>,
    /// Its `macro_rules!`: name, whether the body names one, whether the body holds a `cfg` (what it expands to depends
    /// on the configuration), the macros the body calls.
    pub defines: Vec<(String, bool, bool, Vec<String>)>,
    /// Its renames `a as b` (`use w::call_it as run`), which call a macro by another name.
    pub aliases: Vec<(String, String)>,
    /// The Rust sources it includes (`include!("body.inc")`), decoded; `include_str!` and `include_bytes!` are data.
    pub includes: Vec<String>,
    /// An `include!` whose path is no plain string (`include!(concat!(..))`): a source not known.
    pub unresolved_include: bool,
}

/// The text of the string literal `lit` as Rust reads it: raw ones as written, cooked ones with their escapes
/// decoded (`"body\u{2e}inc"` is `body.inc`: review of 53031a6e); `None` for another literal.
pub(super) fn string_value(lit: &str) -> Option<String> {
    if let Some(raw) = lit.strip_prefix('r') {
        let hashes = raw.len() - raw.trim_start_matches('#').len();
        return raw.get(hashes + 1..raw.len().checked_sub(hashes + 1)?).map(str::to_string);
    }
    // Rust reads a CRLF source as LF before lexing; a continuation skips only ASCII whitespace (a no-break space after
    // it is the path's: review of d8dcd7b1).
    let body = lit.strip_prefix('"')?.strip_suffix('"')?.replace("\r\n", "\n");
    let mut out = String::new();
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next()? {
            'n' => out.push('\n'),
            't' => out.push('\t'),
            'r' => out.push('\r'),
            '0' => out.push('\0'),
            '\\' => out.push('\\'),
            '"' => out.push('"'),
            '\'' => out.push('\''),
            'x' => {
                let h: String = chars.by_ref().take(2).collect();
                out.push(u8::from_str_radix(&h, 16).ok()? as char);
            }
            'u' => {
                let h: String = chars.by_ref().skip(1).take_while(|&c| c != '}').collect();
                out.push(char::from_u32(u32::from_str_radix(&h.replace('_', ""), 16).ok()?)?);
            }
            '\n' => {
                while chars.peek().is_some_and(|c| matches!(c, ' ' | '\t' | '\n' | '\r')) {
                    chars.next();
                }
            }
            _ => return None,
        }
    }
    Some(out)
}

/// The scan of `text` for `names`; `indexed`: the index read it, so a mention it compiled is in the index already and
/// only a gated one is looked for, outside `own` (the bytes of the item being deleted, in its own file).
pub(super) fn scan(text: &str, names: &[&str], context: &[&str], indexed: bool, own: Option<&Range<usize>>) -> Scan {
    let toks: Vec<_> = ridx_syntax::lex(text).into_iter().filter(|t| !t.is_trivia()).collect();
    let ident = |i: usize| toks.get(i).filter(|t| matches!(t.kind, T::Ident | T::RawIdent)).map(|t| ridx_syntax::sema_fn::norm_ident(t.text(text)));
    let txt = |i: usize| toks.get(i).map_or("", |t| t.text(text));
    let names_word = |w: &str| names.contains(&w) || !w.is_ascii();
    // A string naming one as a word, decoded or as written (what cannot be decoded is read as written).
    let words_name = |lit: &str| {
        let s = string_value(lit).unwrap_or_else(|| lit.to_string());
        s.split(|c: char| !(c.is_alphanumeric() || c == '_')).any(|w| names.contains(&w)) || lit.split(|c: char| !(c.is_alphanumeric() || c == '_')).any(|w| names.contains(&w))
    };
    let mut s = Scan { names_one: false, gated_mention: false, calls: Vec::new(), defines: Vec::new(), aliases: Vec::new(), includes: Vec::new(), unresolved_include: false };
    let mut mentions = Vec::new();
    let mut call_at = Vec::new();
    for i in 0..toks.len() {
        if toks[i].kind.is_literal() && words_name(txt(i)) {
            s.names_one = true;
            mentions.push(toks[i].start as usize);
        }
        let Some(w) = ident(i) else { continue };
        if names_word(w) {
            s.names_one = true;
            mentions.push(toks[i].start as usize);
        }
        if txt(i + 1) == "as" {
            if let Some(alias) = ident(i + 2) {
                s.aliases.push((w.to_string(), alias.to_string()));
            }
        }
        if txt(i + 1) != "!" {
            continue;
        }
        if w == "macro_rules" {
            // `macro_rules! m { .. }`: its body up to the delimiter closing the first one.
            let Some(m) = ident(i + 2) else { continue };
            let (mut depth, mut naming, mut gated, mut calls) = (0i32, false, false, Vec::new());
            for (j, tok) in toks.iter().enumerate().skip(i + 3) {
                match txt(j) {
                    "(" | "[" | "{" => depth += 1,
                    ")" | "]" | "}" => depth -= 1,
                    _ => {
                        naming |= ident(j).is_some_and(names_word) || (tok.kind.is_literal() && words_name(txt(j)));
                        gated |= ident(j).is_some_and(|w| w == "cfg" || w == "cfg_attr");
                        if txt(j + 1) == "!" {
                            calls.extend(ident(j).map(str::to_string));
                            // An include a macro expands is read from where it is called, which is not known here
                            // (review of d8dcd7b1).
                            if ident(j) == Some("include") {
                                s.unresolved_include = true;
                            }
                        }
                    }
                }
                if depth <= 0 {
                    break;
                }
            }
            s.defines.push((m.to_string(), naming, gated, calls));
        } else if matches!(txt(i + 2), "(" | "[" | "{") {
            // Whether the arguments name one: a macro may use them under a `cfg` its body holds (`gen!(unused)`).
            let mut depth = 0i32;
            let mut args_name = false;
            for (j, tok) in toks.iter().enumerate().skip(i + 2) {
                match txt(j) {
                    "(" | "[" | "{" => depth += 1,
                    ")" | "]" | "}" => depth -= 1,
                    _ => args_name |= ident(j).is_some_and(names_word) || (tok.kind.is_literal() && words_name(txt(j))),
                }
                if depth <= 0 {
                    break;
                }
            }
            s.calls.push((w.to_string(), false, args_name));
            call_at.push(toks[i].start as usize);
            if w == "include" {
                // Only a plain string, a trailing comma allowed, is a known path.
                let closes = |k: usize| matches!(txt(k), ")" | "]" | "}");
                let one = closes(i + 4) || (txt(i + 4) == "," && closes(i + 5));
                match toks.get(i + 3).filter(|t| t.kind.is_literal() && one).and_then(|t| string_value(t.text(text))) {
                    Some(path) => s.includes.push(path),
                    None => s.unresolved_include = true,
                }
            }
        }
    }
    if indexed && !(mentions.is_empty() && call_at.is_empty() && !text.contains("cfg")) {
        let tree = ridx_syntax::parse(text);
        let gated = |at: usize| {
            let n = tree.covering(at);
            std::iter::once(n).chain(tree.ancestors(n)).any(|a| {
                tree.children(a)
                    .filter(|&c| tree.kind(c) == K::Attr)
                    .any(|attr| tree.tokens_of(attr).iter().any(|t| matches!(ridx_syntax::sema_fn::norm_ident(t.text(text)), "cfg" | "cfg_attr")))
            })
        };
        s.gated_mention = mentions.iter().any(|&at| !own.is_some_and(|r| r.contains(&at)) && gated(at));
        // An import under `cfg` naming the item's module or the item may rebind an ungated call to it in another
        // configuration (`#[cfg(feature = "x")] use b as pick;`: review of d8dcd7b1).
        s.gated_mention |= tree.descendants(tree.root()).filter(|&d| tree.kind(d) == K::Use).any(|u| {
            gated(tree.range(u).start)
                && tree.tokens_of(u).iter().any(|t| matches!(t.kind, T::Ident | T::RawIdent) && context.contains(&ridx_syntax::sema_fn::norm_ident(t.text(text))))
        });
        for (c, &at) in s.calls.iter_mut().zip(&call_at) {
            c.1 = gated(at);
        }
    }
    s
}

#[cfg(test)]
#[path = "tests/safe_delete_scan_tests.rs"]
mod tests;
