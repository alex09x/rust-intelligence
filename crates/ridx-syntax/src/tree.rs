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

/// What the parser emits; the tree is built from these after parsing so a completed node can be wrapped in a new
/// parent (`a` becomes the left side of `a + b`) without moving nodes.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Event {
    /// Opens a node; `forward_parent` is the distance to a `Start` that must open before this one.
    Start {
        kind: SyntaxKind,
        forward_parent: u32,
    },
    Finish,
    /// Consumes the next significant token.
    Token,
}

impl<'s> SyntaxTree<'s> {
    pub(crate) fn build(src: &'s str, tokens: Vec<Token>, sig: &[u32], mut events: Vec<Event>) -> Self {
        let mut nodes: Vec<Node> = Vec::with_capacity(events.len() / 3 + 1);
        let mut stack: Vec<u32> = Vec::new();
        // Nodes on the stack from this depth on have not met their first token yet.
        let mut unstarted = 0usize;
        let mut next_sig = 0usize;
        let mut last_end = 0u32;
        let mut chain: Vec<SyntaxKind> = Vec::new();
        for i in 0..events.len() {
            match std::mem::replace(&mut events[i], Event::Start { kind: SyntaxKind::Tombstone, forward_parent: 0 }) {
                Event::Start { kind, forward_parent } => {
                    chain.clear();
                    chain.push(kind);
                    let mut fp = forward_parent;
                    let mut at = i;
                    while fp != 0 {
                        at += fp as usize;
                        match std::mem::replace(&mut events[at], Event::Start { kind: SyntaxKind::Tombstone, forward_parent: 0 }) {
                            Event::Start { kind, forward_parent } => {
                                chain.push(kind);
                                fp = forward_parent;
                            }
                            _ => unreachable!("forward parent is a Start"),
                        }
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
                Event::Finish => {
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
                Event::Token => {
                    let raw = sig[next_sig];
                    next_sig += 1;
                    for &id in &stack[unstarted..] {
                        nodes[id as usize].first = raw;
                    }
                    unstarted = stack.len();
                    last_end = raw + 1;
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

    pub fn root(&self) -> NodeId {
        NodeId(0)
    }

    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id.0 as usize]
    }

    pub fn kind(&self, id: NodeId) -> SyntaxKind {
        self.node(id).kind
    }

    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        let p = self.node(id).parent;
        (p != u32::MAX).then_some(NodeId(p))
    }

    /// The direct children of `id`, in source order.
    pub fn children(&self, id: NodeId) -> Children<'_, 's> {
        let n = self.node(id);
        Children { tree: self, next: id.0 + 1, end: n.subtree_end }
    }

    /// The first direct child of `id` of kind `kind`.
    pub fn child(&self, id: NodeId, kind: SyntaxKind) -> Option<NodeId> {
        self.children(id).find(|&c| self.kind(c) == kind)
    }

    /// `id` and its descendants, in preorder.
    pub fn descendants(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        (id.0..self.node(id).subtree_end).map(NodeId)
    }

    /// `id`'s ancestors from its parent up to the root.
    pub fn ancestors(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        std::iter::successors(self.parent(id), move |&p| self.parent(p))
    }

    /// The byte range of `id`'s significant tokens.
    pub fn range(&self, id: NodeId) -> std::ops::Range<usize> {
        let n = self.node(id);
        if n.first >= n.end {
            let at = self.tokens.get(n.first as usize).map_or(self.src.len(), |t| t.start as usize);
            return at..at;
        }
        self.tokens[n.first as usize].start as usize..self.tokens[n.end as usize - 1].end as usize
    }

    pub fn text(&self, id: NodeId) -> &'s str {
        &self.src[self.range(id)]
    }

    /// The tokens `id` covers, trivia included.
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
