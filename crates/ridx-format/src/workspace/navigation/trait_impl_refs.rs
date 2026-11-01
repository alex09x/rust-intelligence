/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Container-scoped `trait ↔ impl` reference resolution.
//!
//! Associated items are linked exclusively through their owning container (`deref_entries` and
//! `find_definitions_by_owner`), never by bare method/associated-item name:
//! - When the target is a trait item (`parent` has `DefKind::Trait`), references expand to
//!   implementing types recorded in `deref_entries` and their matching associated items.
//! - When the target is a concrete `impl` item (`parent` is not `DefKind::Trait`), only direct
//!   references to that `impl` item plus the corresponding trait item declaration are included.

use super::*;
use std::collections::HashSet;

pub(super) fn strip_generics(s: &str) -> &str {
    match s.find('<') {
        Some(idx) => s[..idx].trim_end(),
        None => s.trim(),
    }
}

pub(super) fn path_matches_trait(candidate: &str, raw_trait: &str, local_trait: &str, global_trait: &str, std_cands: &[String]) -> bool {
    let base = strip_generics(candidate);
    if base == raw_trait || base == local_trait || base == global_trait || std_cands.iter().any(|c| c == base) {
        return true;
    }
    let raw_base = strip_generics(raw_trait);
    if base == raw_base {
        return true;
    }
    if local_trait.contains("::") && base.ends_with(local_trait) && base.as_bytes().get(base.len().wrapping_sub(local_trait.len() + 1)) == Some(&b':') {
        return true;
    }
    if base.contains("::") && local_trait.ends_with(base) && local_trait.as_bytes().get(local_trait.len().wrapping_sub(base.len() + 1)) == Some(&b':') {
        return true;
    }
    if let (Some((cand_crate, _)), Some((glob_crate, _))) = (base.split_once("::"), global_trait.split_once("::")) {
        if cand_crate == glob_crate && !matches!(cand_crate, "core" | "alloc" | "std") {
            let cand_short = base.rsplit("::").next().unwrap_or(base);
            let local_short = local_trait.rsplit("::").next().unwrap_or(local_trait);
            if cand_short == local_short && raw_base.ends_with(&format!("::{local_short}")) {
                return true;
            }
        }
    }
    false
}

pub(super) fn reader_path_matches_trait(
    reader: &IndexReader<memmap2::Mmap>,
    candidate: &str,
    raw_trait: &str,
    local_trait: &str,
    global_trait: &str,
    std_cands: &[String],
) -> bool {
    let base = strip_generics(candidate);
    let r_crate = reader.crate_name();
    let glob_crate = super::qualified_path::extract_root_crate(global_trait);
    if r_crate != glob_crate
        && !matches!(glob_crate, "core" | "alloc" | "std")
        && !base.starts_with(&format!("{glob_crate}::"))
        && reader.definitions().iter().any(|d| DefKind::from(d.kind) == DefKind::Trait && reader.get_str(d.path_offset()) == base)
    {
        return false;
    }
    path_matches_trait(base, raw_trait, local_trait, global_trait, std_cands)
}

