/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Arena builder for [`super::mod_tree::ModuleTree`].

use super::attr::{eval_inner_attrs, EffectiveAttrs};
use super::db::{CrateId, FileId, ItemDb, ModuleId, SourceDb, VisKind};
use super::mbe_expand::MAX_MBE_DEPTH;
use super::mod_macro_scope::{LazyMacro, MacroEnv};
use super::mod_tree::{parent_dir_str, ModChild, ModDiag, ModuleData, ModuleTree, MOD_FLAG_NO_IMPLICIT_PRELUDE};
use crate::parser::parse;
use crate::syntax_kind::SyntaxKind as K;
use crate::tree::{NodeId, SyntaxTree};

pub(super) struct Builder<'a, 't> {
    pub(super) db: &'a dyn ItemDb,
    pub(super) sources: &'a dyn SourceDb,
    pub(super) krate: CrateId,
    pub(super) borrow_cached: &'a mut dyn FnMut(FileId) -> Option<&'t SyntaxTree<'t>>,
    pub(super) tree: ModuleTree,
    pub(super) root_file: FileId,
    pub(super) raw_edges: Vec<(ModuleId, ModChild)>,
    pub(super) in_flight_files: Vec<FileId>,
    pub(super) macro_defs: Vec<LazyMacro<'a>>,
    pub(super) exported_macros: MacroEnv,
    pub(super) any_def_has_mod: bool,
    pub(super) scratch_a: String,
    pub(super) scratch_b: String,
    pub(super) scratch_dir: String,
}

impl<'a, 't> Builder<'a, 't> {
    pub(super) fn new(db: &'a dyn ItemDb, sources: &'a dyn SourceDb, krate: CrateId, borrow_cached: &'a mut dyn FnMut(FileId) -> Option<&'t SyntaxTree<'t>>) -> Self {
        Self {
            db,
            sources,
            krate,
            borrow_cached,
            tree: ModuleTree {
                krate,
                edition: db.crate_edition(krate),
                no_std: false,
                no_core: false,
                modules: Vec::with_capacity(4),
                children: Vec::new(),
                file_to_modules: Vec::with_capacity(4),
                strings: String::with_capacity(128),
                diagnostics: Vec::new(),
            },
            root_file: FileId::NONE,
            raw_edges: Vec::new(),
            in_flight_files: Vec::new(),
            macro_defs: Vec::new(),
            exported_macros: Vec::new(),
            any_def_has_mod: false,
            scratch_a: String::new(),
            scratch_b: String::new(),
            scratch_dir: String::new(),
        }
    }

    pub(super) fn run(mut self, root_file: FileId) -> ModuleTree {
        self.build_root(root_file);
        self.finalize()
    }

    #[inline]
    pub(super) fn push_str(&mut self, s: &str) -> (u32, u32) {
        let start = self.tree.strings.len() as u32;
        self.tree.strings.push_str(s);
        (start, self.tree.strings.len() as u32)
    }

    fn build_root(&mut self, root_file: FileId) {
        self.root_file = root_file;
        let root_path = self.sources.file_path(root_file);
        let root_dir = parent_dir_str(root_path);
        let src = self.sources.file_text(root_file);
        let owned;
        let st: &SyntaxTree<'_> = match (self.borrow_cached)(root_file) {
            Some(borrowed) => borrowed,
            None => {
                owned = parse(src);
                &owned
            }
        };

