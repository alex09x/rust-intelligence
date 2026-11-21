/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! The locals an assist moves code across: which binding a name read is, its type, and whether passing or copying it
//! leaves the code meaning what it meant (`Extract into function`, `Inline variable`).

use ridx_syntax::{NodeId, SyntaxKind as K, SyntaxTree, TokenKind as T};

/// What the extraction needs to know from outside the syntax tree: the type of the `let` whose name is at a byte
/// offset (written on it, or as `Add explicit type` would write it).
pub(crate) type LetType<'a> = &'a dyn Fn(usize) -> Option<String>;

pub(crate) const PRIMITIVES: &[&str] = &["u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128", "isize", "f32", "f64", "bool", "char"];

/// Whether a value of type `ty` is copied when passed: a primitive (the file declares none of their names: `type u32 =
/// String`, review of 531974ed) or a shared reference, told by its tokens (`&'a mut T` is no shared one).
pub(crate) fn copy_type(ty: &str, primitive_shadowed: bool) -> bool {
    let toks: Vec<_> = ridx_syntax::lex(ty).into_iter().filter(|t| !t.is_trivia()).collect();
    match toks.as_slice() {
        [t] => !primitive_shadowed && PRIMITIVES.contains(&t.text(ty)),
        [amp, rest @ ..] if amp.kind == T::Amp => {
            let rest = match rest {
                [lt, rest @ ..] if lt.kind == T::Lifetime => rest,
                rest => rest,
            };
            !rest.first().is_some_and(|t| t.kind == T::Mut) && !rest.is_empty()
        }
        _ => false,
    }
}

/// The local `name`, read at byte `at` inside `f`, with its type: a parameter, or a `let` before the selection whose
/// uses include `at`. `Err` when the name is bound otherwise (a pattern, a closure's parameter): not known; or `mut`:
/// a method taking `&mut self` would change the copy passed, not the local.
pub(crate) fn local_of(tree: &SyntaxTree<'_>, f: NodeId, name: &str, at: usize, let_type: LetType) -> Result<Option<String>, ()> {
    // `mut`, or `ref` (whose binding is a reference, not the type written for what it matched: review of 531974ed).
    let mutable = |pat: NodeId| tree.own_tokens(pat).any(|(_, t)| matches!(t.kind, T::Mut | T::Ref));
    for p in ridx_syntax::sema_fn::params(tree, f) {
        if p.bindings.as_slice() == [name] && p.pat.is_some_and(|pat| tree.kind(pat) == K::IdentPat) {
            let uses = ridx_syntax::sema_fn::param_uses(tree, f, name);
            if uses.uses.iter().any(|r| r.start == at) {
                if p.pat.is_some_and(mutable) {
                    return Err(());
                }
                return Ok(p.ty_text.map(str::to_string));
            }
        }
    }
    let mut bound_otherwise = false;
    for pat in tree.descendants(f).filter(|&n| tree.kind(n) == K::IdentPat && tree.range(n).end <= at) {
        let Some(n) = tree.child(pat, K::Name).filter(|&n| ridx_syntax::sema_fn::norm_ident(tree.text(n)) == name) else { continue };
        match tree.parent(pat).filter(|&s| tree.kind(s) == K::LetStmt) {
            Some(stmt) if ridx_syntax::sema_fn::let_uses(tree, stmt, name).uses.iter().any(|r| r.start == at) => {
                // A `let x;` with no initializer is assigned later, which a copy passed would not see (review of
                // 6c5f3f95).
                let initialized = tree.children(stmt).any(|c| tree.kind(c).is_expr() || tree.kind(c) == K::MacroCall);
                if mutable(pat) || !initialized {
                    return Err(());
                }
                let written = tree.children(stmt).find(|&c| tree.kind(c).is_type()).map(|t| tree.text(t).to_string());
                return Ok(written.or_else(|| let_type(tree.range(n).start)));
            }
            Some(_) => {}
            None => bound_otherwise = true,
        }
    }
    if bound_otherwise {
        Err(())
    } else {
        Ok(None)
    }
}

