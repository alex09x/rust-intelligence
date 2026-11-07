/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Inline parameter binding storage (`ParamBindings`), flat pattern binding scanners, block-flavor
//! classification, and the zero-recursion `self` parameter fast path.

use std::ops::Deref;

use crate::sema_fn::{norm_ident, ParamUses};
use crate::sema_macro::scan_self_token_tree;
use crate::syntax_kind::SyntaxKind as K;
use crate::token::{Token, TokenKind as T};
use crate::tree::{Node, NodeId, SyntaxTree};

/// Inline small-buffer of bound parameter identifier slices (zero heap allocation for 0..=2 bindings).
#[derive(Clone, PartialEq, Eq)]
pub enum ParamBindings<'s> {
    Inline { len: u8, slots: [&'s str; 2] },
    Heap(Vec<&'s str>),
}

impl Default for ParamBindings<'_> {
    #[inline]
    fn default() -> Self {
        Self::Inline { len: 0, slots: ["", ""] }
    }
}

impl<'s> ParamBindings<'s> {
    #[inline]
    pub const fn empty() -> Self {
        Self::Inline { len: 0, slots: ["", ""] }
    }

    #[inline]
    pub const fn single(s: &'s str) -> Self {
        Self::Inline { len: 1, slots: [s, ""] }
    }

    #[inline]
    pub fn as_slice(&self) -> &[&'s str] {
        match self {
            Self::Inline { len, slots } => &slots[..*len as usize],
            Self::Heap(v) => v.as_slice(),
        }
    }

    #[inline]
    pub(crate) fn push_unique(&mut self, s: &'s str) {
        if self.as_slice().contains(&s) {
            return;
        }
        match self {
            Self::Inline { len, slots } if (*len as usize) < 2 => {
                slots[*len as usize] = s;
                *len += 1;
            }
            Self::Inline { slots, .. } => {
                let mut v = Vec::with_capacity(4);
                v.push(slots[0]);
                v.push(slots[1]);
                v.push(s);
                *self = Self::Heap(v);
            }
            Self::Heap(v) => v.push(s),
        }
    }
}

impl<'s> Deref for ParamBindings<'s> {
    type Target = [&'s str];
    #[inline]
    fn deref(&self) -> &[&'s str] {
        self.as_slice()
    }
}

impl<'s> AsRef<[&'s str]> for ParamBindings<'s> {
    #[inline]
    fn as_ref(&self) -> &[&'s str] {
        self.as_slice()
    }
}

impl std::fmt::Debug for ParamBindings<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.as_slice().fmt(f)
    }
}

impl<'a, 's> IntoIterator for &'a ParamBindings<'s> {
    type Item = &'a &'s str;
    type IntoIter = std::slice::Iter<'a, &'s str>;
    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.as_slice().iter()
    }
}

impl PartialEq<[&str]> for ParamBindings<'_> {
    fn eq(&self, other: &[&str]) -> bool {
        self.as_slice() == other
    }
}

impl<const N: usize> PartialEq<[&str; N]> for ParamBindings<'_> {
    fn eq(&self, other: &[&str; N]) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl PartialEq<Vec<&str>> for ParamBindings<'_> {
    fn eq(&self, other: &Vec<&str>) -> bool {
        self.as_slice() == other.as_slice()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum BlockFlavor {
    Plain,
    Unsafe,
    Async,
    Try,
    Const,
}

#[inline]
pub(crate) fn block_flavor_fast(tokens: &[Token], src: &[u8], n: &Node) -> BlockFlavor {
    if n.first >= n.end {
        return BlockFlavor::Plain;
    }
    let tok = tokens[n.first as usize];
    match tok.kind {
        T::LBrace => BlockFlavor::Plain,
        T::Async => BlockFlavor::Async,
        T::Const => BlockFlavor::Const,
        T::Unsafe => BlockFlavor::Unsafe,
        T::Ident if tok.end - tok.start == 3 && &src[tok.start as usize..tok.end as usize] == b"try" => BlockFlavor::Try,
        _ => BlockFlavor::Plain,
    }
}

#[inline]
pub(crate) fn tok_matches_ident(tok: Token, src: &[u8], tb: &[u8]) -> bool {
    let len = (tok.end - tok.start) as usize;
    match tok.kind {
        T::Ident => len == tb.len() && &src[tok.start as usize..tok.end as usize] == tb,
        T::RawIdent => len == tb.len() + 2 && &src[tok.start as usize + 2..tok.end as usize] == tb,
        _ => false,
    }
}

pub(crate) fn collect_pat_bindings<'s>(tree: &SyntaxTree<'s>, pat_id: NodeId, out: &mut ParamBindings<'s>) {
    let nodes = &tree.nodes;
    let tokens = &tree.tokens;
    let src = tree.src;
    let mut idx = pat_id.0 as usize;
    let end = nodes[idx].subtree_end as usize;
    while idx < end {
        let n = nodes[idx];
        match n.kind {
            K::IdentPat => {
                if idx + 1 < end && nodes[idx + 1].kind == K::Name {
                    let nn = nodes[idx + 1];
                    if nn.first < nn.end {
                        let b = norm_ident(tokens[nn.first as usize].text(src));
                        if !b.is_empty() && b != "_" {
                            out.push_unique(b);
                        }
                    }
                    idx = nn.subtree_end as usize;
                    continue;
                }
                idx += 1;
            }
            K::RangePat | K::PathPat | K::LiteralPat | K::WildcardPat | K::RestPat | K::MacroPat | K::ConstBlockPat | K::Path => {
                idx = n.subtree_end as usize;
            }
            _ => idx += 1,
        }
    }
}

