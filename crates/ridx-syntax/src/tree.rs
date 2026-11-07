/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! A lossless syntax tree stored flat: nodes in preorder, each covering a range of lexer tokens. Trivia between
//! significant tokens belongs to the innermost node covering both neighbours, and the root covers every token, so
//! the source is recovered from the tokens alone.

use crate::syntax_kind::SyntaxKind;
use crate::token::Token;

/// Index of a node in [`SyntaxTree::nodes`]; the root is `NodeId(0)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(pub u32);

/// One node: its kind, the tokens it covers (`first..end`, both significant at the edges unless empty), its parent,
/// and the index just past its last descendant.
#[derive(Clone, Copy, Debug)]
pub struct Node {
    pub kind: SyntaxKind,
    pub first: u32,
    pub end: u32,
    pub parent: u32,
    pub subtree_end: u32,
}

/// The tree of one source text.
pub struct SyntaxTree<'s> {
    pub src: &'s str,
    pub tokens: Vec<Token>,
    pub nodes: Vec<Node>,
}

/// Compact 4-byte parser event (`Start`, `Finish`, or `Token`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Event {
    pub(crate) kind: SyntaxKind,
    fp_lo: u8,
    fp_hi: u16,
}

const _: () = assert!(core::mem::size_of::<Event>() == 4);

impl Event {
    pub(crate) const FINISH: Self = Self { kind: SyntaxKind::Tombstone, fp_lo: 0, fp_hi: u16::MAX };
    pub(crate) const TOKEN: Self = Self { kind: SyntaxKind::Tombstone, fp_lo: 1, fp_hi: u16::MAX };
    pub(crate) const TOMBSTONE: Self = Self { kind: SyntaxKind::Tombstone, fp_lo: 0, fp_hi: 0 };

    #[inline]
    pub(crate) const fn start(kind: SyntaxKind) -> Self {
        Self { kind, fp_lo: 0, fp_hi: 0 }
    }

    #[inline]
    pub(crate) fn set_fp(&mut self, fp: u32) {
        debug_assert!(fp < 0xFFFE_FFFF);
        self.fp_lo = (fp & 0xFF) as u8;
        self.fp_hi = (fp >> 8) as u16;
    }

    #[inline]
    pub(crate) const fn fp(self) -> u32 {
        (self.fp_lo as u32) | ((self.fp_hi as u32) << 8)
    }

    #[inline]
    pub(crate) const fn is_special(self) -> bool {
        self.fp_hi == u16::MAX
    }

    #[inline]
    pub(crate) const fn is_token(self) -> bool {
        self.fp_lo != 0
    }
}

impl<'s> SyntaxTree<'s> {
    pub(crate) fn build(src: &'s str, tokens: Vec<Token>, sig: &[u32], mut events: Vec<Event>, nodes_cap: usize) -> Self {
        let mut nodes: Vec<Node> = Vec::with_capacity(nodes_cap);
        let mut stack: Vec<u32> = Vec::with_capacity(64);
        // Nodes on the stack from this depth on have not met their first token yet.
        let mut unstarted = 0usize;
        let mut next_sig = 0usize;
        let mut last_end = 0u32;
        let mut chain: Vec<SyntaxKind> = Vec::with_capacity(16);
        for i in 0..events.len() {
            let ev = events[i];
            if ev.is_special() {
                if ev.is_token() {
                    let raw = sig[next_sig];
                    next_sig += 1;
                    for &id in &stack[unstarted..] {
                        nodes[id as usize].first = raw;
                    }
                    unstarted = stack.len();
                    last_end = raw + 1;
                } else {
                    let id = stack.pop().expect("balanced events") as usize;
                    unstarted = unstarted.min(stack.len());
                    let len = nodes.len() as u32;
                    let n = &mut nodes[id];
                    if n.first == u32::MAX {
                        n.first = last_end;
                    }
                    n.end = last_end.max(n.first);
                    n.subtree_end = len;
                }
            } else {
                let fp0 = ev.fp();
                if fp0 == 0 {
                    if ev.kind != SyntaxKind::Tombstone {
                        let parent = stack.last().copied().unwrap_or(u32::MAX);
                        stack.push(nodes.len() as u32);
                        nodes.push(Node { kind: ev.kind, first: u32::MAX, end: 0, parent, subtree_end: 0 });
                    }
                } else {
                    events[i] = Event::TOMBSTONE;
                    chain.clear();
                    chain.push(ev.kind);
                    let mut fp = fp0;
                    let mut at = i;
                    while fp != 0 {
                        at += fp as usize;
                        let target = events[at];
                        events[at] = Event::TOMBSTONE;
                        chain.push(target.kind);
                        fp = target.fp();
                    }
                    for &kind in chain.iter().rev() {
                        if kind == SyntaxKind::Tombstone {
                            continue;
                        }
                        let parent = stack.last().copied().unwrap_or(u32::MAX);
                        stack.push(nodes.len() as u32);
                        nodes.push(Node { kind, first: u32::MAX, end: 0, parent, subtree_end: 0 });
                    }
                }
            }
        }
        debug_assert!(stack.is_empty());
        if let Some(root) = nodes.first_mut() {
            root.first = 0;
            root.end = tokens.len() as u32;
        }
        SyntaxTree { src, tokens, nodes }
    }

