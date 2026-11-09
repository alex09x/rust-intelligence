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
use super::autoderef::{build_autoderef_steps_with_owner, MAX_AUTODEREF_STEPS};
use super::body_ctx::{BodyInferCtx, FnInferResult, Runner};
use super::db::TyDefDb;
use super::infer_expr::is_expr_node;
use super::local_defs::{def_sort_key, LocalCstDefs};
use super::ty::{InferTyId, InferTyKind, TySliceRange};
use crate::name_res::{CrateDefMap, CrateId, FileId, FileNameRes, GlobalDefId, ItemDb, ItemKind, LocalBindId};
use crate::sema_fn::norm_ident;
use crate::syntax_kind::SyntaxKind as K;
use crate::token::TokenKind as T;
use crate::tree::{NodeId, SyntaxTree};

impl Default for FnInferResult {
    fn default() -> Self {
        Self::new()
    }
}

impl FnInferResult {
    pub fn new() -> Self {
        Self {
            fn_node: NodeId(0),
            ret_ty: InferTyId::UNKNOWN,
            body_ty: InferTyId::UNKNOWN,
            expr_tys: Vec::new(),
            macro_expr_tys: Vec::new(),
            pat_tys: Vec::new(),
            local_tys: Vec::new(),
            dot_refs: Vec::new(),
            incomplete_dots: Vec::new(),
        }
    }

    #[inline]
    pub fn clear(&mut self) {
        self.fn_node = NodeId(0);
        self.ret_ty = InferTyId::UNKNOWN;
        self.body_ty = InferTyId::UNKNOWN;
        self.expr_tys.clear();
        self.macro_expr_tys.clear();
        self.pat_tys.clear();
        self.local_tys.clear();
        self.dot_refs.clear();
        self.incomplete_dots.clear();
    }
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
            InferTyKind::Closure { sig, .. } => {
                let s = self.slice(sig);
                out.write_str("impl Fn(")?;
                let n_in = s.len().saturating_sub(1);
                for (i, &inp) in s[..n_in].iter().enumerate() {
                    if i > 0 {
                        out.write_str(", ")?;
                    }
                    self.write_ty_depth(inp, out, depth + 1)?;
                }
                out.write_char(')')?;
                let ret = *s.last().unwrap_or(&InferTyId::UNKNOWN);
                if ret != InferTyId::UNIT {
                    out.write_str(" -> ")?;
                    self.write_ty_depth(ret, out, depth + 1)?;
                }
                Ok(())
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
        let clean_db = match (item_db, def_map) {
            (Some(idb), Some(dm)) => idb.crate_root_def(dm.krate).is_some() && !dm.is_dirty_file(file),
            _ => false,
        };
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
        self.finalize_macro_tables();
    }

    /// Appends the normalized names of all CST `AssocFn` items recorded in `self`.
    pub fn collect_cst_method_names<'s>(&self, st: &SyntaxTree<'s>, out: &mut Vec<&'s str>) {
        for it in &self.items {
            if it.kind == ItemKind::AssocFn {
                if let Some(nn) = st.child(it.node, K::Name) {
                    out.push(norm_ident(st.text(nn)));
                }
            }
        }
    }
}

impl BodyInferCtx {
    /// Appends `(name_idx, method_def)` for every method in `candidate_names` valid for dot-completion
    /// on `recv_ty` at `at_node` inside `fn_node`, preserving `self.table.arena` without clearing caller `InferTyId`s.
    #[allow(clippy::too_many_arguments)]
    pub fn collect_all_completion_methods(
        &mut self,
        st: &SyntaxTree<'_>,
        file_res: &FileNameRes,
        db: &dyn TyDefDb,
        local_defs: &LocalCstDefs,
        fn_node: NodeId,
        at_node: NodeId,
        recv_ty: InferTyId,
        candidate_names: &[&str],
        out: &mut Vec<(u32, GlobalDefId)>,
    ) {
        out.clear();
        let top_snap = self.table.start_snapshot();
        self.fn_has_inner_trait_scopes = file_res.has_inner_trait_scopes(fn_node, st.node(fn_node).subtree_end);
        self.fn_scope_traits_ready = false;
        let impl_node = st.ancestors(fn_node).find(|&a| matches!(st.kind(a), K::Impl | K::Trait));
        let (impl_gc, self_ty) = local_defs.owner_generics_and_self_ty(impl_node, st, file_res, db, &mut self.table);
        let fn_def = local_defs.fn_def_for_node(fn_node);
        let mut dummy_out = FnInferResult::new();
        let mut env = Runner {
            ctx: self,
            st,
            file_res,
            root_st: st,
            root_file_res: file_res,
            db,
            local_defs,
            fn_node,
            root_fn_node: fn_node,
            fn_def,
            impl_node,
            root_impl_node: impl_node,
            impl_gc,
            self_ty,
            loc_lo: 0,
            loc_hi: 0,
            expr_depth: 0,
            sub: None,
            out: &mut dummy_out,
        };
        let mut steps = [InferTyId::UNKNOWN; MAX_AUTODEREF_STEPS];
        let owner = env.norm_owner();
        let step_len = build_autoderef_steps_with_owner(&mut env.ctx.table, db, local_defs, st, file_res, owner, recv_ty, &mut steps);
        let mut cands = core::mem::take(&mut env.ctx.method_cands_buf);
        let mut scope_traits = core::mem::take(&mut env.ctx.scope_traits_buf);
        for (n_idx, &mname) in candidate_names.iter().enumerate() {
            cands.clear();
            scope_traits.clear();
            let _ = env.gather_inherent_cands(mname, &steps[..step_len], &mut cands);
            env.gather_trait_and_bound_cands(at_node, mname, &steps[..step_len], &mut scope_traits, &mut cands);
            for &st_ty in &steps[..step_len] {
                let mut rcvrs = [InferTyId::NONE; 7];
                let mut rlen = 0usize;
                rcvrs[rlen] = st_ty;
                rlen += 1;
                if let InferTyKind::Ref { is_mut: true, inner } = env.ctx.table.arena.kind(st_ty) {
                    rcvrs[rlen] = env.ctx.table.arena.mk_ref(false, inner);
                    rlen += 1;
                }
                rcvrs[rlen] = env.ctx.table.arena.mk_ref(false, st_ty);
                rlen += 1;
                rcvrs[rlen] = env.ctx.table.arena.mk_ref(true, st_ty);
                rlen += 1;
                if let InferTyKind::Array { elem, .. } = env.ctx.table.arena.kind(st_ty) {
                    let sl = env.ctx.table.arena.mk_slice(elem);
                    rcvrs[rlen] = sl;
                    rlen += 1;
                    rcvrs[rlen] = env.ctx.table.arena.mk_ref(false, sl);
                    rlen += 1;
                    rcvrs[rlen] = env.ctx.table.arena.mk_ref(true, sl);
                    rlen += 1;
                }
                for cand in &cands {
                    if rcvrs[..rlen].iter().any(|&rcvr| env.cand_matches_completion_rcvr(cand, rcvr)) {
                        out.push((n_idx as u32, cand.method_def));
                    }
                }
            }
        }
        env.ctx.method_cands_buf = cands;
        env.ctx.scope_traits_buf = scope_traits;
        self.table.rollback_to(top_snap);
    }
}