/// Whether the file of `tree` declares something named as a primitive type (`type u32 = String`): then no spelling of
/// one is known to be it.
/// Declared or imported under such a name, in any spelling: `type r#u32 = String`, `use std::string::String as u32`
/// (review of 5852217f).
/// A glob import may bring in any name, a primitive's too, from a file not read here (review of 9d489afe: `use
/// types::*` with `type u32 = String` in types.rs).
pub(crate) fn primitive_shadowed(tree: &SyntaxTree<'_>, std_only: StdOnly<'_>) -> bool {
    let primitive = |s: &str| PRIMITIVES.contains(&ridx_syntax::sema_fn::norm_ident(s));
    macro_may_declare(tree, tree.root(), std_only)
        || tree.descendants(tree.root()).any(|d| {
            (tree.kind(d) == K::Name && primitive(tree.text(d))) || (tree.kind(d) == K::Use && tree.tokens_of(d).iter().any(|t| t.kind == T::Star || primitive(t.text(tree.src))))
        })
}

/// The standard library's statement macros, which declare nothing where they are called.
pub(crate) const STD_STATEMENT_MACROS: &[&str] = &[
    "println",
    "print",
    "eprintln",
    "eprint",
    "assert",
    "assert_eq",
    "assert_ne",
    "debug_assert",
    "debug_assert_eq",
    "debug_assert_ne",
    "panic",
    "todo",
    "unimplemented",
    "unreachable",
    "write",
    "writeln",
    "dbg",
    "format",
];

/// Whether no macro of a name is known but the standard library's, in the index or in an open document: a bare call in
/// a file is another file's macro when one is (`println!` defined in main.rs before `mod child;`: review of 651cf0e3).
pub(crate) type StdOnly<'a> = &'a dyn Fn(&str) -> bool;

/// The [`StdOnly`] of the index `ws` and the open documents of `overlays`, for `file`: only while the index read every
/// other source of the crates holding it as they are now, an absence in an older reading of them being no proof
/// (review of f60136bc: a closed parent changed since).
pub(crate) fn std_only<'a>(ws: &'a ridx_format::WorkspaceIndex, overlays: &'a crate::overlay::OverlayManager, file: &'a str) -> impl Fn(&str) -> bool + 'a {
    let current = std::cell::OnceCell::new();
    move |n: &str| {
        let indexed = ws.exact_symbols_by_name(n).iter().filter(|s| s.kind == ridx_format::DefKind::Macro).all(|s| matches!(s.crate_name, "std" | "core" | "alloc"));
        let typed = overlays.texts().any(|text| {
            let toks: Vec<_> = ridx_syntax::lex(text).into_iter().filter(|t| !t.is_trivia()).collect();
            toks.windows(3).any(|w| w[0].text(text) == "macro_rules" && w[1].kind == T::Bang && ridx_syntax::sema_fn::norm_ident(w[2].text(text)) == n)
        });
        indexed && !typed && *current.get_or_init(|| super::assist_current::crates_current(ws, overlays, file))
    }
}

/// The compiler's own attributes, which expand to no item.
const BUILTIN_ATTRIBUTES: &[&str] = &[
    "cfg",
    "test",
    "inline",
    "allow",
    "warn",
    "deny",
    "forbid",
    "expect",
    "doc",
    "must_use",
    "deprecated",
    "derive",
    "repr",
    "path",
    "macro_export",
    "macro_use",
    "non_exhaustive",
    "track_caller",
    "cold",
    "no_mangle",
    "export_name",
    "link_section",
    "used",
    "should_panic",
    "ignore",
    "automatically_derived",
    "rustfmt",
    "clippy",
    "diagnostic",
];

/// The derives of the standard library, which implement a trait and declare no name.
const STD_DERIVES: &[&str] =
    &["Debug", "Clone", "Copy", "PartialEq", "Eq", "PartialOrd", "Ord", "Hash", "Default", "std", "core", "fmt", "cmp", "hash", "clone", "marker", "default"];

