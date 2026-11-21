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
pub(crate) fn primitive_shadowed(tree: &SyntaxTree<'_>) -> bool {
    tree.descendants(tree.root()).any(|d| tree.kind(d) == K::Name && PRIMITIVES.contains(&tree.text(d)))
}
