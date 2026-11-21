/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! High-throughput, cache-linear CST scope walkers for `sema_fn`: single-token `PathExpr`
//! matching and lexical shadowing across blocks, let-chains, matches, loops, and closures.

use crate::sema_fn::ParamUses;
use crate::sema_macro::scan_token_tree;
use crate::sema_pat::{block_flavor_fast, find_self_uses, item_shadows_value, preceded_by_eq, tok_matches_ident, BlockFlavor};
use crate::syntax_kind::SyntaxKind as K;
use crate::token::{Token, TokenKind as T};
use crate::tree::{Node, NodeId, SyntaxTree};

/// Single flat pass over a pattern: scans any `MacroPat` into `out` and returns `true` if `pat_id` binds `tb`.
fn check_pat(ctx: &mut Ctx<'_, '_>, pat_id: u32) -> bool {
    let mut binds = false;
    let mut idx = pat_id as usize;
    let end = ctx.nodes[idx].subtree_end as usize;
    while idx < end {
        let n = ctx.nodes[idx];
        match n.kind {
            K::IdentPat => {
                if !binds && idx + 1 < end && ctx.nodes[idx + 1].kind == K::Name {
                    let nn = ctx.nodes[idx + 1];
                    if nn.first < nn.end && tok_matches_ident(ctx.tokens[nn.first as usize], ctx.src, ctx.target) {
                        binds = true;
                    }
                    idx = nn.subtree_end as usize;
                    continue;
                }
                idx += 1;
            }
            K::MacroPat => {
                let mut c = idx + 1;
                let mend = n.subtree_end as usize;
                while c < mend {
                    if ctx.nodes[c].kind == K::TokenTree {
                        scan_token_tree(ctx.tokens, ctx.src_str, &ctx.nodes[c], ctx.target, ctx.out);
                        break;
                    }
                    c = ctx.nodes[c].subtree_end as usize;
                }
                idx = mend;
            }
            K::RangePat | K::PathPat | K::LiteralPat | K::WildcardPat | K::RestPat | K::ConstBlockPat | K::Path => {
                idx = n.subtree_end as usize;
            }
            _ => idx += 1,
        }
    }
    binds
}

pub(crate) fn find_param_uses(tree: &SyntaxTree<'_>, body: NodeId, target: &str, out: &mut ParamUses) {
    if target == "self" {
        find_self_uses(tree, body, out);
        return;
    }
    let mut ctx = Ctx { nodes: &tree.nodes, tokens: &tree.tokens, src: tree.src.as_bytes(), src_str: tree.src, target: target.as_bytes(), out };
    walk_block(&mut ctx, body.0, true);
}

/// The uses of `target`, bound by the `let` statement `let_stmt`, in the rest of its block: the statements after it,
/// up to one that binds the name again. An item of the block named so does not end them: a `let` shadows the items
/// of its block (review of 63f24322: `let value = 1; fn value() {}` then `value` reads the local).
pub(crate) fn find_let_uses(tree: &SyntaxTree<'_>, let_stmt: NodeId, target: &str, out: &mut ParamUses) {
    let Some(block) = tree.parent(let_stmt) else { return };
    let mut ctx = Ctx { nodes: &tree.nodes, tokens: &tree.tokens, src: tree.src.as_bytes(), src_str: tree.src, target: target.as_bytes(), out };
    let (start, end) = (tree.nodes[let_stmt.0 as usize].subtree_end as usize, tree.nodes[block.0 as usize].subtree_end as usize);
    scan_range(&mut ctx, u32::MAX, 0, 0, start, end);
}

struct Ctx<'t, 's> {
    nodes: &'t [Node],
    tokens: &'t [Token],
    src: &'s [u8],
    src_str: &'s str,
    target: &'t [u8],
    out: &'t mut ParamUses,
}

fn walk_block(ctx: &mut Ctx<'_, '_>, block_id: u32, is_fn_root: bool) {
    let bn = ctx.nodes[block_id as usize];
    if !is_fn_root && block_flavor_fast(ctx.tokens, ctx.src, &bn) == BlockFlavor::Const {
        return;
    }
    let u0 = ctx.out.uses.len();
    let m0 = ctx.out.macro_uses.len();
    scan_range(ctx, block_id, u0, m0, block_id as usize + 1, bn.subtree_end as usize);
}

#[inline]
fn scan_sub(ctx: &mut Ctx<'_, '_>, id: u32) {
    let idx = id as usize;
    scan_range(ctx, u32::MAX, 0, 0, idx, ctx.nodes[idx].subtree_end as usize);
}

