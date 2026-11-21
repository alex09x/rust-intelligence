/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! `ridx/expandMacro` (prod-code, m=20257): the `macro_rules!` call around the cursor and its expansion as source,
//! fully expanded, or `None` wherever ridx cannot tell the expansion is the one the compiler makes.

use super::action_inline_macro::{crate_prefix, macro_definition};
use super::action_inline_macro_text::{rejoin_punctuation, tidy_spacing};
use super::action_parse::{line_starts_of, mask_comments_and_strings};
use ridx_format::{DefKind, WorkspaceIndex};
use ridx_syntax::name_res::mbe_parse::MacroRulesDef;
use ridx_syntax::{SyntaxKind as K, SyntaxTree, Token, TokenKind as T};

/// Whether the delimiters of `toks` nest as they open, every one closed by its own kind (`{[(])}` is no token tree:
/// review of 29b77abe).
fn balanced(toks: &[Token]) -> bool {
    let mut stack = Vec::new();
    for t in toks {
        match t.kind {
            T::LParen | T::LBracket | T::LBrace => stack.push(t.kind),
            T::RParen | T::RBracket | T::RBrace => {
                let open = match t.kind {
                    T::RParen => T::LParen,
                    T::RBracket => T::LBracket,
                    _ => T::LBrace,
                };
                if stack.pop() != Some(open) {
                    return false;
                }
            }
            _ => {}
        }
    }
    stack.is_empty()
}

/// Whether every fragment a matcher of the definition `def` (its braces included) declares is one the text renders as
/// the compiler groups it: a token tree, an identifier or a lifetime. A literal may be `-2i32`, which `$x.abs()` takes
/// whole (review of 29b77abe). Only the matchers are read, a transcriber's `$x: u32` or `$t::MAX` being no fragment.
fn fragments_faithful(def: &str) -> bool {
    let toks: Vec<_> = ridx_syntax::lex(def).into_iter().filter(|t| !t.is_trivia()).collect();
    let txt = |i: usize| toks.get(i).map_or("", |t| t.text(def));
    let (mut depth, mut group, mut in_matcher) = (0i32, 0usize, false);
    for i in 0..toks.len() {
        match txt(i) {
            "(" | "[" | "{" => {
                depth += 1;
                // Inside the definition's braces, its groups alternate: matcher, transcriber.
                if depth == 2 {
                    in_matcher = group % 2 == 0;
                    group += 1;
                }
            }
            ")" | "]" | "}" => depth -= 1,
            "$" if in_matcher && depth >= 2 && txt(i + 2) == ":" && !matches!(txt(i + 3), "tt" | "ident" | "lifetime") => return false,
            _ => {}
        }
    }
    true
}

/// The definition the call named `name` at `name_at` resolves to: the index's own edge, when the index read this very
/// text (an edge of an older text is not this call's: review of 29b77abe), by the target's own name (a re-export's
/// differs); else, for an unqualified call, a `macro_rules!` of that name at the top of this file before it, which
/// is in scope there; nothing else (a same-named macro elsewhere is not known to be visible).
fn definition(doc: &str, tree: &SyntaxTree<'_>, file: &str, name_at: usize, name: &str, qualified: bool, ws: &WorkspaceIndex) -> Option<(String, Option<String>)> {
    let starts = line_starts_of(doc);
    let current = ws.source_hash_for_file(file) == Some(ridx_format::normalized_source_hash(doc.as_bytes()));
    let (line, col) = super::action_struct_conv_index::byte_to_line_col(doc, &starts, name_at);
    if current {
        if let Some(r) = ws.find_direct_references_at(file, line, col).filter(|r| r.target_kind == DefKind::Macro) {
            let (target, path, target_line) = (r.target_symbol.to_string(), r.target_resolved_path(), r.target_line);
            // The definition's file must be the text the index read too, or its coordinates name another macro; and its
            // line must hold one definition of that name, the line being all that is looked up (review of b40e1834).
            let text = std::fs::read_to_string(&path).ok()?;
            if ws.source_hash_for_file(&path.to_string_lossy()) != Some(ridx_format::normalized_source_hash(text.as_bytes())) {
                return None;
            }
            let on_line = text.lines().nth(target_line.saturating_sub(1) as usize).unwrap_or("");
            let def_toks: Vec<_> = ridx_syntax::lex(on_line).into_iter().filter(|t| !t.is_trivia()).collect();
            let defs = def_toks
                .windows(3)
                .filter(|w| w[0].text(on_line) == "macro_rules" && w[1].text(on_line) == "!" && ridx_syntax::sema_fn::norm_ident(w[2].text(on_line)) == target)
                .count();
            if defs > 1 {
                return None;
            }
            return macro_definition(doc, &starts, file, name_at, &target, ws, false);
        }
    }
    if qualified {
        return None;
    }
    // The one definition of that name in the file, at its top, before the call, under no attribute: a nearer one in a
    // block may shadow it, a `cfg` may compile it out (review of b40e1834).
    let same_name: Vec<_> = tree
        .descendants(tree.root())
        .filter(|&c| tree.kind(c) == K::MacroRules)
        .filter(|&c| tree.child(c, K::Name).is_some_and(|n| ridx_syntax::sema_fn::norm_ident(tree.text(n)) == name))
        .collect();
    let [def] = same_name.as_slice() else { return None };
    let gated = tree
        .children(*def)
        .filter(|&c| tree.kind(c) == K::Attr)
        .any(|a| tree.tokens_of(a).iter().any(|t| matches!(ridx_syntax::sema_fn::norm_ident(t.text(doc)), "cfg" | "cfg_attr")));
    if tree.parent(*def) != Some(tree.root()) || tree.range(*def).end > name_at || gated {
        return None;
    }
    let body = tree.child(*def, K::TokenTree)?;
    Some((tree.text(body).to_string(), None))
}