        let mut inner = EffectiveAttrs::default();
        eval_inner_attrs(st, st.root(), self.db, self.krate, &mut inner);
        self.tree.no_std = inner.no_std;
        self.tree.no_core = inner.no_core;
        let (cp_s, cp_e) = self.push_str("crate");
        let (dir_s, dir_e) = self.push_str(root_dir);
        let flags = if inner.no_implicit_prelude { MOD_FLAG_NO_IMPLICIT_PRELUDE } else { 0 };
        self.tree.modules.push(ModuleData {
            id: ModuleId::ROOT,
            parent: ModuleId::NONE,
            file: root_file,
            decl_file: root_file,
            inline_node: u32::MAX,
            decl_start: 0,
            decl_end: src.len() as u32,
            name_start: 0,
            name_end: 0,
            crate_path_start: cp_s,
            crate_path_end: cp_e,
            dir_start: dir_s,
            dir_end: dir_e,
            children_start: 0,
            children_len: 0,
            vis: VisKind::Public,
            flags,
        });
        self.tree.file_to_modules.push((root_file, ModuleId::ROOT));
        if !inner.enabled {
            self.tree.diagnostics.push(ModDiag::DisabledCrateRoot { file: root_file });
            return;
        }
        let mut env = MacroEnv::new();
        self.scan_container(ModuleId::ROOT, root_file, st, st.root(), &mut env, 0);
    }

    pub(super) fn scan_container(&mut self, mod_id: ModuleId, file: FileId, st: &SyntaxTree<'_>, container: NodeId, macro_env: &mut MacroEnv, mbe_depth: u32) {
        for item in st.children(container) {
            match st.kind(item) {
                K::MacroRules => self.record_macro_rules(file, st, item, macro_env, mbe_depth),
                K::Module => self.handle_mod_item(mod_id, file, st, item, macro_env, mbe_depth),
                K::MacroCall if mbe_depth < MAX_MBE_DEPTH => {
                    self.try_expand_macro_call(mod_id, file, st, item, macro_env, mbe_depth);
                }
                _ => {}
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn alloc_module(
        &mut self,
        parent: ModuleId,
        file: FileId,
        decl_file: FileId,
        inline_node: u32,
        decl_start: u32,
        decl_end: u32,
        name: &str,
        vis: VisKind,
        flags: u8,
    ) -> ModuleId {
        let id = ModuleId(self.tree.modules.len() as u32);
        let (ns, ne) = self.push_str(name);
        let (cs, ce) = self.push_child_crate_path_zero_alloc(parent, name);
        let ds = self.tree.strings.len() as u32;
        self.tree.strings.push_str(&self.scratch_dir);
        let de = self.tree.strings.len() as u32;
        self.tree.modules.push(ModuleData {
            id,
            parent,
            file,
            decl_file,
            inline_node,
            decl_start,
            decl_end,
            name_start: ns,
            name_end: ne,
            crate_path_start: cs,
            crate_path_end: ce,
            dir_start: ds,
            dir_end: de,
            children_start: 0,
            children_len: 0,
            vis,
            flags,
        });
        if file.is_some() {
            self.tree.file_to_modules.push((file, id));
        }
        self.raw_edges.push((parent, ModChild { child: id, name_start: ns, name_end: ne, vis }));
        id
    }

    #[inline]
    fn push_child_crate_path_zero_alloc(&mut self, parent: ModuleId, child_name: &str) -> (u32, u32) {
        let pm = self.tree.modules[parent.0 as usize];
        let p_start = pm.crate_path_start as usize;
        let p_end = pm.crate_path_end as usize;
        self.tree.strings.reserve((p_end - p_start) + 2 + child_name.len());
        let start = self.tree.strings.len() as u32;
        // SAFETY: `p_start..p_end` was pushed as valid UTF-8 into `self.tree.strings`.
        let buf = unsafe { self.tree.strings.as_mut_vec() };
        buf.extend_from_within(p_start..p_end);
        self.tree.strings.push_str("::");
        self.tree.strings.push_str(child_name);
        (start, self.tree.strings.len() as u32)
    }

    fn finalize(mut self) -> ModuleTree {
        let mod_count = self.tree.modules.len();
        if !self.raw_edges.is_empty() {
            let mut offsets = vec![0u32; mod_count];
            for &(p, _) in &self.raw_edges {
                offsets[p.0 as usize] += 1;
            }
            let mut running = 0u32;
            for (m, slot) in offsets.iter_mut().enumerate() {
                let count = *slot;
                self.tree.modules[m].children_start = running;
                self.tree.modules[m].children_len = count;
                *slot = running;
                running += count;
            }
            let dummy = ModChild { child: ModuleId::NONE, name_start: 0, name_end: 0, vis: VisKind::Private };
            self.tree.children.resize(running as usize, dummy);
            for (p, edge) in self.raw_edges {
                let slot = &mut offsets[p.0 as usize];
                self.tree.children[*slot as usize] = edge;
                *slot += 1;
            }
        }
        if self.tree.file_to_modules.len() > 1 {
            let mods = &self.tree.modules;
            self.tree.file_to_modules.sort_by_key(|&(f, m)| {
                let md = &mods[m.0 as usize];
                (f.0, md.is_inline() as u8, md.decl_start)
            });
        }
        self.tree
    }
}