fn walk_let_stmt(ctx: &mut Ctx<'_, '_>, let_idx: usize, let_end: usize) -> bool {
    let mut pat_id: Option<u32> = None;
    let mut c = let_idx + 1;
    while c < let_end {
        let cn = ctx.nodes[c];
        let ck = cn.kind;
        let cend = cn.subtree_end as usize;
        if pat_id.is_none() && ck.is_pat() {
            pat_id = Some(c as u32);
        } else if ck.is_expr() || (ck == K::MacroCall && preceded_by_eq(ctx.tokens, ctx.nodes[let_idx].first, cn.first)) {
            scan_range(ctx, u32::MAX, 0, 0, c, cend);
        } else if ck == K::LetElse {
            let mut le = c + 1;
            while le < cend {
                if ctx.nodes[le].kind == K::BlockExpr {
                    walk_block(ctx, le as u32, false);
                    break;
                }
                le = ctx.nodes[le].subtree_end as usize;
            }
        }
        c = cend;
    }
    pat_id.is_some_and(|p| check_pat(ctx, p))
}

#[inline]
fn is_logical_and_bin(ctx: &Ctx<'_, '_>, lhs_end_tok: u32, bin_end_tok: u32) -> bool {
    let mut i = lhs_end_tok as usize;
    let end = bin_end_tok as usize;
    while i < end {
        let t = ctx.tokens[i];
        if !t.is_trivia() {
            return t.kind == T::Amp && t.is_joint() && i + 1 < end && ctx.tokens[i + 1].kind == T::Amp;
        }
        i += 1;
    }
    false
}

#[inline]
fn next_expr_child(nodes: &[Node], cur: &mut usize, end: usize) -> Option<u32> {
    while *cur < end {
        let id = *cur;
        let n = nodes[id];
        *cur = n.subtree_end as usize;
        if n.kind.is_expr() || n.kind == K::MacroCall {
            return Some(id as u32);
        }
    }
    None
}

fn walk_cond(ctx: &mut Ctx<'_, '_>, cond_id: u32) -> bool {
    let idx = cond_id as usize;
    let n = ctx.nodes[idx];
    let end = n.subtree_end as usize;
    match n.kind {
        K::LetExpr => {
            let mut pat_id: Option<u32> = None;
            let mut c = idx + 1;
            while c < end {
                let ck = ctx.nodes[c].kind;
                let cend = ctx.nodes[c].subtree_end as usize;
                if pat_id.is_none() && ck.is_pat() {
                    pat_id = Some(c as u32);
                } else if ck.is_expr() || ck == K::MacroCall {
                    scan_range(ctx, u32::MAX, 0, 0, c, cend);
                }
                c = cend;
            }
            pat_id.is_some_and(|p| check_pat(ctx, p))
        }
        K::BinExpr => {
            let mut cur = idx + 1;
            let Some(lhs) = next_expr_child(ctx.nodes, &mut cur, end) else { return false };
            if is_logical_and_bin(ctx, ctx.nodes[lhs as usize].end, n.end) {
                if walk_cond(ctx, lhs) {
                    return true;
                }
                return next_expr_child(ctx.nodes, &mut cur, end).is_some_and(|rhs| walk_cond(ctx, rhs));
            }
            scan_range(ctx, u32::MAX, 0, 0, idx + 1, end);
            false
        }
        K::ParenExpr => {
            let mut cur = idx + 1;
            next_expr_child(ctx.nodes, &mut cur, end).is_some_and(|inner| walk_cond(ctx, inner))
        }
        _ => {
            scan_range(ctx, u32::MAX, 0, 0, idx, end);
            false
        }
    }
}

