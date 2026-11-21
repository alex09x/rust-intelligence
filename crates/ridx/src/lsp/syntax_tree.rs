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
//! columns 0-based, columns in UTF-8 bytes, as rust-analyzer gives them. The tree is ridx's own parser's: the wrappers
//! rust-analyzer has and it does not (`STMT_LIST` in a block, `NAME_REF` around a name used) are added; the rest of
//! rust-analyzer's structure may differ (docs/integration.md §7). With `range`, the smallest node covering it.

use ridx_syntax::{NodeId, SyntaxKind as K, SyntaxTree, Token, TokenKind as T};
use std::fmt::Write;

/// rust-analyzer's name of a node kind: `PathType` is `PATH_TYPE`; an `ItemList` of an impl or a trait is an
/// `ASSOC_ITEM_LIST`, of an `extern` block an `EXTERN_ITEM_LIST`.
fn node_kind(tree: &SyntaxTree<'_>, n: NodeId) -> String {
    let k = tree.kind(n);
    if k == K::ItemList {
        match tree.parent(n).map(|p| tree.kind(p)) {
            Some(K::Impl | K::Trait) => return "ASSOC_ITEM_LIST".into(),
            Some(K::ExternBlock) => return "EXTERN_ITEM_LIST".into(),
            _ => {}
        }
    }
    let mut out = String::new();
    for (i, c) in format!("{k:?}").chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            out.push('_');
        }
        out.extend(c.to_uppercase());
    }
    out
}

/// rust-analyzer's name of a token of kind `k`.
fn token_kind(k: T) -> &'static str {
    match k {
        T::Whitespace => "WHITESPACE",
        T::LineComment | T::BlockComment => "COMMENT",
        T::Shebang => "SHEBANG",
        T::Ident | T::RawIdent => "IDENT",
        T::Lifetime => "LIFETIME_IDENT",
        T::Int => "INT_NUMBER",
        T::Float => "FLOAT_NUMBER",
        T::Char => "CHAR",
        T::Byte => "BYTE",
        T::Str | T::RawStr => "STRING",
        T::ByteStr | T::RawByteStr => "BYTE_STRING",
        T::CStr | T::RawCStr => "C_STRING",
        T::SelfKw => "SELF_KW",
        T::SelfType => "SELF_TYPE_KW",
        T::Underscore => "UNDERSCORE",
        T::LParen => "L_PAREN",
        T::RParen => "R_PAREN",
        T::LBrace => "L_CURLY",
        T::RBrace => "R_CURLY",
        T::LBracket => "L_BRACK",
        T::RBracket => "R_BRACK",
        T::Bang => "BANG",
        T::Hash => "POUND",
        T::Dollar => "DOLLAR",
        T::Percent => "PERCENT",
        T::Amp => "AMP",
        T::Star => "STAR",
        T::Plus => "PLUS",
        T::Comma => "COMMA",
        T::Minus => "MINUS",
        T::Dot => "DOT",
        T::Slash => "SLASH",
        T::Colon => "COLON",
        T::Semi => "SEMICOLON",
        T::Lt => "L_ANGLE",
        T::Eq => "EQ",
        T::Gt => "R_ANGLE",
        T::Question => "QUESTION",
        T::At => "AT",
        T::Caret => "CARET",
        T::Pipe => "PIPE",
        T::Tilde => "TILDE",
        T::Unknown => "ERROR",
        _ => keyword(k),
    }
}

/// A keyword's token name: `FN_KW`.
fn keyword(k: T) -> &'static str {
    match k {
        T::As => "AS_KW",
        T::Async => "ASYNC_KW",
        T::Await => "AWAIT_KW",
        T::Break => "BREAK_KW",
        T::Const => "CONST_KW",
        T::Continue => "CONTINUE_KW",
        T::Crate => "CRATE_KW",
        T::Dyn => "DYN_KW",
        T::Else => "ELSE_KW",
        T::Enum => "ENUM_KW",
        T::Extern => "EXTERN_KW",
        T::False => "FALSE_KW",
        T::Fn => "FN_KW",
        T::For => "FOR_KW",
        T::If => "IF_KW",
        T::Impl => "IMPL_KW",
        T::In => "IN_KW",
        T::Let => "LET_KW",
        T::Loop => "LOOP_KW",
        T::Match => "MATCH_KW",
        T::Mod => "MOD_KW",
        T::Move => "MOVE_KW",
        T::Mut => "MUT_KW",
        T::Pub => "PUB_KW",
        T::Ref => "REF_KW",
        T::Return => "RETURN_KW",
        T::Static => "STATIC_KW",
        T::Struct => "STRUCT_KW",
        T::Super => "SUPER_KW",
        T::Trait => "TRAIT_KW",
        T::True => "TRUE_KW",
        T::Type => "TYPE_KW",
        T::Unsafe => "UNSAFE_KW",
        T::Use => "USE_KW",
        T::Where => "WHERE_KW",
        T::While => "WHILE_KW",
        _ => "ERROR",
    }
}