impl WorkspaceIndex {
    /// Expands `out_refs` with container-scoped `trait ↔ impl` links for associated items.
    pub(super) fn expand_trait_impl_references<'a>(&'a self, def: &ResolvedDefinition<'a>, out_refs: &mut Vec<WorkspaceReference<'a>>) {
        self.expand_trait_impl_inner(def, out_refs, false);
    }

    /// Expands `out_refs` with full `trait ↔ all impls` links and associated type projections for rename.
    pub(super) fn expand_trait_impl_for_rename<'a>(&'a self, def: &ResolvedDefinition<'a>, out_refs: &mut Vec<WorkspaceReference<'a>>) {
        self.expand_trait_impl_inner(def, out_refs, true);
    }

    fn expand_trait_impl_inner<'a>(&'a self, def: &ResolvedDefinition<'a>, out_refs: &mut Vec<WorkspaceReference<'a>>, for_rename: bool) {
        if !matches!(def.kind, DefKind::AssocFn | DefKind::AssocConst | DefKind::AssocTy) {
            return;
        }
        let Some((raw_parent, item_name)) = def.path.rsplit_once("::") else {
            return;
        };

        let mut seen_locs: HashSet<(String, u32, u32)> = out_refs.iter().map(|r| (r.file.to_string(), r.span.start_line(), r.span.start_col())).collect();
        seen_locs.insert((def.file.to_string(), def.start_line, def.start_col));

        if let Some((trait_reader_idx, local_trait, global_trait)) = self.resolve_workspace_trait_container(raw_parent, def.reader_idx, false) {
            self.expand_from_trait_to_impls(def, item_name, raw_parent, &local_trait, &global_trait, trait_reader_idx, &mut seen_locs, out_refs, for_rename);
            return;
        }

        if for_rename || def.kind == DefKind::AssocTy {
            if let Some(tr) = super::qualified_path::extract_impl_trait(def.path) {
                let base_tr = strip_generics(tr);
                if let Some((trait_reader_idx, local_trait, global_trait)) = self.resolve_workspace_trait_container(base_tr, def.reader_idx, true) {
                    if let Some(t_idx) = trait_reader_idx {
                        if !self.is_reader_external(t_idx) {
                            let r = &self.readers[t_idx];
                            let sr = self.source_roots.get(t_idx).and_then(|s| s.as_deref()).or(self.workspace_root.as_deref());
                            let uk = self.unit_keys.get(t_idx).and_then(|u| u.as_deref());
                            for t_def in r.find_definitions_by_owner(&local_trait).into_iter().chain(r.find_definitions_by_owner(&global_trait)) {
                                if DefKind::from(t_def.kind) == def.kind && r.get_str(t_def.name_offset()) == item_name {
                                    let sp = t_def.span();
                                    let f = r.get_file(sp.file_id());
                                    if seen_locs.insert((f.to_string(), sp.start_line(), sp.start_col())) {
                                        out_refs.push(WorkspaceReference {
                                            crate_name: r.crate_name(),
                                            file: f,
                                            span: sp,
                                            kind: def.kind,
                                            is_write: false,
                                            is_macro_expansion: false,
                                            source_root: sr,
                                            unit_key: uk,
                                        });
                                    }
                                }
                            }
                        }
                    }
                    let trait_assoc_path = format!("{global_trait}::{item_name}");
                    let t_uk = trait_reader_idx.and_then(|idx| self.unit_keys.get(idx).and_then(|u| u.as_deref()));
                    for r in self.find_references_for_unit_with_target(&trait_assoc_path, t_uk, None) {
                        if seen_locs.insert((r.file.to_string(), r.span.start_line(), r.span.start_col())) {
                            out_refs.push(r);
                        }
                    }
                    self.expand_from_trait_to_impls(def, item_name, base_tr, &local_trait, &global_trait, trait_reader_idx, &mut seen_locs, out_refs, for_rename);
                }
            }
        }
    }

    fn resolve_workspace_trait_container(&self, raw_parent: &str, preferred_reader: Option<usize>, known_trait_syntax: bool) -> Option<(Option<usize>, String, String)> {
        if raw_parent.starts_with('<') || raw_parent.ends_with('>') {
            return None;
        }
        let base_parent = strip_generics(raw_parent);
        let check_reader = |i: usize| -> Option<(Option<usize>, String, String)> {
            let r = &self.readers[i];
            let cname = r.crate_name();
            let local = base_parent.strip_prefix(&format!("{cname}::")).unwrap_or(base_parent);
            let global = format!("{cname}::{local}");
            let has_exact_trait = [base_parent, local, global.as_str()].iter().any(|&p| {
                r.find_definition_by_path(p).is_some_and(|d| DefKind::from(d.kind) == DefKind::Trait)
                    || r.definitions().iter().any(|d| DefKind::from(d.kind) == DefKind::Trait && r.get_str(d.path_offset()) == p)
            });
            if has_exact_trait {
                return Some((Some(i), local.to_string(), global));
            }
            if known_trait_syntax && local.contains("::") {
                let suffix = format!("::{local}");
                let trait_short = local.rsplit("::").next().unwrap_or(local);
                let mut single_match: Option<&str> = None;
                let mut match_count = 0usize;
                for d in r.definitions() {
                    if DefKind::from(d.kind) == DefKind::Trait {
                        let dp = r.get_str(d.path_offset());
                        if dp.ends_with(&suffix) {
                            let d_local = dp.strip_prefix(&format!("{cname}::")).unwrap_or(dp);
                            return Some((Some(i), d_local.to_string(), format!("{cname}::{d_local}")));
                        }
                        if r.get_str(d.name_offset()) == trait_short {
                            single_match = Some(dp);
                            match_count += 1;
                        }
                    }
                }
                if match_count == 1 {
                    if let Some(dp) = single_match {
                        let d_local = dp.strip_prefix(&format!("{cname}::")).unwrap_or(dp);
                        return Some((Some(i), d_local.to_string(), format!("{cname}::{d_local}")));
                    }
                }
            }
            None
        };

        if let Some(pref) = preferred_reader {
            if let Some(res) = check_reader(pref) {
                return Some(res);
            }
        }
        let cname = super::qualified_path::extract_root_crate(base_parent);
        let fast_indices = self.readers_for_crate_fast(cname);
        if !fast_indices.is_empty() {
            for &i in fast_indices.iter() {
                if Some(i) != preferred_reader && !self.is_reader_external(i) {
                    if let Some(res) = check_reader(i) {
                        return Some(res);
                    }
                }
            }
        } else {
            for i in 0..self.readers.len() {
                if Some(i) != preferred_reader && !self.is_reader_external(i) {
                    if let Some(res) = check_reader(i) {
                        return Some(res);
                    }
                }
            }
        }
        if known_trait_syntax && base_parent.contains("::") {
            let local = base_parent.strip_prefix(&format!("{cname}::")).unwrap_or(base_parent);
            let global = format!("{cname}::{local}");
            return Some((None, local.to_string(), global));
        }
        None
    }

    #[allow(clippy::too_many_arguments)]
    fn expand_from_trait_to_impls<'a>(
        &'a self,
        def: &ResolvedDefinition<'a>,
        item_name: &'a str,
        raw_parent: &str,
        local_trait: &str,
        global_trait: &str,
        trait_reader_idx: Option<usize>,
        seen_locs: &mut HashSet<(String, u32, u32)>,
        out_refs: &mut Vec<WorkspaceReference<'a>>,
        for_rename: bool,
    ) {
        let std_cands = super::std_map::std_path_candidates(global_trait);
        let max_impl_queries = if def.kind == DefKind::AssocTy
            || matches!(item_name, "poll" | "clone" | "into_iter" | "from" | "into" | "default" | "fmt" | "eq" | "partial_cmp" | "cmp" | "hash" | "deref")
        {
            0
        } else {
            24
        };
        let mut expanded_impl_refs = 0usize;
        let mut seen_impl_paths: HashSet<(usize, &'a str)> = HashSet::new();
        let mut assoc_files: Vec<(usize, u32)> = Vec::new();
        if for_rename && def.kind == DefKind::AssocTy {
            if let Some(t_idx) = trait_reader_idx.filter(|&idx| !self.is_reader_external(idx)) {
                let r = &self.readers[t_idx];
                if let Some(td) = r.find_definition_by_path(local_trait).or_else(|| r.find_definition_by_path(global_trait)) {
                    assoc_files.push((t_idx, td.span().file_id()));
                }
            }
        }

        for (i, reader) in self.readers.iter().enumerate() {
            if self.is_reader_external(i) {
                continue;
            }
            let source_root = self.source_roots.get(i).and_then(|r| r.as_deref()).or(self.workspace_root.as_deref());
            let unit_key = self.unit_keys.get(i).and_then(|u| u.as_deref());
            let defs = reader.definitions();
            let has_qualified_impl_defs = self
                .unit_match
                .assoc_ty_defs(i, reader)
                .iter()
                .any(|&idx| defs.get(idx as usize).is_some_and(|d| super::qualified_path::extract_impl_trait(reader.get_str(d.path_offset())).is_some()));

            for de in reader.deref_entries() {
                let tgt = reader.get_str(de.target_type_offset);
                if !reader_path_matches_trait(reader, tgt, raw_parent, local_trait, global_trait, &std_cands) {
                    continue;
                }
                let impl_owner = reader.get_str(de.source_type_offset);
                for impl_def in reader.find_definitions_by_owner(impl_owner) {
                    if DefKind::from(impl_def.kind) != def.kind || (!for_rename && impl_def.is_macro_expansion()) || reader.get_str(impl_def.name_offset()) != item_name {
                        continue;
                    }
                    let impl_path = reader.get_str(impl_def.path_offset());
                    if let Some(impl_trait) = super::qualified_path::extract_impl_trait(impl_path) {
                        if !reader_path_matches_trait(reader, impl_trait, raw_parent, local_trait, global_trait, &std_cands) {
                            continue;
                        }
                    } else if has_qualified_impl_defs {
                        continue;
                    }
                    let span = impl_def.span();
                    let file = reader.get_file(span.file_id());
                    if for_rename && def.kind == DefKind::AssocTy {
                        assoc_files.push((i, span.file_id()));
                    }
                    if !impl_def.is_macro_expansion() && seen_locs.insert((file.to_string(), span.start_line(), span.start_col())) {
                        out_refs.push(WorkspaceReference {
                            crate_name: reader.crate_name(),
                            file,
                            span,
                            kind: DefKind::from(impl_def.kind),
                            is_write: false,
                            is_macro_expansion: false,
                            source_root,
                            unit_key,
                        });
                    }
                    if (!for_rename || !impl_path.starts_with('<')) && expanded_impl_refs < max_impl_queries && seen_impl_paths.insert((i, impl_path)) {
                        expanded_impl_refs += 1;
                        let explicit = (!impl_def.is_macro_expansion()).then_some((file, span.start_line(), def.kind, item_name));
                        for r in self.find_references_for_unit_with_target(impl_path, unit_key, explicit) {
                            if seen_locs.insert((r.file.to_string(), r.span.start_line(), r.span.start_col())) {
                                out_refs.push(r);
                            }
                        }
                    }
                }
            }

            for &def_idx in self.unit_match.assoc_ty_defs(i, reader) {
                let Some(impl_def) = defs.get(def_idx as usize) else {
                    continue;
                };
                if DefKind::from(impl_def.kind) != def.kind || (!for_rename && impl_def.is_macro_expansion()) || reader.get_str(impl_def.name_offset()) != item_name {
                    continue;
                }
                let impl_path = reader.get_str(impl_def.path_offset());
                let Some(impl_trait) = super::qualified_path::extract_impl_trait(impl_path) else {
                    continue;
                };
                if !reader_path_matches_trait(reader, impl_trait, raw_parent, local_trait, global_trait, &std_cands) {
                    continue;
                }
                let span = impl_def.span();
                let file = reader.get_file(span.file_id());
                if for_rename && def.kind == DefKind::AssocTy {
                    assoc_files.push((i, span.file_id()));
                }
                if !impl_def.is_macro_expansion() && seen_locs.insert((file.to_string(), span.start_line(), span.start_col())) {
                    out_refs.push(WorkspaceReference {
                        crate_name: reader.crate_name(),
                        file,
                        span,
                        kind: DefKind::from(impl_def.kind),
                        is_write: false,
                        is_macro_expansion: false,
                        source_root,
                        unit_key,
                    });
                }
            }
        }

        if for_rename {
            self.expand_trait_qualified_refs(def, item_name, raw_parent, local_trait, global_trait, &std_cands, seen_locs, out_refs);
            if def.kind == DefKind::AssocTy {
                self.expand_assoc_ty_projections(def, item_name, raw_parent, local_trait, global_trait, &std_cands, &mut assoc_files, seen_locs, out_refs);
            }
        }
    }
}
