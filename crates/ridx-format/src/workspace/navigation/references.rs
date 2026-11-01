/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use super::reference_record::{matches_reference_target, points_at_definition, push_reference, ReferenceTarget};
use super::*;
impl WorkspaceIndex {
    /// Finds all references to a symbol, optionally scoped to a target dependency unit.
    pub fn find_references_for_unit<'a>(&'a self, target_path: &str, target_unit_key: Option<&str>) -> Vec<WorkspaceReference<'a>> {
        self.find_references_for_unit_with_target(target_path, target_unit_key, None)
    }

    /// Finds all references to a symbol with optional explicit target definition metadata.
    pub(crate) fn find_references_for_unit_with_target<'a>(
        &'a self,
        target_path: &str,
        target_unit_key: Option<&str>,
        explicit_def: Option<(&str, u32, DefKind, &'a str)>,
    ) -> Vec<WorkspaceReference<'a>> {
        let target_unit_key = target_unit_key.filter(|k| !k.is_empty());
        let mut all_refs = Vec::new();
        let mut seen_refs = std::collections::HashSet::new();
        let raw_target_idx = if let Some(target_unit) = target_unit_key { self.unit_keys.iter().position(|u| u.as_deref() == Some(target_unit)) } else { None };
        let mut target_crate = super::qualified_path::extract_root_crate(target_path);
        if !self.has_crate(target_crate) && !matches!(target_crate, "core" | "alloc" | "std" | "proc_macro") {
            let parent_path = target_path.rsplit_once("::").map(|(p, _)| p);
            let mut resolved = false;
            if let Some(t_idx) = raw_target_idx {
                let r = &self.readers[t_idx];
                if !target_path.contains("::") || r.find_definition_by_path(target_path).is_some() || parent_path.is_some_and(|p| r.find_definition_by_path(p).is_some()) {
                    target_crate = r.crate_name();
                    resolved = true;
                }
            }
            if !resolved {
                for r in &self.readers {
                    if r.find_definition_by_path(target_path).is_some() {
                        target_crate = r.crate_name();
                        resolved = true;
                        break;
                    }
                }
            }
            if !resolved {
                for r in &self.readers {
                    if parent_path.is_some_and(|p| r.find_definition_by_path(p).is_some_and(|d| DefKind::from(d.kind) == DefKind::Enum)) {
                        target_crate = r.crate_name();
                        break;
                    }
                }
            }
        }
        let target_idx = if target_unit_key.is_some() {
            raw_target_idx
        } else {
            let fast = self.readers_for_crate_fast(target_crate);
            fast.iter()
                .copied()
                .find(|&idx| {
                    let r = &self.readers[idx];
                    let sub = target_path.strip_prefix(&format!("{target_crate}::")).unwrap_or(target_path);
                    r.find_definition_by_path(target_path).is_some() || r.find_definition_by_path(sub).is_some()
                })
                .or_else(|| fast.first().copied())
        };

        if let Some(t_idx) = target_idx {
            if (!target_path.starts_with('<')
                && !matches!(target_crate, "core" | "alloc" | "std" | "proc_macro")
                && (explicit_def.is_none_or(|(ef, _, _, _)| ef != "external") || !self.has_crate(target_crate)))
                || self.readers[t_idx].crate_name() == target_crate
            {
                target_crate = self.readers[t_idx].crate_name();
            }
        }
        let effective_unit_key = target_unit_key;

        let resolve_full = |t_idx: usize, f: &str| -> String {
            self.source_roots
                .get(t_idx)
                .and_then(|r| r.as_ref())
                .or(self.workspace_root.as_ref())
                .map(|root| crate::format::join_resolving_overlap(root, Path::new(f)).to_string_lossy().to_string())
                .unwrap_or_else(|| f.to_string())
        };

        let target_def_coords = if let Some((ef, el, _, _)) = explicit_def.filter(|d| !d.0.is_empty() && d.1 > 0) {
            let full = target_idx.map(|t| resolve_full(t, ef)).unwrap_or_else(|| ef.to_string());
            Some((ef.to_string(), full, el))
        } else if let Some(t_idx) = target_idx {
            let r = &self.readers[t_idx];
            let def_opt = r.find_definition_by_path(target_path).or_else(|| {
                let rest = target_path.strip_prefix(&format!("{target_crate}::"))?;
                r.find_definition_by_path(&format!("{}::{rest}", r.crate_name()))
            });
            def_opt.map(|def| {
                let span = def.span();
                let file = r.get_file(span.file_id());
                (file.to_string(), resolve_full(t_idx, file), span.start_line())
            })
        } else {
            None
        };

        let prefix = super::qualified_path::extract_root_crate(target_path);
        let is_alias_for_target =
            if let Some(target_unit) = effective_unit_key { self.reader_extern_units.iter().any(|m| m.get(prefix).is_some_and(|uk| uk == target_unit)) } else { false };

        let local_path = if target_path.starts_with(&format!("{target_crate}::")) {
            &target_path[target_crate.len() + 2..]
        } else if !target_path.starts_with('<') && is_alias_for_target && target_path.contains("::") {
            &target_path[prefix.len() + 2..]
        } else {
            target_path
        };
        let is_global_angle = local_path.starts_with('<') && {
            let c = super::qualified_path::extract_root_crate(local_path);
            c == target_crate || matches!(c, "core" | "alloc" | "std" | "proc_macro") || self.has_crate(c)
        };
        let global_path_buf = if is_global_angle { local_path.to_string() } else { format!("{target_crate}::{local_path}") };
        let global_path = global_path_buf.as_str();
        let mut spellings = super::qualified_path::consumer_spellings(local_path, target_crate);
        for imp_sp in super::qualified_path::impl_consumer_spellings(local_path, target_crate) {
            if !spellings.contains(&imp_sp) {
                spellings.push(imp_sp);
            }
        }
        for cand in super::std_map::std_path_candidates(global_path) {
            if cand != global_path && cand != local_path && cand != target_path && !spellings.contains(&cand) {
                spellings.push(cand);
            }
        }
        if explicit_def.map(|d| d.2) == Some(DefKind::Macro) {
            if let Some((_, macro_name)) = local_path.rsplit_once("::") {
                let root_macro = format!("{target_crate}::{macro_name}");
                if !spellings.contains(&root_macro) {
                    spellings.push(root_macro);
                }
                if !spellings.contains(&macro_name.to_string()) {
                    spellings.push(macro_name.to_string());
                }
            }
        }
        if let Some((ty, tr, tail)) = super::qualified_path::split_qualified_parts(target_path) {
            let tr_crate = super::qualified_path::extract_root_crate(tr);
            if ty.len() == 1 && ty.as_bytes()[0].is_ascii_uppercase() && matches!(tr_crate, "core" | "alloc" | "std") {
                let trait_item = format!("{tr}{tail}");
                for cand in super::std_map::std_path_candidates(&trait_item) {
                    if !spellings.contains(&cand) {
                        spellings.push(cand);
                    }
                }
            }
        }

        let disambiguate_line = explicit_def.is_some()
            && target_idx.is_some_and(|t| {
                let r = &self.readers[t];
                r.has_multiple_definitions_at_path(target_path) || r.has_multiple_definitions_at_path(local_path) || r.has_multiple_definitions_at_path(global_path)
            });
        let ref_target = ReferenceTarget {
            kind: explicit_def.map(|d| d.2),
            name: explicit_def.map(|d| d.3),
            coords: target_def_coords.clone(),
            disambiguate_line,
            is_qualified_path: target_path.contains("::"),
        };

        // The target and the other builds of its crate and version (feature sets compile one
        // source into several units): a consumer reaching any of them uses the item.
        let target_builds: Vec<usize> = target_idx
            .map(|t| {
                let (name, version) = (self.readers[t].crate_name(), self.readers[t].crate_version());
                (0..self.readers.len()).filter(|&s| s == t || (self.readers[s].crate_name() == name && self.readers[s].crate_version() == version)).collect()
            })
            .unwrap_or_default();

        let mut match_memo = std::collections::HashMap::<(usize, &str), bool>::new();

        for (i, reader) in self.readers.iter().enumerate() {
            if self.is_reader_external(i) {
                continue;
            }
            let reader_unit = self.unit_keys.get(i).and_then(|u| u.as_deref());
            let reader_crate = reader.crate_name();

            let is_exact_target_unit =
                if let (Some(target_unit), Some(ru)) = (effective_unit_key, reader_unit) { ru == target_unit } else { target_idx == Some(i) || reader_crate == target_crate };

            let is_sibling_unit = target_idx.map(|t| self.are_sibling_workspace_units(t, i)).unwrap_or(false);

            let is_same_package = is_exact_target_unit || is_sibling_unit;

            if !is_same_package {
                let is_consumer_of_target = effective_unit_key
                    .map(|tu| {
                        let in_deps = self.reader_dependencies.get(i).map(|d| d.iter().any(|dep| dep == tu)).unwrap_or(false);
                        let in_extern = self.reader_extern_units.get(i).map(|m| m.values().any(|uk| uk == tu)).unwrap_or(false);
                        in_deps || in_extern
                    })
                    .unwrap_or(false);

                if reader_crate == target_crate && !is_consumer_of_target {
                    // Different package that collides on crate_name but is not a sibling unit.
                    continue;
                }
                // The reader is an external/referring crate.
                // If effective_unit_key is given:
                if let Some(target_unit) = effective_unit_key {
                    let has_deps = self.reader_dependencies.get(i).map(|d| !d.is_empty()).unwrap_or(false);
                    let has_extern_units = self.reader_extern_units.get(i).map(|m| !m.is_empty()).unwrap_or(false);

                    if has_deps || has_extern_units {
                        let in_deps = self.reader_dependencies.get(i).map(|d| d.iter().any(|dep| dep == target_unit || dep == target_crate)).unwrap_or(false);
                        let in_extern_units =
                            self.reader_extern_units.get(i).map(|m| m.keys().any(|k| k == target_crate) || m.values().any(|uk| uk == target_unit)).unwrap_or(false);
                        // A consumer may reach the target through a unit it depends on (an item
                        // used through a re-export): the dependency graph is walked from it.
                        if !in_deps && !in_extern_units && !target_builds.iter().any(|&t| self.is_reader_in_dep_graph(&[i], t)) {
                            // Referring crate has dependency info but depends neither on
                            // target_unit or target_crate nor, through another unit, on the
                            // target. Exclude!
                            continue;
                        }
                    } else {
                        // Legacy reader without dependency info: exclude if multiple versions of target_crate exist in workspace
                        let target_crate_versions = self.readers.iter().filter(|r| r.crate_name() == target_crate).count();
                        if target_crate_versions > 1 && reader_crate == target_crate {
                            continue;
                        }
                    }
                }
            }

            // References with canonical target_path (querying both local and global path representations)
            let source_root = self.source_roots.get(i).and_then(|r| r.as_deref()).or(self.workspace_root.as_deref());

            let mut query_path_vec = Vec::new();
            if local_path == global_path {
                query_path_vec.push(target_path);
            } else if is_same_package {
                query_path_vec.push(local_path);
                query_path_vec.push(global_path);
                if target_path != local_path && target_path != global_path {
                    query_path_vec.push(target_path);
                }
            } else {
                query_path_vec.push(global_path);
                if target_path != global_path && target_path != local_path {
                    query_path_vec.push(target_path);
                }
            }
            query_path_vec.extend(spellings.iter().map(String::as_str));
            for &q_path in &query_path_vec {
                let refs = reader.find_references(q_path);
                for r in refs {
                    let r_target_unit = reader.target_unit(r);
                    if let Some(target_unit) = effective_unit_key {
                        if !r_target_unit.is_empty() {
                            if !self.matches_unit_memoized(&mut match_memo, i, r_target_unit, target_unit, target_crate) {
                                continue;
                            }
                        } else if reader_crate == target_crate {
                            if !is_same_package {
                                continue;
                            }
                            if !is_sibling_unit {
                                if let Some(ru) = reader_unit {
                                    if ru != target_unit {
                                        continue;
                                    }
                                }
                            }
                        } else if let Some(coords) = &target_def_coords {
                            if !points_at_definition(reader, r, coords) {
                                continue;
                            }
                        }
                    }

                    if !matches_reference_target(reader, r, &ref_target) {
                        continue;
                    }

                    let owner = (reader_crate, reader_unit, source_root);
                    push_reference(reader, r, owner, &mut seen_refs, &mut all_refs);
                }
            }

            if let Some(ref coords) = target_def_coords {
                if !target_path.starts_with('<') && !target_path.contains("<impl ") && local_path.contains("::") {
                    if let Some((mod_prefix, item_name)) = local_path.rsplit_once("::") {
                        let parent_mod = mod_prefix.rsplit_once("::").map(|(p, _)| p);
                        for cname in [target_crate, reader_crate] {
                            for cand in [
                                Some(format!("{cname}::{item_name}")),
                                (cname != target_crate).then(|| format!("{cname}::{local_path}")),
                                parent_mod.map(|pm| format!("{cname}::{pm}::{item_name}")),
                                (cname == target_crate).then(|| parent_mod.map(|pm| format!("{pm}::{item_name}"))).flatten(),
                            ]
                            .into_iter()
                            .flatten()
                            {
                                for r in reader.find_references(&cand) {
                                    if r.target_line() != 0 && points_at_definition(reader, r, coords) && matches_reference_target(reader, r, &ref_target) {
                                        push_reference(reader, r, (reader_crate, reader_unit, source_root), &mut seen_refs, &mut all_refs);
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Also check if reader refers to target_path via an extern alias (e.g. `a1::foo` instead of `a::foo`)
            if let Some(extern_map) = self.reader_extern_units.get(i) {
                for (alias, uk) in extern_map {
                    if alias != target_crate {
                        if let Some(target_unit) = target_unit_key {
                            if !self.matches_unit_memoized(&mut match_memo, i, uk.as_str(), target_unit, target_crate) {
                                continue;
                            }
                        }
                        let aliased_path = format!("{alias}::{local_path}");
                        let mut aliased = super::qualified_path::consumer_spellings(local_path, alias);
                        aliased.push(aliased_path);
                        for r in aliased.iter().flat_map(|p| reader.find_references(p)) {
                            let r_target_unit = reader.target_unit(r);
                            if let Some(target_unit) = target_unit_key {
                                if !r_target_unit.is_empty() {
                                    if !self.matches_unit_memoized(&mut match_memo, i, r_target_unit, target_unit, target_crate) {
                                        continue;
                                    }
                                } else if let Some(coords) = &target_def_coords {
                                    if !points_at_definition(reader, r, coords) {
                                        continue;
                                    }
                                }
                            }
                            if !matches_reference_target(reader, r, &ref_target) {
                                continue;
                            }

                            let owner = (reader_crate, reader_unit, source_root);
                            push_reference(reader, r, owner, &mut seen_refs, &mut all_refs);
                        }
                    }
                }
            }
        }

        all_refs
    }
}