/// The punctuation rust-analyzer's parser joins into one token, by its text.
const JOINED: &[(&str, &str)] = &[
    ("..=", "DOT2EQ"),
    ("...", "DOT3"),
    ("<<=", "SHLEQ"),
    (">>=", "SHREQ"),
    ("::", "COLON2"),
    ("->", "THIN_ARROW"),
    ("=>", "FAT_ARROW"),
    ("==", "EQ2"),
    ("!=", "NEQ"),
    ("<=", "LTEQ"),
    (">=", "GTEQ"),
    ("&&", "AMP2"),
    ("||", "PIPE2"),
    ("+=", "PLUSEQ"),
    ("-=", "MINUSEQ"),
    ("*=", "STAREQ"),
    ("/=", "SLASHEQ"),
    ("%=", "PERCENTEQ"),
    ("^=", "CARETEQ"),
    ("&=", "AMPEQ"),
    ("|=", "PIPEEQ"),
    ("<<", "SHL"),
    (">>", "SHR"),
    ("..", "DOT2"),
];

/// What a node holds, in source order.
enum Item {
    Node(NodeId),
    /// One token, or several joint ones rust-analyzer has as one, with its kind.
    Token(usize, usize, &'static str),
    /// A `NAME_REF` around the token at that byte range.
    NameRef(usize, usize, &'static str),
    /// A `STMT_LIST` around these items: a block's braces and what is between them.
    StmtList(Vec<Item>),
}

/// The node `n`'s children and own tokens in source order, joint punctuation joined.
fn items(tree: &SyntaxTree<'_>, n: NodeId) -> Vec<Item> {
    let toks: Vec<Token> = tree.own_tokens(n).map(|(_, t)| t).filter(|t| !t.is_trivia()).collect();
    let mut out: Vec<(usize, Item)> = tree.children(n).map(|c| (tree.range(c).start, Item::Node(c))).collect();
    // Whitespace and comments, which the tree's nodes do not hold: in the innermost node around them, as
    // rust-analyzer's tree has every byte.
    let r = range_of(tree, n);
    let kids: Vec<std::ops::Range<usize>> = tree.children(n).map(|c| tree.range(c)).collect();
    let lo = tree.tokens.partition_point(|t| (t.start as usize) < r.start);
    for t in tree.tokens[lo..].iter().take_while(|t| (t.start as usize) < r.end) {
        if t.is_trivia() && !kids.iter().any(|k| k.start <= t.start as usize && t.end as usize <= k.end) {
            out.push((t.start as usize, Item::Token(t.start as usize, t.end as usize, token_kind(t.kind))));
        }
    }
    let name_ref_owner = matches!(tree.kind(n), K::PathSegment | K::FieldExpr | K::MethodCallExpr | K::RecordExprField | K::RecordPatField);
    let mut i = 0;
    while i < toks.len() {
        let t = toks[i];
        let mut joined = None;
        for (text, kind) in JOINED {
            let n_chars = text.len();
            if i + n_chars <= toks.len() && (i..i + n_chars - 1).all(|j| toks[j].is_joint() && toks[j].end == toks[j + 1].start) {
                let s: String = toks[i..i + n_chars].iter().map(|t| t.text(tree.src)).collect();
                if s == *text {
                    joined = Some((n_chars, *kind));
                    break;
                }
            }
        }
        let (count, kind) = joined.unwrap_or((1, token_kind(t.kind)));
        let (start, end) = (t.start as usize, toks[i + count - 1].end as usize);
        let item = if name_ref_owner && count == 1 && matches!(t.kind, T::Ident | T::RawIdent | T::SelfKw | T::SelfType | T::Super | T::Crate | T::Int) {
            Item::NameRef(start, end, kind)
        } else {
            Item::Token(start, end, kind)
        };
        out.push((start, item));
        i += count;
    }
    out.sort_by_key(|(s, _)| *s);
    let mut list: Vec<Item> = out.into_iter().map(|(_, it)| it).collect();
    // A block's braces and statements are a `STMT_LIST` in rust-analyzer's tree.
    if tree.kind(n) == K::BlockExpr {
        let open = list.iter().position(|it| matches!(it, Item::Token(_, _, "L_CURLY")));
        let close = list.iter().rposition(|it| matches!(it, Item::Token(_, _, "R_CURLY")));
        if let (Some(o), Some(c)) = (open, close) {
            let inner: Vec<Item> = list.drain(o..=c).collect();
            list.insert(o, Item::StmtList(inner));
        }
    }
    list
}

/// `[offset,line,col]` of byte `b` of `src`.
fn pos(src: &str, b: usize) -> String {
    let ls = src[..b].rfind('\n').map_or(0, |i| i + 1);
    format!("[{b},{},{}]", src[..ls].matches('\n').count(), b - ls)
}

fn write_items(tree: &SyntaxTree<'_>, list: &[Item], out: &mut String) {
    for (i, it) in list.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let src = tree.src;
        match it {
            Item::Node(c) => write_node(tree, *c, out),
            Item::Token(s, e, k) => {
                let _ = write!(out, r#"{{"type":"Token","kind":"{k}","start":{},"end":{}}}"#, pos(src, *s), pos(src, *e));
            }
            Item::NameRef(s, e, k) => {
                let (ps, pe) = (pos(src, *s), pos(src, *e));
                let _ = write!(out, r#"{{"type":"Node","kind":"NAME_REF","start":{ps},"end":{pe},"children":[{{"type":"Token","kind":"{k}","start":{ps},"end":{pe}}}]}}"#);
            }
            Item::StmtList(inner) => {
                let (s, e) = (first_start(tree, inner), last_end(tree, inner));
                let _ = write!(out, r#"{{"type":"Node","kind":"STMT_LIST","start":{},"end":{},"children":["#, pos(src, s), pos(src, e));
                write_items(tree, inner, out);
                out.push_str("]}");
            }
        }
    }
}

fn first_start(tree: &SyntaxTree<'_>, list: &[Item]) -> usize {
    match list.first() {
        Some(Item::Node(c)) => tree.range(*c).start,
        Some(Item::Token(s, ..) | Item::NameRef(s, ..)) => *s,
        Some(Item::StmtList(inner)) => first_start(tree, inner),
        None => 0,
    }
}

fn last_end(tree: &SyntaxTree<'_>, list: &[Item]) -> usize {
    match list.last() {
        Some(Item::Node(c)) => tree.range(*c).end,
        Some(Item::Token(_, e, ..) | Item::NameRef(_, e, _)) => *e,
        Some(Item::StmtList(inner)) => last_end(tree, inner),
        None => 0,
    }
}

/// The bytes of node `n`: the whole text for the root, as rust-analyzer's `SOURCE_FILE`.
fn range_of(tree: &SyntaxTree<'_>, n: NodeId) -> std::ops::Range<usize> {
    if n == tree.root() {
        0..tree.src.len()
    } else {
        tree.range(n)
    }
}

fn write_node(tree: &SyntaxTree<'_>, n: NodeId, out: &mut String) {
    let r = range_of(tree, n);
    let _ = write!(out, r#"{{"type":"Node","kind":"{}","start":{},"end":{},"children":["#, node_kind(tree, n), pos(tree.src, r.start), pos(tree.src, r.end));
    write_items(tree, &items(tree, n), out);
    out.push_str("]}");
}

/// The tree of `src` as rust-analyzer's JSON, from the smallest node covering the byte range `range` when given.
pub fn syntax_tree_json(src: &str, range: Option<std::ops::Range<usize>>) -> String {
    let tree = ridx_syntax::parse(src);
    let mut root = tree.root();
    if let Some(r) = range {
        let c = tree.covering(r.start);
        root = std::iter::once(c).chain(tree.ancestors(c)).find(|&n| tree.range(n).end >= r.end).unwrap_or(root);
    }
    let mut out = String::new();
    write_node(&tree, root, &mut out);
    out
}

#[cfg(test)]
#[path = "syntax_tree_tests.rs"]
mod tests;
