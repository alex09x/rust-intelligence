/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! rust-analyzer's names of node and token kinds, for `ridx/syntaxTree` (`syntax_tree`).

use ridx_syntax::{NodeId, SyntaxKind as K, SyntaxTree, TokenKind as T};

/// rust-analyzer's name of a node kind: `PathType` is `PATH_TYPE`; an `ItemList` of an impl or a trait is an
/// `ASSOC_ITEM_LIST`, of an `extern` block an `EXTERN_ITEM_LIST`.
pub(super) fn node_kind(tree: &SyntaxTree<'_>, n: NodeId) -> String {
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
pub(super) fn token_kind(k: T) -> &'static str {
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
pub(super) const JOINED: &[(&str, &str)] = &[
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

/// A contextual keyword's token name, where its node gives the word that role (`macro_rules` of a `macro_rules!`, the
/// `union` of a union): rust-analyzer's parser remaps them from `IDENT` (review of 27bd4208).
pub(super) fn contextual(owner: K, word: &str) -> Option<&'static str> {
    Some(match (owner, word) {
        (K::MacroRules, "macro_rules") => "MACRO_RULES_KW",
        (K::Union, "union") => "UNION_KW",
        (K::Fn | K::Impl | K::Const | K::TypeAlias, "default") => "DEFAULT_KW",
        (K::Trait | K::Impl, "auto") => "AUTO_KW",
        (K::Fn | K::Static, "safe") => "SAFE_KW",
        (K::RefExpr, "raw") => "RAW_KW",
        (K::BlockExpr, "try") => "TRY_KW",
        _ => return None,
    })
}