pub(crate) fn pat_binds_bytes(nodes: &[Node], tokens: &[Token], src: &[u8], pat_id: NodeId, tb: &[u8]) -> bool {
    let mut idx = pat_id.0 as usize;
    let end = nodes[idx].subtree_end as usize;
    while idx < end {
        let n = nodes[idx];
        match n.kind {
            K::IdentPat => {
                if idx + 1 < end && nodes[idx + 1].kind == K::Name {
                    let nn = nodes[idx + 1];
                    if nn.first < nn.end && tok_matches_ident(tokens[nn.first as usize], src, tb) {
                        return true;
                    }
                    idx = nn.subtree_end as usize;
                    continue;
                }
                idx += 1;
            }
            K::RangePat | K::PathPat | K::LiteralPat | K::WildcardPat | K::RestPat | K::MacroPat | K::ConstBlockPat | K::Path => {
                idx = n.subtree_end as usize;
            }
            _ => idx += 1,
        }
    }
    false
}

/// Flat preorder fast path for `self`: `self` is a strict keyword and cannot be shadowed by patterns
/// or value items, only cut off by nested items, types, and `const { ... }` blocks.
pub(crate) fn find_self_uses(tree: &SyntaxTree<'_>, body: NodeId, out: &mut ParamUses) {
    let nodes = &tree.nodes;
    let tokens = &tree.tokens;
    let src = tree.src.as_bytes();
    let end = nodes[body.0 as usize].subtree_end as usize;
    let mut idx = body.0 as usize + 1;
    while idx < end {
        let n = nodes[idx];
        match n.kind {
            K::PathExpr => {
                if n.end == n.first + 1 && tokens[n.first as usize].kind == T::SelfKw {
                    let t = tokens[n.first as usize];
                    out.uses.push(t.start as usize..t.end as usize);
                }
                idx = n.subtree_end as usize;
            }
            K::MacroCall | K::MacroPat => {
                let mut c = idx + 1;
                let mend = n.subtree_end as usize;
                while c < mend {
                    if nodes[c].kind == K::TokenTree {
                        scan_self_token_tree(tokens, &nodes[c], out);
                        break;
                    }
                    c = nodes[c].subtree_end as usize;
                }
                idx = mend;
            }
            K::BlockExpr if block_flavor_fast(tokens, src, &n) == BlockFlavor::Const => {
                idx = n.subtree_end as usize;
            }
            k if k.is_sema_skip() => {
                idx = n.subtree_end as usize;
            }
            _ => idx += 1,
        }
    }
}

/// Checks whether a block-scoped item introduces a value-namespace binding matching `target`.
pub(crate) fn item_shadows_value(nodes: &[Node], tokens: &[Token], src: &[u8], target: &[u8], item_idx: usize) -> bool {
    let n = nodes[item_idx];
    match n.kind {
        K::Fn | K::Const | K::Static => node_name_matches(nodes, tokens, src, target, item_idx),
        K::Struct => {
            if !node_name_matches(nodes, tokens, src, target, item_idx) {
                return false;
            }
            let mut c = item_idx + 1;
            let end = n.subtree_end as usize;
            while c < end {
                let cn = nodes[c];
                if cn.kind == K::TokenTree {
                    return !(cn.first < cn.end && tokens[cn.first as usize].kind == T::LBrace);
                }
                c = cn.subtree_end as usize;
            }
            true
        }
        _ => false,
    }
}

#[inline]
fn node_name_matches(nodes: &[Node], tokens: &[Token], src: &[u8], target: &[u8], id: usize) -> bool {
    let mut c = id + 1;
    let end = nodes[id].subtree_end as usize;
    while c < end {
        let cn = nodes[c];
        if cn.kind == K::Name {
            return cn.first < cn.end && tok_matches_ident(tokens[cn.first as usize], src, target);
        }
        c = cn.subtree_end as usize;
    }
    false
}

#[inline]
pub(crate) fn preceded_by_eq(tokens: &[Token], min_tok: u32, before_tok: u32) -> bool {
    let mut i = before_tok as usize;
    let lo = min_tok as usize;
    while i > lo {
        i -= 1;
        let t = tokens[i];
        if !t.is_trivia() {
            return t.kind == T::Eq;
        }
    }
    false
}