/// Whether an attribute in the file of `tree` may be a macro's, an attribute macro or a derive not the standard
/// library's, whose expansion may declare any name (`#[maker::make_helper]` emitting `fn fun_name`: review of
/// 651cf0e3); a `cfg_attr` may apply one. A standard derive's name is the standard library's only where the file
/// imports nothing by it (`use maker::Debug;`, a glob) and no other macro of it is known (review of f60136bc).
pub(crate) fn macro_attribute(tree: &SyntaxTree<'_>, std_only: StdOnly<'_>) -> bool {
    tree.descendants(tree.root()).filter(|&d| tree.kind(d) == K::Attr).any(|attr| {
        let toks: Vec<_> = tree.tokens_of(attr).iter().filter(|t| !t.is_trivia() && !matches!(t.kind, T::Hash | T::Bang | T::LBracket)).collect();
        let names = |w: &[&ridx_syntax::Token]| {
            w.iter().filter(|t| matches!(t.kind, T::Ident | T::RawIdent)).map(|t| ridx_syntax::sema_fn::norm_ident(t.text(tree.src))).collect::<Vec<_>>()
        };
        let head = toks.first().filter(|t| matches!(t.kind, T::Ident | T::RawIdent)).map(|t| ridx_syntax::sema_fn::norm_ident(t.text(tree.src)));
        match head {
            Some("derive") => {
                names(&toks[1..]).iter().any(|n| !STD_DERIVES.contains(n) || (n.starts_with(|c: char| c.is_ascii_uppercase()) && (named_by_file(tree, n) || !std_only(n))))
            }
            // A path (`a::b`) is a macro's, but a tool's (`clippy::all`).
            Some(h) => !BUILTIN_ATTRIBUTES.contains(&h) || (toks.get(1).is_some_and(|t| t.kind == T::Colon) && !matches!(h, "rustfmt" | "clippy" | "diagnostic")),
            None => true,
        }
    })
}

/// Whether a macro called under `within` in an item's or a statement's place may declare an item or an import there
/// that the unexpanded tree does not show (`m!();` expanding to `use types::*;` or `const N: i32 = 3;`: review of
/// 7dca0b11): any in an item's place; in a statement's, any but the standard library's statement macros, unless the
/// file defines a macro of that name itself or `std_only` knows one elsewhere.
pub(crate) fn macro_may_declare(tree: &SyntaxTree<'_>, within: NodeId, std_only: StdOnly<'_>) -> bool {
    // The call's name and whether its path is the name alone (`types::println!` is a user's macro, whatever its last
    // segment: review of 9276edef).
    let name = |mc: NodeId| {
        let toks: Vec<_> = tree.tokens_of(mc).iter().filter(|t| !t.is_trivia()).collect();
        let bang = toks.iter().position(|t| t.kind == T::Bang)?;
        let name = toks[..bang].iter().rev().find(|t| matches!(t.kind, T::Ident | T::RawIdent))?;
        Some((ridx_syntax::sema_fn::norm_ident(name.text(tree.src)), bang == 1 && name.kind == T::Ident))
    };
    let by_node = std::iter::once(within).chain(tree.descendants(within)).filter(|&d| tree.kind(d) == K::MacroCall).any(|mc| {
        let parent = tree.parent(mc).map(|p| tree.kind(p));
        let item_place = matches!(parent, Some(K::SourceFile | K::ItemList | K::Module)) && name(mc) != Some(("macro_rules", true));
        let statement_place = matches!(parent, Some(K::BlockExpr | K::ExprStmt));
        item_place || (statement_place && name(mc).is_none_or(|(n, bare)| !bare || !STD_STATEMENT_MACROS.contains(&n) || named_by_file(tree, n) || !std_only(n)))
    });
    // A raw name's call the parser may not make a node of (`r#declare!();`): any counts, a comparison (`r#type != 0`)
    // being none.
    let toks: Vec<_> = tree.tokens_of(within).iter().filter(|t| !t.is_trivia()).collect();
    let raw_call = toks.windows(3).any(|w| w[0].kind == T::RawIdent && w[1].kind == T::Bang && matches!(w[2].kind, T::LParen | T::LBracket | T::LBrace));
    // An attribute macro's or a derive's expansion may declare anything too (review of f60136bc: an alias of `u8`).
    by_node || raw_call || macro_attribute(tree, std_only)
}

/// Whether the file of `tree` defines or imports a macro by the name `n`, or imports by a glob (`use types::println;`):
/// then it is not the standard library's.
fn named_by_file(tree: &SyntaxTree<'_>, n: &str) -> bool {
    tree.descendants(tree.root()).any(|d| {
        (tree.kind(d) == K::MacroRules && tree.child(d, K::Name).is_some_and(|m| ridx_syntax::sema_fn::norm_ident(tree.text(m)) == n))
            || (tree.kind(d) == K::Use
                && tree
                    .tokens_of(d)
                    .iter()
                    .any(|t| matches!(t.kind, T::Ident | T::RawIdent | T::Star) && (t.kind == T::Star || ridx_syntax::sema_fn::norm_ident(t.text(tree.src)) == n)))
    })
}
