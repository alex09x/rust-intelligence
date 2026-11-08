/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Partial-code receiver recovery (`x.` / `self.0.`), human-readable `{unknown}` type rendering,
//! and single-function targeted CST indexing (`Checkbox 3.5`).

use core::fmt::{self, Write};

use super::arena::InferTyArena;
use super::body_ctx::FnInferResult;
use super::infer_expr::is_expr_node;
use super::local_defs::{def_sort_key, LocalCstDefs};
use super::ty::{InferTyId, InferTyKind, TySliceRange};
use crate::name_res::{CrateDefMap, CrateId, FileId, ItemDb, LocalBindId};
use crate::syntax_kind::SyntaxKind as K;
use crate::token::TokenKind as T;
use crate::tree::{NodeId, SyntaxTree};

impl FnInferResult {
    #[inline]
    pub fn expr_ty(&self, node: NodeId) -> Option<InferTyId> {
        self.expr_tys.binary_search_by_key(&node, |&(n, _)| n).ok().map(|i| self.expr_tys[i].1)
    }

    #[inline]
    pub fn pat_ty(&self, node: NodeId) -> Option<InferTyId> {
        self.pat_tys.binary_search_by_key(&node, |&(n, _)| n).ok().map(|i| self.pat_tys[i].1)
    }

    #[inline]
    pub fn local_ty(&self, id: LocalBindId) -> Option<InferTyId> {
        self.local_tys.binary_search_by_key(&id, |&(l, _)| l).ok().map(|i| self.local_tys[i].1)
    }

    /// Returns the inferred type of the receiver expression immediately preceding a `.` ending at `dot_end`
    /// (`x.`, `self.0.`, `x.partial`, `x.foo(`, `x.await`), or `None` when no receiver expression exists there.
    pub fn receiver_ty_at_dot(&self, st: &SyntaxTree<'_>, dot_end: u32) -> Option<InferTyId> {
        if let Ok(idx) = self.incomplete_dots.binary_search_by_key(&dot_end, |&(off, _)| off) {
            return Some(self.incomplete_dots[idx].1);
        }
        let probe = dot_end.checked_sub(1)? as usize;
        let mut cur = self.fn_node;
        let mut best: Option<NodeId> = None;
        loop {
            let mut descended = false;
            for ch in st.children(cur) {
                let r = st.range(ch);
                if probe >= r.start && probe < r.end {
                    if matches!(st.kind(ch), K::FieldExpr | K::MethodCallExpr | K::AwaitExpr | K::Literal) {
                        best = Some(ch);
                    }
                    cur = ch;
                    descended = true;
                    break;
                }
            }
            if !descended {
                break;
            }
        }
        let mut cand = best?;
        loop {
            match st.kind(cand) {
                K::FieldExpr | K::MethodCallExpr | K::AwaitExpr => {
                    if st.own_tokens(cand).any(|(_, t)| t.kind == T::Dot && t.end == dot_end) {
                        let recv = st.children(cand).find(|&c| is_expr_node(st.kind(c)))?;
                        return self.expr_ty(recv);
                    }
                    if st.kind(cand) == K::FieldExpr {
                        if let Some((_, tok)) = st.own_tokens(cand).find(|(_, t)| t.kind == T::Float && dot_end > t.start && dot_end <= t.end) {
                            let prefix = &tok.text(st.src)[..(dot_end - tok.start) as usize];
                            if prefix.ends_with('.') {
                                return self.incomplete_dots.binary_search_by_key(&tok.end, |&(o, _)| o).ok().map(|i| self.incomplete_dots[i].1).or_else(|| self.expr_ty(cand));
                            }
                        }
                    }
                }
                K::Literal => {
                    // Bare integer receiver `0.` / `42.` at EOF lexes as `T::Float("0.")`.
                    if let Some((_, tok)) = st.own_tokens(cand).find(|(_, t)| t.kind == T::Float && t.end == dot_end) {
                        let txt = tok.text(st.src);
                        if let Some(stem) = txt.strip_suffix('.') {
                            if !stem.is_empty() && stem.bytes().all(|b| b.is_ascii_digit() || b == b'_') {
                                return Some(InferTyId::I32);
                            }
                        }
                    }
                }
                _ => {}
            }
            cand = st.parent(cand)?;
        }
    }
}

impl InferTyArena {
    /// Formats `ty` as a concise Rust-style diagnostic string (`InferTyKind::Unknown` -> `"{unknown}"`).
    pub fn format_ty(&self, ty: InferTyId) -> String {
        let mut s = String::with_capacity(32);
        let _ = self.write_ty(ty, &mut s);
        s
    }

    /// Writes `ty` into `out` without intermediate allocations (`InferTyKind::Unknown` -> `"{unknown}"`).
    pub fn write_ty(&self, ty: InferTyId, out: &mut impl Write) -> fmt::Result {
        self.write_ty_depth(ty, out, 0)
    }

