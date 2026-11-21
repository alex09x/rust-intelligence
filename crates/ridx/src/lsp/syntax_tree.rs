/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! `ridx/syntaxTree {textDocument, range?}`: the document's syntax tree (its buffer's text when open) in the JSON
//! rust-analyzer's `rust-analyzer/viewSyntaxTree` returns, a string of nested `{"type": "Node" | "Token", "kind",
//! "start": [offset, line, col], "end": [...], "children": [...]}`, lossless, with rust-analyzer's kind names
//! (prod-code, m=20253: its introduce-variable and convert-to-method tools read it). Offsets are UTF-8 bytes, lines and
//! columns 0-based, columns in UTF-8 bytes, as rust-analyzer gives them.
//!
//! The tree is ridx's own parser's, in rust-analyzer's shape where its tools read it (review of 27bd4208): the wrappers
//! rust-analyzer has and ridx's tree does not are added (`STMT_LIST`, an unclosed one included, `NAME_REF`,
//! `MACRO_EXPR` and `MACRO_TYPE` around a macro call in an expression or a type), contextual keywords and joint
//! punctuation are named by their role (`MACRO_RULES_KW`; a closure's `||` is two `PIPE`), and the comments right
//! above an item are the item's, as rust-analyzer attaches them. Record and variant lists are not split into fields.
//! The tree is written with an explicit stack: no nesting depth exhausts the server's.

use super::syntax_tree_kinds::{contextual, node_kind, token_kind, JOINED};
use ridx_syntax::{NodeId, SyntaxKind as K, SyntaxTree, Token, TokenKind as T};
use std::fmt::Write;
use std::ops::Range;

