/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! A compact text form of a tree for tests and debugging: `Fn(Name ParamList(Param(IdentPat PathType)) BlockExpr)`.

use crate::tree::{NodeId, SyntaxTree};

impl SyntaxTree<'_> {
    /// The subtree of `id` as nested kinds; `with_text` appends the source of nodes without children.
    pub fn dump(&self, id: NodeId, with_text: bool) -> String {
        let mut out = String::new();
        self.dump_into(id, with_text, &mut out);
        out
    }

    fn dump_into(&self, id: NodeId, with_text: bool, out: &mut String) {
        out.push_str(&format!("{:?}", self.kind(id)));
        let mut kids = self.children(id).peekable();
        if kids.peek().is_some() {
            out.push('(');
            let mut first = true;
            for c in kids {
                if !first {
                    out.push(' ');
                }
                first = false;
                self.dump_into(c, with_text, out);
            }
            out.push(')');
        } else if with_text {
            out.push_str(&format!("{:?}", self.text(id)));
        }
    }

    /// The `Error` nodes of the tree.
    pub fn errors(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.descendants(self.root()).filter(|&n| self.kind(n) == crate::SyntaxKind::Error)
    }
}
