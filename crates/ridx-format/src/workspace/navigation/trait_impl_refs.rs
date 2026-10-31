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

fn strip_generics(s: &str) -> &str {
    match s.find('<') {
        Some(idx) => s[..idx].trim_end(),
        None => s.trim(),
    }
}

fn path_matches_trait(candidate: &str, raw_trait: &str, local_trait: &str, global_trait: &str, std_cands: &[String]) -> bool {
    let base = strip_generics(candidate);
    base == raw_trait || base == local_trait || base == global_trait || std_cands.iter().any(|c| c == base)
}

impl WorkspaceIndex {
    /// Expands `out_refs` with container-scoped `trait ↔ impl` links for associated items.
    pub(super) fn expand_trait_impl_references<'a>(&'a self, def: &ResolvedDefinition<'a>, out_refs: &mut Vec<WorkspaceReference<'a>>) {
        if !matches!(def.kind, DefKind::AssocFn | DefKind::AssocConst | DefKind::AssocTy) {
            return;
        }
        let Some((raw_parent, item_name)) = def.path.rsplit_once("::") else {
            return;
        };

        let mut seen_locs: HashSet<(String, u32, u32)> = out_refs.iter().map(|r| (r.file.to_string(), r.span.start_line(), r.span.start_col())).collect();
        seen_locs.insert((def.file.to_string(), def.start_line, def.start_col));

        if let Some((trait_reader_idx, local_trait, global_trait)) = self.resolve_workspace_trait_container(raw_parent, def.reader_idx, false) {
            self.expand_from_trait_to_impls(def, item_name, raw_parent, &local_trait, &global_trait, trait_reader_idx, &mut seen_locs, out_refs);
            return;
        }

        if def.kind == DefKind::AssocTy {
            if let Some(tr) = super::qualified_path::extract_impl_trait(def.path) {
                let base_tr = strip_generics(tr);
                if let Some((trait_reader_idx, local_trait, global_trait)) = self.resolve_workspace_trait_container(base_tr, def.reader_idx, true) {
                    if let Some(t_idx) = trait_reader_idx {
                        if !self.is_reader_external(t_idx) {
                            let r = &self.readers[t_idx];
                            let sr = self.source_roots.get(t_idx).and_then(|s| s.as_deref()).or(self.workspace_root.as_deref());
                            let uk = self.unit_keys.get(t_idx).and_then(|u| u.as_deref());
                            for t_def in r.find_definitions_by_owner(&local_trait).into_iter().chain(r.find_definitions_by_owner(&global_trait)) {
                                if DefKind::from(t_def.kind) == DefKind::AssocTy && r.get_str(t_def.name_offset()) == item_name {
                                    let sp = t_def.span();
                                    let f = r.get_file(sp.file_id());
                                    if seen_locs.insert((f.to_string(), sp.start_line(), sp.start_col())) {
                                        out_refs.push(WorkspaceReference {
                                            crate_name: r.crate_name(),
                                            file: f,
                                            span: sp,
                                            kind: DefKind::AssocTy,
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
                    self.expand_from_trait_to_impls(def, item_name, base_tr, &local_trait, &global_trait, trait_reader_idx, &mut seen_locs, out_refs);
                }
            }
        }
    }

    fn resolve_workspace_trait_container(&self, raw_parent: &str, preferred_reader: Option<usize>, known_trait_syntax: bool) -> Option<(Option<usize>, String, String)> {
        if raw_parent.starts_with('<') || raw_parent.ends_with('>') {
            return None;
        }
        let base_parent = strip_generics(raw_parent);
        let check_reader = |i: usize| -> Option<(bool, Option<usize>, String, String)> {
            let r = &self.readers[i];
            let cname = r.crate_name();
            let local = base_parent.strip_prefix(&format!("{cname}::")).unwrap_or(base_parent);
            let global = format!("{cname}::{local}");
            let d = r.find_definition_by_path(base_parent).or_else(|| r.find_definition_by_path(local)).or_else(|| r.find_definition_by_path(&global))?;
            Some((DefKind::from(d.kind) == DefKind::Trait, Some(i), local.to_string(), global))
        };

        if let Some(pref) = preferred_reader {
            if let Some((is_trait, idx, local, global)) = check_reader(pref) {
                return is_trait.then_some((idx, local, global));
            }
        }
        let cname = super::qualified_path::extract_root_crate(base_parent);
        let fast_indices = self.readers_for_crate_fast(cname);
        if !fast_indices.is_empty() {
            for &i in fast_indices.iter() {
                if Some(i) != preferred_reader && !self.is_reader_external(i) {
                    if let Some((is_trait, idx, local, global)) = check_reader(i) {
                        return is_trait.then_some((idx, local, global));
                    }
                }
            }
        } else {
            for i in 0..self.readers.len() {
                if Some(i) != preferred_reader && !self.is_reader_external(i) {
                    if let Some((true, idx, local, global)) = check_reader(i) {
                        return Some((idx, local, global));
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
        _trait_reader_idx: Option<usize>,
        seen_locs: &mut HashSet<(String, u32, u32)>,
        out_refs: &mut Vec<WorkspaceReference<'a>>,
    ) {
        let std_cands = super::std_map::std_path_candidates(global_trait);
        let skip_impl_queries = def.kind == DefKind::AssocTy
            || matches!(item_name, "poll" | "clone" | "into_iter" | "from" | "into" | "default" | "fmt" | "eq" | "partial_cmp" | "cmp" | "hash" | "deref");
        let max_impl_queries = if skip_impl_queries { 0 } else { 24 };
        let mut expanded_impl_refs = 0usize;
        let mut seen_impl_paths: HashSet<(usize, &'a str)> = HashSet::new();
        for (i, reader) in self.readers.iter().enumerate() {
            if self.is_reader_external(i) {
                continue;
            }
            let source_root = self.source_roots.get(i).and_then(|r| r.as_deref()).or(self.workspace_root.as_deref());
            let unit_key = self.unit_keys.get(i).and_then(|u| u.as_deref());

            for de in reader.deref_entries() {
                let tgt = reader.get_str(de.target_type_offset);
                if !path_matches_trait(tgt, raw_parent, local_trait, global_trait, &std_cands) {
                    continue;
                }
                let impl_owner = reader.get_str(de.source_type_offset);
                for impl_def in reader.find_definitions_by_owner(impl_owner) {
                    if DefKind::from(impl_def.kind) != def.kind || impl_def.is_macro_expansion() || reader.get_str(impl_def.name_offset()) != item_name {
                        continue;
                    }
                    let impl_path = reader.get_str(impl_def.path_offset());
                    if let Some(impl_trait) = super::qualified_path::extract_impl_trait(impl_path) {
                        if !path_matches_trait(impl_trait, raw_parent, local_trait, global_trait, &std_cands) {
                            continue;
                        }
                    }
                    let span = impl_def.span();
                    let file = reader.get_file(span.file_id());
                    if seen_locs.insert((file.to_string(), span.start_line(), span.start_col())) {
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
                    if expanded_impl_refs < max_impl_queries && seen_impl_paths.insert((i, impl_path)) {
                        expanded_impl_refs += 1;
                        let impl_refs = self.find_references_for_unit_with_target(impl_path, unit_key, Some((file, span.start_line(), def.kind, item_name)));
                        for r in impl_refs {
                            if seen_locs.insert((r.file.to_string(), r.span.start_line(), r.span.start_col())) {
                                out_refs.push(r);
                            }
                        }
                    }
                }
            }

            let defs = reader.definitions();
            for &def_idx in self.unit_match.assoc_ty_defs(i, reader) {
                let Some(impl_def) = defs.get(def_idx as usize) else {
                    continue;
                };
                if DefKind::from(impl_def.kind) != def.kind || reader.get_str(impl_def.name_offset()) != item_name {
                    continue;
                }
                let impl_path = reader.get_str(impl_def.path_offset());
                let Some(impl_trait) = super::qualified_path::extract_impl_trait(impl_path) else {
                    continue;
                };
                if !path_matches_trait(impl_trait, raw_parent, local_trait, global_trait, &std_cands) {
                    continue;
                };
                let span = impl_def.span();
                let file = reader.get_file(span.file_id());
                if seen_locs.insert((file.to_string(), span.start_line(), span.start_col())) {
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
    }
}