/// What a node holds, in source order.
enum Item {
    Node(NodeId),
    /// One token, or several joint ones rust-analyzer has as one, with its kind.
    Token(usize, usize, &'static str),
    /// A `NAME_REF` around the token at that byte range.
    NameRef(usize, usize, &'static str),
    /// A node rust-analyzer has and ridx's tree does not (`STMT_LIST`, `MACRO_EXPR`), around these items.
    Wrapper(&'static str, Vec<Item>),
}

/// The bytes node `n` takes in rust-analyzer's tree: the whole text for the root, and for an item the comments right
/// above it too, which rust-analyzer attaches to it as its parser does (`n_attached_trivias`): up to an inner doc
/// comment (`//!`, `/*!`, its scope's), or a blank line, which outer documentation (`///`, `/**`) crosses (review of
/// d7ed2459).
fn range_of(tree: &SyntaxTree<'_>, n: NodeId) -> Range<usize> {
    if n == tree.root() {
        return 0..tree.src.len();
    }
    let r = tree.range(n);
    if !tree.kind(n).is_item() {
        return r;
    }
    let inner = |s: &str| s.starts_with("//!") || s.starts_with("/*!");
    let outer = |s: &str| (s.starts_with("///") && !s.starts_with("////")) || (s.starts_with("/**") && !s.starts_with("/***"));
    let mut start = r.start;
    let i = tree.tokens.partition_point(|t| (t.start as usize) < r.start);
    let mut before = tree.tokens[..i].iter().rev().peekable();
    while let Some(t) = before.next() {
        match t.kind {
            T::LineComment | T::BlockComment if inner(t.text(tree.src)) => break,
            T::LineComment | T::BlockComment => start = t.start as usize,
            T::Whitespace if !t.text(tree.src).contains("\n\n") => {}
            T::Whitespace if before.peek().is_some_and(|p| matches!(p.kind, T::LineComment | T::BlockComment) && outer(p.text(tree.src))) => {}
            _ => break,
        }
    }
    start..r.end
}

/// The wrapper rust-analyzer puts around the macro call `mc`: `MACRO_TYPE` in a type's place, none for an item or a
/// pattern, else `MACRO_EXPR`.
fn macro_wrapper(tree: &SyntaxTree<'_>, mc: NodeId) -> Option<&'static str> {
    let p = tree.parent(mc)?;
    let start = tree.range(mc).start;
    match tree.kind(p) {
        K::SourceFile | K::ItemList | K::MacroPat => None,
        // By the slot it fills (review of d7ed2459): a type alias's right side and a cast's target are types; an array
        // type's length, after its `;`, is a value.
        K::TypeAlias | K::CastExpr if tree.own_tokens(p).any(|(_, t)| matches!(t.kind, T::Eq | T::As) && (t.start as usize) < start) => Some("MACRO_TYPE"),
        K::ArrayType => Some(if tree.own_tokens(p).any(|(_, t)| t.kind == T::Semi && (t.start as usize) < start) { "MACRO_EXPR" } else { "MACRO_TYPE" }),
        K::TypeParam | K::RetType | K::Impl | K::TypeArg | K::RefType | K::PtrType | K::SliceType | K::TupleType | K::ParenType => Some("MACRO_TYPE"),
        // A type before the `=`, a value after it.
        K::LetStmt | K::Const | K::Static | K::TypeAlias | K::Param => {
            let eq_before = tree.own_tokens(p).any(|(_, t)| t.kind == T::Eq && (t.start as usize) < start);
            let colon_before = tree.own_tokens(p).any(|(_, t)| t.kind == T::Colon && (t.start as usize) < start);
            Some(if !eq_before && (colon_before || tree.kind(p) == K::TypeAlias) { "MACRO_TYPE" } else { "MACRO_EXPR" })
        }
        _ => Some("MACRO_EXPR"),
    }
}

/// The node `n`'s children and own tokens in source order, as rust-analyzer's tree has them.
fn items(tree: &SyntaxTree<'_>, n: NodeId) -> Vec<Item> {
    let kind = tree.kind(n);
    let toks: Vec<Token> = tree.own_tokens(n).map(|(_, t)| t).filter(|t| !t.is_trivia()).collect();
    let mut out: Vec<(usize, Item)> = Vec::new();
    for c in tree.children(n) {
        let item = match tree.kind(c) {
            K::MacroCall => match macro_wrapper(tree, c) {
                Some(w) => Item::Wrapper(w, vec![Item::Node(c)]),
                None => Item::Node(c),
            },
            _ => Item::Node(c),
        };
        out.push((range_of(tree, c).start, item));
    }
    // Whitespace and comments, which the tree's nodes do not hold: in the innermost node around them.
    let r = range_of(tree, n);
    let kids: Vec<Range<usize>> = tree.children(n).map(|c| range_of(tree, c)).collect();
    let lo = tree.tokens.partition_point(|t| (t.start as usize) < r.start);
    for t in tree.tokens[lo..].iter().take_while(|t| (t.start as usize) < r.end) {
        if t.is_trivia() && !kids.iter().any(|k| k.start <= t.start as usize && t.end as usize <= k.end) {
            out.push((t.start as usize, Item::Token(t.start as usize, t.end as usize, token_kind(t.kind))));
        }
    }
    let name_ref_owner = matches!(kind, K::PathSegment | K::FieldExpr | K::MethodCallExpr | K::RecordExprField | K::RecordPatField);
    // A closure's parameter list `||` is two delimiters, not a logical or.
    let joins = !matches!(kind, K::ClosureExpr | K::ParamList);
    let mut i = 0;
    while i < toks.len() {
        let t = toks[i];
        let mut joined = None;
        for (text, jk) in JOINED.iter().filter(|_| joins) {
            let n_chars = text.len();
            if i + n_chars <= toks.len() && (i..i + n_chars - 1).all(|j| toks[j].is_joint() && toks[j].end == toks[j + 1].start) {
                let s: String = toks[i..i + n_chars].iter().map(|t| t.text(tree.src)).collect();
                if s == *text {
                    joined = Some((n_chars, *jk));
                    break;
                }
            }
        }
        let word = t.text(tree.src);
        let own_kind = if t.kind == T::Ident { contextual(kind, word) } else { None };
        let (count, tk) = joined.unwrap_or((1, own_kind.unwrap_or_else(|| token_kind(t.kind))));
        let (start, end) = (t.start as usize, toks[i + count - 1].end as usize);
        let item = if name_ref_owner && count == 1 && matches!(t.kind, T::Ident | T::RawIdent | T::SelfKw | T::SelfType | T::Super | T::Crate | T::Int) {
            Item::NameRef(start, end, tk)
        } else {
            Item::Token(start, end, tk)
        };
        out.push((start, item));
        i += count;
    }
    out.sort_by_key(|(s, _)| *s);
    let mut list: Vec<Item> = out.into_iter().map(|(_, it)| it).collect();
    // A block's braces and statements are a `STMT_LIST` in rust-analyzer's tree, an unclosed block's too.
    if kind == K::BlockExpr {
        if let Some(o) = list.iter().position(|it| matches!(it, Item::Token(_, _, "L_CURLY"))) {
            let c = list.iter().rposition(|it| matches!(it, Item::Token(_, _, "R_CURLY"))).filter(|&c| c > o).unwrap_or(list.len() - 1);
            let inner: Vec<Item> = list.drain(o..=c).collect();
            list.insert(o, Item::Wrapper("STMT_LIST", inner));
        }
    }
    list
}

fn item_range(tree: &SyntaxTree<'_>, it: &Item) -> Range<usize> {
    match it {
        Item::Node(c) => range_of(tree, *c),
        Item::Token(s, e, _) | Item::NameRef(s, e, _) => *s..*e,
        Item::Wrapper(_, inner) => item_range(tree, &inner[0]).start..item_range(tree, inner.last().unwrap_or(&inner[0])).end,
    }
}

/// `[offset,line,col]` of byte `b` of `src`.
fn pos(src: &str, b: usize) -> String {
    let ls = src[..b].rfind('\n').map_or(0, |i| i + 1);
    format!("[{b},{},{}]", src[..ls].matches('\n').count(), b - ls)
}

/// Writes `root` and everything under it, with an explicit stack of the lists being written.
fn write_tree(tree: &SyntaxTree<'_>, root: Item, out: &mut String) {
    let src = tree.src;
    let mut stack: Vec<std::vec::IntoIter<Item>> = vec![vec![root].into_iter()];
    let mut first = vec![true];
    while let Some(top) = stack.last_mut() {
        let Some(it) = top.next() else {
            stack.pop();
            first.pop();
            if !stack.is_empty() {
                out.push_str("]}");
            }
            continue;
        };
        if let Some(f) = first.last_mut() {
            if !std::mem::replace(f, false) {
                out.push(',');
            }
        }
        let r = item_range(tree, &it);
        let (ps, pe) = (pos(src, r.start), pos(src, r.end));
        match it {
            Item::Token(_, _, k) => {
                let _ = write!(out, r#"{{"type":"Token","kind":"{k}","start":{ps},"end":{pe}}}"#);
            }
            Item::NameRef(_, _, k) => {
                let _ = write!(out, r#"{{"type":"Node","kind":"NAME_REF","start":{ps},"end":{pe},"children":[{{"type":"Token","kind":"{k}","start":{ps},"end":{pe}}}]}}"#);
            }
            Item::Node(c) => {
                let _ = write!(out, r#"{{"type":"Node","kind":"{}","start":{ps},"end":{pe},"children":["#, node_kind(tree, c));
                stack.push(items(tree, c).into_iter());
                first.push(true);
            }
            Item::Wrapper(k, inner) => {
                let _ = write!(out, r#"{{"type":"Node","kind":"{k}","start":{ps},"end":{pe},"children":["#);
                stack.push(inner.into_iter());
                first.push(true);
            }
        }
    }
}

/// The smallest item of the tree rust-analyzer's shape gives that covers `r`, from node `n` down.
fn smallest(tree: &SyntaxTree<'_>, n: NodeId, r: &Range<usize>) -> Item {
    // The node or synthetic node (a `NAME_REF`, a `STMT_LIST`) holding all of `r`, innermost on a tie (review of
    // 27bd4208: the parser's tree alone gave the segment around a `NAME_REF`).
    // A cursor (an empty range) is in a node from its start up to before its end, at the end of the text in what ends
    // there: `fn a() {}|fn b() {}` is in `b` (review of d7ed2459).
    let covers = |it: &Item| {
        let ir = item_range(tree, it);
        let holds =
            if r.is_empty() { ir.start <= r.start && (r.start < ir.end || (r.start == ir.end && ir.end == tree.src.len())) } else { ir.start <= r.start && r.end <= ir.end };
        !matches!(it, Item::Token(..)) && holds
    };
    let mut current = Item::Node(n);
    loop {
        let next = match &mut current {
            Item::Node(c) => items(tree, *c).into_iter().find(&covers),
            Item::Wrapper(_, inner) => inner.iter().position(&covers).map(|i| inner.swap_remove(i)),
            _ => None,
        };
        match next {
            Some(it) => current = it,
            None => return current,
        }
    }
}

/// The tree of `src` as rust-analyzer's JSON, from the smallest node covering the byte range `range` when given.
pub fn syntax_tree_json(src: &str, range: Option<Range<usize>>) -> String {
    let tree = ridx_syntax::parse(src);
    let root = match range {
        Some(r) => smallest(&tree, tree.root(), &r),
        None => Item::Node(tree.root()),
    };
    let mut out = String::new();
    write_tree(&tree, root, &mut out);
    out
}

#[cfg(test)]
#[path = "syntax_tree_tests.rs"]
mod tests;
