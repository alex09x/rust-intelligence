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
    /// format string capturing one (`"{N}"`: review of c5dee55b).
    pub names_one: bool,
    /// For a file the index read: a mention under `cfg` or `cfg_attr`, which another configuration of the same bytes
    /// compiles (a feature turned on in the manifest: review of c5dee55b).
    pub gated_mention: bool,
    /// The macros it calls.
    pub calls: Vec<String>,
    /// Its `macro_rules!`: name, whether the body names one, the macros the body calls.
    pub defines: Vec<(String, bool, Vec<String>)>,
    /// Its renames `a as b` (`use w::call_it as run`), which call a macro by another name.
    pub aliases: Vec<(String, String)>,
    /// The files it includes (`include!("body.inc")`), as written.
    pub includes: Vec<String>,
}

/// The scan of `text` for `names`; `indexed`: the index read it, so a mention it compiled is in the index already and
/// only a gated one is looked for, outside `own` (the bytes of the item being deleted, in its own file).
pub(super) fn scan(text: &str, names: &[&str], indexed: bool, own: Option<&Range<usize>>) -> Scan {
    let toks: Vec<_> = ridx_syntax::lex(text).into_iter().filter(|t| !t.is_trivia()).collect();
    let ident = |i: usize| toks.get(i).filter(|t| matches!(t.kind, T::Ident | T::RawIdent)).map(|t| ridx_syntax::sema_fn::norm_ident(t.text(text)));
    let txt = |i: usize| toks.get(i).map_or("", |t| t.text(text));
    let names_word = |w: &str| names.contains(&w) || !w.is_ascii();
    let captures = |s: &str| names.iter().any(|n| s.contains(&format!("{{{n}}}")) || s.contains(&format!("{{{n}:")));
    let mut s = Scan { names_one: false, gated_mention: false, calls: Vec::new(), defines: Vec::new(), aliases: Vec::new(), includes: Vec::new() };
    let mut mentions = Vec::new();
    for i in 0..toks.len() {
        if toks[i].kind.is_literal() && captures(txt(i)) {
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
            let (mut depth, mut naming, mut calls) = (0i32, false, Vec::new());
            for (j, tok) in toks.iter().enumerate().skip(i + 3) {
                match txt(j) {
                    "(" | "[" | "{" => depth += 1,
                    ")" | "]" | "}" => depth -= 1,
                    _ => {
                        naming |= ident(j).is_some_and(names_word) || (tok.kind.is_literal() && captures(txt(j)));
                        if txt(j + 1) == "!" {
                            calls.extend(ident(j).map(str::to_string));
                        }
                    }
                }
                if depth <= 0 {
                    break;
                }
            }
            s.defines.push((m.to_string(), naming, calls));
        } else if matches!(txt(i + 2), "(" | "[" | "{") {
            s.calls.push(w.to_string());
            if matches!(w, "include" | "include_str" | "include_bytes") && toks.get(i + 3).is_some_and(|t| t.kind.is_literal()) {
                s.includes.push(txt(i + 3).trim_start_matches('r').trim_matches('#').trim_matches('"').to_string());
            }
        }
    }
    if indexed && !mentions.is_empty() {
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
    }
    s
}