/// The name of the `macro_rules!` macro called around `cursor` and its expansion as source: fully expanded, so `None`
/// when it calls a macro again, and for a macro ridx does not expand (the standard library's, a proc macro).
pub fn expand_macro_at(doc: &str, file: &str, cursor: usize, ws: &WorkspaceIndex) -> Option<(String, String)> {
    let masked = mask_comments_and_strings(doc);
    // The call around the cursor as the syntax tree has it: trivia before the `!`, a name beyond ASCII (review of
    // d2cb318a).
    let tree = ridx_syntax::parse(doc);
    let c = tree.covering(cursor);
    let mc = std::iter::once(c).chain(tree.ancestors(c)).find(|&n| tree.kind(n) == K::MacroCall)?;
    let toks: Vec<_> = tree.tokens_of(mc).iter().filter(|t| !t.is_trivia()).copied().collect();
    let bang = toks.iter().position(|t| t.kind == T::Bang)?;
    let name_at = toks[..bang].iter().rposition(|t| matches!(t.kind, T::Ident | T::RawIdent))?;
    let name_tok = toks[name_at];
    let tt = tree.child(mc, K::TokenTree)?;
    // Delimited arguments, closed by the delimiter that opened them and nested as they open: the parser's recovery
    // makes a token tree of `id!(12` and of `{[(])}` too (reviews of 94c450dc and 29b77abe).
    let tt_toks: Vec<_> = tree.tokens_of(tt).iter().filter(|t| !t.is_trivia()).copied().collect();
    let closed = match (tt_toks.first(), tt_toks.last()) {
        (Some(o), Some(c)) if tt_toks.len() >= 2 => matches!((o.kind, c.kind), (T::LParen, T::RParen) | (T::LBracket, T::RBracket) | (T::LBrace, T::RBrace)),
        _ => false,
    };
    if !closed || !balanced(&tt_toks) {
        return None;
    }
    let (open, close) = (tt_toks[0].start as usize, tt_toks[tt_toks.len() - 1].end as usize);
    let name = &doc[name_tok.start as usize..name_tok.end as usize];
    if name == "macro_rules" || cursor > close {
        return None;
    }
    let (def_text, def_crate) = definition(doc, &tree, file, name_tok.start as usize, ridx_syntax::sema_fn::norm_ident(name), name_at > 0, ws)?;
    if !fragments_faithful(&def_text) {
        return None;
    }
    let def = MacroRulesDef::parse_body(&def_text)?;
    // The path as its significant tokens spell it (`crate /* c */ ::m!` is `crate::`: review of 94c450dc).
    let call_path: String = toks[..name_at].iter().map(|t| t.text(doc)).collect();
    let prefix = crate_prefix(ws, file, &masked, &call_path, def_crate.as_deref())?;
    let expansion = tidy_spacing(&rejoin_punctuation(&def.expand(&doc[open + 1..close - 1], &prefix)?)).trim().to_string();
    // Fully expanded: no call `name!(..)` left, `r#name!` included; `a != b` is none.
    let toks: Vec<_> = ridx_syntax::lex(&expansion).into_iter().filter(|t| !t.is_trivia()).collect();
    let calls_again = toks.windows(3).any(|w| matches!(w[0].kind, T::Ident | T::RawIdent) && w[1].kind == T::Bang && matches!(w[2].kind, T::LParen | T::LBracket | T::LBrace));
    (!expansion.is_empty() && !calls_again).then(|| (name.to_string(), expansion))
}