    fn write_ty_depth(&self, ty: InferTyId, out: &mut impl Write, depth: u8) -> fmt::Result {
        if !ty.is_some() || depth > 32 {
            return out.write_str("{unknown}");
        }
        match self.kind(ty) {
            InferTyKind::Unknown => out.write_str("{unknown}"),
            InferTyKind::Never => out.write_str("!"),
            InferTyKind::Prim(p) => out.write_str(p.name()),
            InferTyKind::Tuple(r) => {
                out.write_char('(')?;
                for (i, &elem) in self.slice(r).iter().enumerate() {
                    if i > 0 {
                        out.write_str(", ")?;
                    }
                    self.write_ty_depth(elem, out, depth + 1)?;
                }
                if r.len == 1 {
                    out.write_char(',')?;
                }
                out.write_char(')')
            }
            InferTyKind::Ref { is_mut, inner } => {
                out.write_str(if is_mut { "&mut " } else { "&" })?;
                self.write_ty_depth(inner, out, depth + 1)
            }
            InferTyKind::Ptr { is_mut, inner } => {
                out.write_str(if is_mut { "*mut " } else { "*const " })?;
                self.write_ty_depth(inner, out, depth + 1)
            }
            InferTyKind::Slice(elem) => {
                out.write_char('[')?;
                self.write_ty_depth(elem, out, depth + 1)?;
                out.write_char(']')
            }
            InferTyKind::Array { elem, len } => {
                out.write_char('[')?;
                self.write_ty_depth(elem, out, depth + 1)?;
                if len == u64::MAX {
                    out.write_str("; _]")
                } else {
                    write!(out, "; {len}]")
                }
            }
            InferTyKind::Adt { def, args } => {
                write!(out, "Adt({}:{})", def.krate.0, def.index)?;
                self.write_angle_args(args, out, depth + 1)
            }
            InferTyKind::FnPtr { sig } => {
                let s = self.slice(sig);
                out.write_str("fn(")?;
                let n_in = s.len().saturating_sub(1);
                for (i, &inp) in s[..n_in].iter().enumerate() {
                    if i > 0 {
                        out.write_str(", ")?;
                    }
                    self.write_ty_depth(inp, out, depth + 1)?;
                }
                out.write_str(") -> ")?;
                self.write_ty_depth(*s.last().unwrap_or(&InferTyId::UNKNOWN), out, depth + 1)
            }
            InferTyKind::FnDef { def, args } => {
                write!(out, "fn#{}:{}", def.krate.0, def.index)?;
                self.write_angle_args(args, out, depth + 1)
            }
            InferTyKind::Closure { id, sig } => {
                write!(out, "{{closure#{id}}}")?;
                self.write_angle_args(sig, out, depth + 1)
            }
            InferTyKind::Param { idx, .. } => write!(out, "Param({idx})"),
            InferTyKind::Alias { assoc_def, self_ty, args, .. } => {
                out.write_char('<')?;
                self.write_ty_depth(self_ty, out, depth + 1)?;
                write!(out, ">::Assoc({}:{})", assoc_def.krate.0, assoc_def.index)?;
                self.write_angle_args(args, out, depth + 1)
            }
            InferTyKind::Dyn { principal, args } => {
                write!(out, "dyn Trait({}:{})", principal.krate.0, principal.index)?;
                self.write_angle_args(args, out, depth + 1)
            }
            InferTyKind::Opaque { def, args } => {
                write!(out, "impl Trait({}:{})", def.krate.0, def.index)?;
                self.write_angle_args(args, out, depth + 1)
            }
            InferTyKind::Infer(v) => write!(out, "?{}", v.0),
        }
    }

    fn write_angle_args(&self, r: TySliceRange, out: &mut impl Write, depth: u8) -> fmt::Result {
        if r.is_empty() {
            return Ok(());
        }
        out.write_char('<')?;
        for (i, &arg) in self.slice(r).iter().enumerate() {
            if i > 0 {
                out.write_str(", ")?;
            }
            self.write_ty_depth(arg, out, depth)?;
        }
        out.write_char('>')
    }
}

impl LocalCstDefs {
    /// Indexes only the top-level ancestor item containing `target_fn` when `file` has a clean `item_db`
    /// (`O(fn_size)` instead of `O(file_size)`), falling back to full-file scan for unindexed/dirty files.
    pub fn index_target_fn_with_item_db(&mut self, st: &SyntaxTree<'_>, def_map: Option<&CrateDefMap>, item_db: Option<&dyn ItemDb>, file: FileId, target_fn: NodeId) {
        let clean_db = item_db.is_some() && def_map.is_some_and(|dm| !dm.is_dirty_file(file));
        if !clean_db {
            self.index_file_with_item_db(st, def_map, item_db, file);
            return;
        }
        self.clear();
        self.file_krate = def_map.map_or(CrateId::NONE, |dm| dm.krate);
        self.clean_indexed = true;
        let ts = target_fn.0;
        let te = st.node(target_fn).subtree_end;
        self.scan_container(st, st.root(), None, def_map, item_db, file, true, Some((ts, te)));
        for (idx, it) in self.items.iter().enumerate() {
            let i32_idx = idx as u32;
            self.by_def.push((def_sort_key(it.def), i32_idx));
            self.by_node.push((it.node.0, i32_idx));
        }
        self.by_def.sort_unstable_by_key(|&(k, _)| k);
        self.by_node.sort_unstable_by_key(|&(n, _)| n);
    }
}