/// Flat preorder scan over `[idx..end)` while `target` is in scope: zero recursion on ordinary expressions!
fn scan_range(ctx: &mut Ctx<'_, '_>, blk: u32, u0: usize, m0: usize, mut idx: usize, end: usize) {
    while idx < end {
        let n = ctx.nodes[idx];
        let nend = n.subtree_end as usize;
        let k = n.kind;
        if k.is_sema_skip() {
            if k.is_item() && n.parent == blk && item_shadows_value(ctx.nodes, ctx.tokens, ctx.src, ctx.target, idx) {
                ctx.out.uses.truncate(u0);
                ctx.out.macro_uses.truncate(m0);
                return;
            }
            idx = nend;
            continue;
        }
        match k {
            K::PathExpr => {
                if n.end == n.first + 1 && tok_matches_ident(ctx.tokens[n.first as usize], ctx.src, ctx.target) {
                    let t = ctx.tokens[n.first as usize];
                    ctx.out.uses.push(t.start as usize..t.end as usize);
                }
                idx = nend;
            }
            K::MacroCall | K::MacroPat => {
                let mut c = idx + 1;
                while c < nend {
                    if ctx.nodes[c].kind == K::TokenTree {
                        scan_token_tree(ctx.tokens, ctx.src_str, &ctx.nodes[c], ctx.target, ctx.out);
                        break;
                    }
                    c = ctx.nodes[c].subtree_end as usize;
                }
                idx = nend;
            }
            K::BlockExpr => {
                walk_block(ctx, idx as u32, false);
                idx = nend;
            }
            K::LetStmt => {
                if walk_let_stmt(ctx, idx, nend) {
                    if blk != u32::MAX {
                        let mut rem = nend;
                        while rem < end {
                            let rn = ctx.nodes[rem];
                            if rn.kind.is_item() && item_shadows_value(ctx.nodes, ctx.tokens, ctx.src, ctx.target, rem) {
                                ctx.out.uses.truncate(u0);
                                ctx.out.macro_uses.truncate(m0);
                                break;
                            }
                            rem = rn.subtree_end as usize;
                        }
                    }
                    return;
                }
                idx = nend;
            }
            K::IfExpr => {
                let mut cur = idx + 1;
                if let Some(cond) = next_expr_child(ctx.nodes, &mut cur, nend) {
                    let shadowed = walk_cond(ctx, cond);
                    if let Some(then_blk) = next_expr_child(ctx.nodes, &mut cur, nend) {
                        if !shadowed {
                            scan_sub(ctx, then_blk);
                        }
                    }
                    if let Some(else_br) = next_expr_child(ctx.nodes, &mut cur, nend) {
                        scan_sub(ctx, else_br);
                    }
                }
                idx = nend;
            }
            K::WhileExpr => {
                let mut cur = idx + 1;
                if let Some(cond) = next_expr_child(ctx.nodes, &mut cur, nend) {
                    let shadowed = walk_cond(ctx, cond);
                    if let Some(body) = next_expr_child(ctx.nodes, &mut cur, nend) {
                        if !shadowed {
                            scan_sub(ctx, body);
                        }
                    }
                }
                idx = nend;
            }
            K::ForExpr => {
                let mut pat_id: Option<u32> = None;
                let mut c = idx + 1;
                while c < nend {
                    if ctx.nodes[c].kind.is_pat() {
                        pat_id = Some(c as u32);
                        c = ctx.nodes[c].subtree_end as usize;
                        break;
                    }
                    c = ctx.nodes[c].subtree_end as usize;
                }
                let shadowed = pat_id.is_some_and(|p| check_pat(ctx, p));
                if let Some(iter_e) = next_expr_child(ctx.nodes, &mut c, nend) {
                    scan_sub(ctx, iter_e);
                }
                if let Some(body) = next_expr_child(ctx.nodes, &mut c, nend) {
                    if !shadowed {
                        scan_sub(ctx, body);
                    }
                }
                idx = nend;
            }
            K::MatchExpr => {
                let mut cur = idx + 1;
                if let Some(scrut) = next_expr_child(ctx.nodes, &mut cur, nend) {
                    scan_sub(ctx, scrut);
                }
                let mut c = idx + 1;
                while c < nend {
                    if ctx.nodes[c].kind == K::MatchArmList {
                        walk_match_arms(ctx, c + 1, ctx.nodes[c].subtree_end as usize);
                        break;
                    }
                    c = ctx.nodes[c].subtree_end as usize;
                }
                idx = nend;
            }
            K::ClosureExpr => {
                let mut shadowed = false;
                let mut c = idx + 1;
                while c < nend {
                    let cn = ctx.nodes[c];
                    if cn.kind == K::ParamList {
                        let mut pm = c + 1;
                        let pend = cn.subtree_end as usize;
                        while pm < pend {
                            if ctx.nodes[pm].kind == K::Param {
                                let mut pc = pm + 1;
                                let pm_end = ctx.nodes[pm].subtree_end as usize;
                                while pc < pm_end {
                                    if ctx.nodes[pc].kind.is_pat() && check_pat(ctx, pc as u32) {
                                        shadowed = true;
                                    }
                                    pc = ctx.nodes[pc].subtree_end as usize;
                                }
                            }
                            pm = ctx.nodes[pm].subtree_end as usize;
                        }
                    } else if !shadowed && (cn.kind.is_expr() || cn.kind == K::MacroCall) {
                        scan_range(ctx, u32::MAX, 0, 0, c, cn.subtree_end as usize);
                    }
                    c = cn.subtree_end as usize;
                }
                idx = nend;
            }
            K::LetExpr => {
                let _ = walk_cond(ctx, idx as u32);
                idx = nend;
            }
            k if k.is_pat() => {
                idx = nend;
            }
            _ => {
                idx += 1;
            }
        }
    }
}

fn walk_match_arms(ctx: &mut Ctx<'_, '_>, mut arm: usize, list_end: usize) {
    while arm < list_end {
        let an = ctx.nodes[arm];
        let aend = an.subtree_end as usize;
        if an.kind == K::MatchArm {
            let mut c = arm + 1;
            while c < aend {
                let cn = ctx.nodes[c];
                let cend = cn.subtree_end as usize;
                if cn.kind.is_pat() {
                    if check_pat(ctx, c as u32) {
                        break;
                    }
                } else if cn.kind == K::MatchGuard {
                    let mut gc = c + 1;
                    if next_expr_child(ctx.nodes, &mut gc, cend).is_some_and(|ge| walk_cond(ctx, ge)) {
                        break;
                    }
                } else if cn.kind.is_expr() || cn.kind == K::MacroCall {
                    scan_range(ctx, u32::MAX, 0, 0, c, cend);
                }
                c = cend;
            }
        }
        arm = aend;
    }
}
