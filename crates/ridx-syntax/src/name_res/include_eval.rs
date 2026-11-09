/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Compile-time string-expression evaluator for `include!(...)` paths (`"lit.rs"`,
//! `concat!(...)`, `env!("OUT_DIR")`, and local/indexed `macro_rules!` path wrappers).

use super::cfg::decode_str_literal;
use super::db::{CrateId, FileId, ItemDb, SourceDb};
use super::mod_tree::{join_norm_into, parent_dir_str};
use crate::lexer::lex;
use crate::syntax_kind::SyntaxKind as K;
use crate::token::{Token, TokenKind as T};
use crate::tree::{NodeId, SyntaxTree};

const MAX_STR_EVAL_DEPTH: u32 = 6;

#[inline]
pub fn is_include_call_path(raw_call: &str) -> bool {
    matches!(raw_call.trim(), "include" | "std::include" | "core::include" | "::std::include" | "::core::include")
}

/// Evaluates the path expression inside an `include!(...)` `MacroCall` node (`call_node`).
pub fn eval_include_path_str(
    st: &SyntaxTree<'_>,
    call_node: NodeId,
    db: &dyn ItemDb,
    krate: CrateId,
    resolve_macro: &mut dyn FnMut(&str, &str) -> Option<String>,
) -> Option<String> {
    let tt_node = st.child(call_node, K::TokenTree)?;
    let inner = strip_outer_delims(st.text(tt_node));
    eval_str_expr_text(inner, db, krate, resolve_macro, 0)
}

/// Resolves an `include!(...)` `MacroCall` node in `cur_file` to its target [`FileId`] in `sources`.
pub fn resolve_include_target(
    st: &SyntaxTree<'_>,
    call_node: NodeId,
    cur_file: FileId,
    db: &dyn ItemDb,
    sources: &dyn SourceDb,
    krate: CrateId,
    resolve_macro: &mut dyn FnMut(&str, &str) -> Option<String>,
) -> Option<FileId> {
    let raw_target = eval_include_path_str(st, call_node, db, krate, resolve_macro)?;
    let base_dir = if cur_file.is_some() { parent_dir_str(sources.file_path(cur_file)) } else { "" };
    let mut norm = String::with_capacity(base_dir.len() + raw_target.len() + 1);
    join_norm_into(&mut norm, base_dir, &raw_target, "");
    sources.resolve_path(&norm).or_else(|| {
        join_norm_into(&mut norm, "", &raw_target, "");
        sources.resolve_path(&norm)
    })
}

pub fn eval_str_expr_text(text: &str, db: &dyn ItemDb, krate: CrateId, resolve_macro: &mut dyn FnMut(&str, &str) -> Option<String>, depth: u32) -> Option<String> {
    if depth >= MAX_STR_EVAL_DEPTH {
        return None;
    }
    let toks = lex(text);
    let sig: Vec<Token> = toks.into_iter().filter(|t| !t.is_trivia()).collect();
    eval_str_tokens(text, &sig, db, krate, resolve_macro, depth)
}

fn eval_str_tokens(src: &str, mut sig: &[Token], db: &dyn ItemDb, krate: CrateId, resolve_macro: &mut dyn FnMut(&str, &str) -> Option<String>, depth: u32) -> Option<String> {
    if depth >= MAX_STR_EVAL_DEPTH || sig.is_empty() {
        return None;
    }
    if let Some((last, rest)) = sig.split_last() {
        if last.kind == T::Comma {
            sig = rest;
        }
    }
    if sig.len() == 1 {
        let t = sig[0];
        return match t.kind {
            T::Str | T::RawStr => decode_str_literal(src, t).map(|c| c.into_owned()),
            T::Char => {
                let raw = t.text(src).strip_prefix('\'')?.strip_suffix('\'')?;
                Some(raw.to_string())
            }
            T::Int | T::Float | T::True | T::False => Some(t.text(src).to_string()),
            _ => None,
        };
    }

    let bang_pos = sig.iter().position(|t| t.kind == T::Bang)?;
    if bang_pos == 0 || bang_pos + 2 > sig.len() {
        return None;
    }
    let open = sig[bang_pos + 1];
    let close = *sig.last()?;
    if !matches!((open.kind, close.kind), (T::LParen, T::RParen) | (T::LBracket, T::RBracket) | (T::LBrace, T::RBrace)) {
        return None;
    }
    let mut path_buf = String::new();
    for pt in &sig[..bang_pos] {
        path_buf.push_str(pt.text(src).trim());
    }
    let args_toks = &sig[bang_pos + 2..sig.len() - 1];
    match path_buf.as_str() {
        "concat" | "std::concat" | "core::concat" | "::std::concat" | "::core::concat" => {
            let mut out = String::new();
            for chunk in split_top_level_commas(args_toks) {
                if chunk.is_empty() {
                    continue;
                }
                let piece = eval_str_tokens(src, chunk, db, krate, resolve_macro, depth + 1)?;
                out.push_str(&piece);
            }
            Some(out)
        }
        "env" | "std::env" | "core::env" | "::std::env" | "::core::env" => {
            let first = split_top_level_commas(args_toks).into_iter().find(|c| !c.is_empty())?;
            let var_name = eval_str_tokens(src, first, db, krate, resolve_macro, depth + 1)?;
            db.crate_env(krate, &var_name).map(str::to_owned)
        }
        other => {
            let inner_start = open.end as usize;
            let inner_end = close.start as usize;
            let inner_str = src.get(inner_start..inner_end)?;
            let expanded = resolve_macro(other, inner_str)?;
            eval_str_expr_text(&expanded, db, krate, resolve_macro, depth + 1)
        }
    }
}

fn split_top_level_commas(toks: &[Token]) -> Vec<&[Token]> {
    let mut out = Vec::new();
    let mut depth = 0u32;
    let mut start = 0usize;
    for (i, t) in toks.iter().enumerate() {
        match t.kind {
            T::LParen | T::LBracket | T::LBrace => depth += 1,
            T::RParen | T::RBracket | T::RBrace => depth = depth.saturating_sub(1),
            T::Comma if depth == 0 => {
                out.push(&toks[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    if start < toks.len() {
        out.push(&toks[start..]);
    }
    out
}

#[inline]
pub fn strip_outer_delims(s: &str) -> &str {
    let b = s.as_bytes();
    if b.len() >= 2 && matches!((b[0], b[b.len() - 1]), (b'{', b'}') | (b'(', b')') | (b'[', b']')) {
        &s[1..s.len() - 1]
    } else {
        s
    }
}
