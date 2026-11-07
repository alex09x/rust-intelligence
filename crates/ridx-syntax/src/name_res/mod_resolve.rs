/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Inline and out-of-line `mod` item handling (`rustc` `DirOwnership`, `#[path]`, `foo.rs` vs `foo/mod.rs`).

use super::attr::{eval_inner_attrs, eval_outer_attrs, extract_visibility, EffectiveAttrs};
use super::db::{FileId, ModuleId};
use super::mod_builder::Builder;
use super::mod_macro_scope::{finish_macro_scope, MacroEnv};
use super::mod_tree::{join_norm_into, parent_dir_str, ModDiag, MOD_FLAG_INLINE, MOD_FLAG_MACRO_USE, MOD_FLAG_NON_MOD_RS, MOD_FLAG_NO_IMPLICIT_PRELUDE, MOD_FLAG_UNRESOLVED};
use crate::parser::parse;
use crate::sema_fn::norm_ident;
use crate::syntax_kind::SyntaxKind as K;
use crate::tree::{NodeId, SyntaxTree};

impl<'a, 't> Builder<'a, 't> {
    pub(super) fn handle_mod_item(&mut self, parent_id: ModuleId, decl_file: FileId, st: &SyntaxTree<'_>, mod_node: NodeId, macro_env: &mut MacroEnv, mbe_depth: u32) {
        let outer = eval_outer_attrs(st, mod_node, self.db, self.krate);
        if !outer.enabled {
            return;
        }
        let Some(name_node) = st.child(mod_node, K::Name) else {
            return;
        };
        let mod_name = norm_ident(st.text(name_node));
        if mod_name.is_empty() {
            return;
        }
        if self.raw_edges.iter().any(|(p, e)| *p == parent_id && self.tree.child_name(e) == mod_name) {
            self.tree.diagnostics.push(ModDiag::DuplicateModDecl { parent: parent_id, name: mod_name.to_string() });
            return;
        }

        let (vis, _) = extract_visibility(st, mod_node);
        let decl_range = st.range(mod_node);
        let p_mod = self.tree.modules[parent_id.0 as usize];
        let p_dir_range = p_mod.dir_start as usize..p_mod.dir_end as usize;

        if let Some(item_list) = st.child(mod_node, K::ItemList) {
            let mut inner = EffectiveAttrs::default();
            eval_inner_attrs(st, item_list, self.db, self.krate, &mut inner);
            if !inner.enabled {
                return;
            }
            let p_sub_dir = &self.tree.strings[p_dir_range];
            let base = if outer.explicit_path.is_some() && p_mod.is_non_mod_rs() { parent_dir_str(p_sub_dir) } else { p_sub_dir };
            let rel = outer.explicit_path.as_deref().unwrap_or(mod_name);
            join_norm_into(&mut self.scratch_dir, base, rel, "");
            let mut flags = MOD_FLAG_INLINE | (p_mod.flags & MOD_FLAG_NO_IMPLICIT_PRELUDE);
            if outer.no_implicit_prelude || inner.no_implicit_prelude {
                flags |= MOD_FLAG_NO_IMPLICIT_PRELUDE;
            }
            if outer.macro_use {
                flags |= MOD_FLAG_MACRO_USE;
            }
            let (inline_node, d_start, d_end) = if mbe_depth == 0 { (mod_node.0, decl_range.start as u32, decl_range.end as u32) } else { (u32::MAX, u32::MAX, u32::MAX) };
            let child_id = self.alloc_module(parent_id, decl_file, decl_file, inline_node, d_start, d_end, mod_name, vis, flags);
            let env_mark = macro_env.len();
            self.scan_container(child_id, decl_file, st, item_list, macro_env, mbe_depth);
            finish_macro_scope(&outer, env_mark, macro_env, &self.macro_defs);
            return;
        }

        let (resolved_file, extra_flags) = self.resolve_out_of_line_file(parent_id, p_dir_range, p_mod.is_non_mod_rs(), mod_name, outer.explicit_path.as_deref());

        let Some(target_file) = resolved_file else {
            let flags = MOD_FLAG_UNRESOLVED | extra_flags | if outer.no_implicit_prelude { MOD_FLAG_NO_IMPLICIT_PRELUDE } else { 0 };
            self.alloc_module(parent_id, FileId::NONE, decl_file, u32::MAX, decl_range.start as u32, decl_range.end as u32, mod_name, vis, flags);
            return;
        };

        if target_file == self.root_file || self.in_flight_files.contains(&target_file) {
            self.tree.diagnostics.push(ModDiag::CyclicModFile { parent: parent_id, name: mod_name.to_string(), file: target_file });
            return;
        }

        let child_src = self.sources.file_text(target_file);
        let owned_child;
        let child_st: &SyntaxTree<'_> = match (self.borrow_cached)(target_file) {
            Some(borrowed) => borrowed,
            None => {
                owned_child = parse(child_src);
                &owned_child
            }
        };
        let mut inner = EffectiveAttrs::default();
        eval_inner_attrs(child_st, child_st.root(), self.db, self.krate, &mut inner);
        if !inner.enabled {
            return;
        }
        let mut flags = extra_flags | (p_mod.flags & MOD_FLAG_NO_IMPLICIT_PRELUDE);
        if outer.no_implicit_prelude || inner.no_implicit_prelude {
            flags |= MOD_FLAG_NO_IMPLICIT_PRELUDE;
        }
        if outer.macro_use {
            flags |= MOD_FLAG_MACRO_USE;
        }
        let child_id = self.alloc_module(parent_id, target_file, decl_file, u32::MAX, decl_range.start as u32, decl_range.end as u32, mod_name, vis, flags);
        self.in_flight_files.push(target_file);
        let env_mark = macro_env.len();
        self.scan_container(child_id, target_file, child_st, child_st.root(), macro_env, 0);
        self.in_flight_files.pop();
        finish_macro_scope(&outer, env_mark, macro_env, &self.macro_defs);
    }

    fn resolve_out_of_line_file(
        &mut self,
        parent_id: ModuleId,
        p_dir_range: std::ops::Range<usize>,
        parent_is_non_mod_rs: bool,
        mod_name: &str,
        explicit_path: Option<&str>,
    ) -> (Option<FileId>, u8) {
        let p_sub_dir = &self.tree.strings[p_dir_range];
        if let Some(explicit) = explicit_path {
            let attr_base = if parent_is_non_mod_rs { parent_dir_str(p_sub_dir) } else { p_sub_dir };
            join_norm_into(&mut self.scratch_a, attr_base, explicit, "");
            self.scratch_dir.clear();
            self.scratch_dir.push_str(parent_dir_str(&self.scratch_a));
            return match self.sources.resolve_path(&self.scratch_a) {
                Some(fid) => (Some(fid), 0),
                None => {
                    self.tree.diagnostics.push(ModDiag::UnresolvedModFile {
                        parent: parent_id,
                        name: mod_name.to_string(),
                        tried_primary: self.scratch_a.clone(),
                        tried_mod_rs: None,
                    });
                    (None, 0)
                }
            };
        }

        join_norm_into(&mut self.scratch_a, p_sub_dir, mod_name, ".rs");
        join_norm_into(&mut self.scratch_b, p_sub_dir, mod_name, "/mod.rs");
        join_norm_into(&mut self.scratch_dir, p_sub_dir, mod_name, "");
        match (self.sources.resolve_path(&self.scratch_a), self.sources.resolve_path(&self.scratch_b)) {
            (Some(f1), Some(f2)) => {
                if f1 != f2 {
                    self.tree.diagnostics.push(ModDiag::AmbiguousModFile { parent: parent_id, name: mod_name.to_string(), chosen: f1, alternate: f2 });
                }
                (Some(f1), MOD_FLAG_NON_MOD_RS)
            }
            (Some(f1), None) => (Some(f1), MOD_FLAG_NON_MOD_RS),
            (None, Some(f2)) => (Some(f2), 0),
            (None, None) => {
                self.tree.diagnostics.push(ModDiag::UnresolvedModFile {
                    parent: parent_id,
                    name: mod_name.to_string(),
                    tried_primary: self.scratch_a.clone(),
                    tried_mod_rs: Some(self.scratch_b.clone()),
                });
                (None, 0)
            }
        }
    }
}
