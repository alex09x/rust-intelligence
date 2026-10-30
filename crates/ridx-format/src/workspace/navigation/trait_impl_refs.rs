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

fn path_matches_trait(
    candidate: &str,
    raw_trait: &str,
    local_trait: &str,
    global_trait: &str,
) -> bool {
    let base = strip_generics(candidate);
    base == raw_trait || base == local_trait || base == global_trait
}

impl WorkspaceIndex {
    /// Expands `out_refs` with container-scoped `trait ↔ impl` links for associated items.
    pub(super) fn expand_trait_impl_references<'a>(
        &'a self,
        def: &ResolvedDefinition<'a>,
        out_refs: &mut Vec<WorkspaceReference<'a>>,
    ) {
        if !matches!(
            def.kind,
            DefKind::AssocFn | DefKind::AssocConst | DefKind::AssocTy
        ) {
            return;
        }
        let Some((raw_parent, item_name)) = def.path.rsplit_once("::") else {
            return;
        };

        let mut seen_locs: HashSet<(String, u32, u32)> = out_refs
            .iter()
            .map(|r| (r.file.to_string(), r.span.start_line(), r.span.start_col()))
            .collect();
        seen_locs.insert((def.file.to_string(), def.start_line, def.start_col));

        if let Some((trait_reader_idx, local_trait, global_trait)) =
            self.resolve_workspace_trait_container(raw_parent, def.reader_idx)
        {
            self.expand_from_trait_to_impls(
                def,
                item_name,
                raw_parent,
                &local_trait,
                &global_trait,
                trait_reader_idx,
                &mut seen_locs,
                out_refs,
            );
        }
    }

    fn resolve_workspace_trait_container(
        &self,
        raw_parent: &str,
        preferred_reader: Option<usize>,
    ) -> Option<(usize, String, String)> {
        if raw_parent.starts_with('<') || raw_parent.ends_with('>') {
            return None;
        }
        let base_parent = strip_generics(raw_parent);
        let check_reader = |i: usize| -> Option<(usize, String, String)> {
            if self.is_reader_external(i) {
                return None;
            }
            let r = &self.readers[i];
            let cname = r.crate_name();
            let local = base_parent
                .strip_prefix(&format!("{cname}::"))
                .unwrap_or(base_parent);
            let global = format!("{cname}::{local}");
            let d = r
                .find_definition_by_path(base_parent)
                .or_else(|| r.find_definition_by_path(local))
                .or_else(|| r.find_definition_by_path(&global))?;
            if DefKind::from(d.kind) == DefKind::Trait {
                Some((i, local.to_string(), global))
            } else {
                None
            }
        };

        if let Some(pref) = preferred_reader {
            if let Some(found) = check_reader(pref) {
                return Some(found);
            }
        }
        for i in 0..self.readers.len() {
            if Some(i) != preferred_reader {
                if let Some(found) = check_reader(i) {
                    return Some(found);
                }
            }
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
        _trait_reader_idx: usize,
        seen_locs: &mut HashSet<(String, u32, u32)>,
        out_refs: &mut Vec<WorkspaceReference<'a>>,
    ) {
        let mut expanded_impl_refs = 0usize;
        for (i, reader) in self.readers.iter().enumerate() {
            if self.is_reader_external(i) {
                continue;
            }
            let source_root = self
                .source_roots
                .get(i)
                .and_then(|r| r.as_deref())
                .or(self.workspace_root.as_deref());
            let unit_key = self.unit_keys.get(i).and_then(|u| u.as_deref());

            for de in reader.deref_entries() {
                let tgt = reader.get_str(de.target_type_offset);
                if !path_matches_trait(tgt, raw_parent, local_trait, global_trait) {
                    continue;
                }
                let impl_owner = reader.get_str(de.source_type_offset);
                for impl_def in reader.find_definitions_by_owner(impl_owner) {
                    if DefKind::from(impl_def.kind) != def.kind
                        || reader.get_str(impl_def.name_offset()) != item_name
                        || impl_def.is_macro_expansion()
                    {
                        continue;
                    }
                    let impl_path = reader.get_str(impl_def.path_offset());
                    if let Some((_, after_as)) = impl_path.split_once(" as ") {
                        if let Some((impl_trait, _)) = after_as.split_once('>') {
                            if !path_matches_trait(
                                impl_trait,
                                raw_parent,
                                local_trait,
                                global_trait,
                            ) {
                                continue;
                            }
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
                    if expanded_impl_refs < 24 {
                        expanded_impl_refs += 1;
                        let impl_refs = self.find_references_for_unit_with_target(
                            impl_path,
                            unit_key,
                            Some((file, span.start_line(), def.kind, item_name)),
                        );
                        for r in impl_refs {
                            if seen_locs.insert((
                                r.file.to_string(),
                                r.span.start_line(),
                                r.span.start_col(),
                            )) {
                                out_refs.push(r);
                            }
                        }
                    }
                }
            }
        }
    }
}