    #[inline]
    pub fn root(&self) -> NodeId {
        NodeId(0)
    }

    #[inline]
    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id.0 as usize]
    }

    #[inline]
    pub fn kind(&self, id: NodeId) -> SyntaxKind {
        self.node(id).kind
    }

    #[inline]
    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        let p = self.node(id).parent;
        (p != u32::MAX).then_some(NodeId(p))
    }

    /// The direct children of `id`, in source order.
    #[inline]
    pub fn children(&self, id: NodeId) -> Children<'_, 's> {
        let n = self.node(id);
        Children { tree: self, next: id.0 + 1, end: n.subtree_end }
    }

    /// The first direct child of `id` of kind `kind`.
    #[inline]
    pub fn child(&self, id: NodeId, kind: SyntaxKind) -> Option<NodeId> {
        self.children(id).find(|&c| self.kind(c) == kind)
    }

    /// `id` and its descendants, in preorder.
    #[inline]
    pub fn descendants(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        (id.0..self.node(id).subtree_end).map(NodeId)
    }

    /// `id`'s ancestors from its parent up to the root.
    #[inline]
    pub fn ancestors(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        std::iter::successors(self.parent(id), move |&p| self.parent(p))
    }

    /// The byte range of `id`'s significant tokens.
    #[inline]
    pub fn range(&self, id: NodeId) -> std::ops::Range<usize> {
        let n = self.node(id);
        if n.first >= n.end {
            let at = self.tokens.get(n.first as usize).map_or(self.src.len(), |t| t.start as usize);
            return at..at;
        }
        self.tokens[n.first as usize].start as usize..self.tokens[n.end as usize - 1].end as usize
    }

    #[inline]
    pub fn text(&self, id: NodeId) -> &'s str {
        &self.src[self.range(id)]
    }

    /// The tokens `id` covers, trivia included.
    #[inline]
    pub fn tokens_of(&self, id: NodeId) -> &[Token] {
        let n = self.node(id);
        &self.tokens[n.first as usize..n.end.max(n.first) as usize]
    }

    /// The significant tokens directly in `id`, not inside a child node, with their indices.
    pub fn own_tokens(&self, id: NodeId) -> impl Iterator<Item = (usize, Token)> + '_ {
        let n = *self.node(id);
        let mut kids = self.children(id).map(|c| *self.node(c)).peekable();
        let mut i = n.first as usize;
        std::iter::from_fn(move || loop {
            if i >= n.end as usize {
                return None;
            }
            if let Some(k) = kids.peek() {
                if i >= k.first as usize && k.end > k.first {
                    i = k.end as usize;
                    kids.next();
                    continue;
                }
                if k.end <= k.first && i >= k.first as usize {
                    kids.next();
                    continue;
                }
            }
            let t = self.tokens[i];
            i += 1;
            if !t.is_trivia() {
                return Some((i - 1, t));
            }
        })
    }

    /// The innermost node whose significant range contains byte `offset`.
    pub fn covering(&self, offset: usize) -> NodeId {
        let mut id = self.root();
        'down: loop {
            for c in self.children(id) {
                let r = self.range(c);
                if r.start <= offset && offset < r.end {
                    id = c;
                    continue 'down;
                }
            }
            return id;
        }
    }

    /// The innermost node of kind `kind` containing byte `offset`.
    pub fn covering_of(&self, offset: usize, kind: SyntaxKind) -> Option<NodeId> {
        let id = self.covering(offset);
        std::iter::once(id).chain(self.ancestors(id)).find(|&a| self.kind(a) == kind)
    }
}

/// Iterator over the direct children of a node.
pub struct Children<'t, 's> {
    tree: &'t SyntaxTree<'s>,
    next: u32,
    end: u32,
}

impl Iterator for Children<'_, '_> {
    type Item = NodeId;

    fn next(&mut self) -> Option<NodeId> {
        if self.next >= self.end {
            return None;
        }
        let id = self.next;
        self.next = self.tree.nodes[id as usize].subtree_end;
        Some(NodeId(id))
    }
}
