/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use super::*;
impl WorkspaceIndex {
    /// Finds all references to a symbol, optionally scoped to a target dependency unit.
    pub fn find_references_for_unit<'a>(
        &'a self,
        target_path: &str,
        target_unit_key: Option<&str>,
    ) -> Vec<WorkspaceReference<'a>> {
        let target_unit_key = target_unit_key.filter(|k| !k.is_empty());
        let mut all_refs = Vec::new();
        let mut seen_refs = std::collections::HashSet::new();
        let mut target_crate = target_path.split("::").next().unwrap_or(target_path);
        if !self.readers.iter().any(|r| r.crate_name() == target_crate) {
            for r in &self.readers {
                if r.find_definition_by_path(target_path).is_some() {
                    target_crate = r.crate_name();
                    break;
                }
            }
        }
        // If target_unit_key is given, locate the definition in target_unit to get its line and file
        let target_idx = if let Some(target_unit) = target_unit_key {
            self.unit_keys
                .iter()
                .position(|u| u.as_deref() == Some(target_unit))
        } else {
            self.readers.iter().position(|r| {
                r.crate_name() == target_crate && r.find_definition_by_path(target_path).is_some()
            })
        };

        if let Some(t_idx) = target_idx {
            target_crate = self.readers[t_idx].crate_name();
        }

        let target_def_coords = if let Some(t_idx) = target_idx {
            let r = &self.readers[t_idx];
            let mut coords = None;
            if let Some(def) = r.find_definition_by_path(target_path) {
                let span = def.span();
                let file = r.get_file(span.file_id());
                let full_file = if let Some(Some(root)) = self.source_roots.get(t_idx) {
                    crate::format::join_resolving_overlap(root, Path::new(file))
                        .to_string_lossy()
                        .to_string()
                } else if let Some(ref ws_root) = self.workspace_root {
                    crate::format::join_resolving_overlap(ws_root, Path::new(file))
                        .to_string_lossy()
                        .to_string()
                } else {
                    file.to_string()
                };
                coords = Some((file.to_string(), full_file, span.start_line()));
            } else if let Some(rest) = target_path.strip_prefix(&format!("{target_crate}::")) {
                let mapped = format!("{}::{}", r.crate_name(), rest);
                if let Some(def) = r.find_definition_by_path(&mapped) {
                    let span = def.span();
                    let file = r.get_file(span.file_id());
                    let full_file = self
                        .source_roots
                        .get(t_idx)
                        .and_then(|r| r.as_ref())
                        .or(self.workspace_root.as_ref())
                        .map(|root| {
                            crate::format::join_resolving_overlap(root, Path::new(file))
                                .to_string_lossy()
                                .to_string()
                        })
                        .unwrap_or_else(|| file.to_string());

                    coords = Some((file.to_string(), full_file, span.start_line()));
                }
            }
            coords
        } else {
            None
        };

        let prefix = target_path.split("::").next().unwrap_or(target_path);
        let is_alias_for_target = if let Some(target_unit) = target_unit_key {
            self.reader_extern_units
                .iter()
                .any(|m| m.get(prefix).is_some_and(|uk| uk == target_unit))
        } else {
            false
        };

        let local_path = if target_path.starts_with(&format!("{target_crate}::")) {
            &target_path[target_crate.len() + 2..]
        } else if is_alias_for_target && target_path.contains("::") {
            &target_path[prefix.len() + 2..]
        } else {
            target_path
        };
        let global_path_buf = format!("{target_crate}::{local_path}");
        let global_path = global_path_buf.as_str();

        // The target and the other builds of its crate and version (feature sets compile one
        // source into several units): a consumer reaching any of them uses the item.
        let target_builds: Vec<usize> = target_idx
            .map(|t| {
                let (name, version) = (
                    self.readers[t].crate_name(),
                    self.readers[t].crate_version(),
                );
                (0..self.readers.len())
                    .filter(|&s| {
                        s == t
                            || (self.readers[s].crate_name() == name
                                && self.readers[s].crate_version() == version)
                    })
                    .collect()
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
                if let (Some(target_unit), Some(ru)) = (target_unit_key, reader_unit) {
                    ru == target_unit
                } else {
                    target_idx == Some(i) || reader_crate == target_crate
                };

            let is_sibling_unit = target_idx
                .map(|t| self.are_sibling_workspace_units(t, i))
                .unwrap_or(false);

            let is_same_package = is_exact_target_unit || is_sibling_unit;

            if !is_same_package {
                let is_consumer_of_target = target_unit_key
                    .map(|tu| {
                        let in_deps = self
                            .reader_dependencies
                            .get(i)
                            .map(|d| d.iter().any(|dep| dep == tu))
                            .unwrap_or(false);
                        let in_extern = self
                            .reader_extern_units
                            .get(i)
                            .map(|m| m.values().any(|uk| uk == tu))
                            .unwrap_or(false);
                        in_deps || in_extern
                    })
                    .unwrap_or(false);

                if reader_crate == target_crate && !is_consumer_of_target {
                    // Different package that collides on crate_name but is not a sibling unit.
                    continue;
                }
                // The reader is an external/referring crate.
                // If target_unit_key is given:
                if let Some(target_unit) = target_unit_key {
                    let has_deps = self
                        .reader_dependencies
                        .get(i)
                        .map(|d| !d.is_empty())
                        .unwrap_or(false);
                    let has_extern_units = self
                        .reader_extern_units
                        .get(i)
                        .map(|m| !m.is_empty())
                        .unwrap_or(false);

                    if has_deps || has_extern_units {
                        let in_deps = self
                            .reader_dependencies
                            .get(i)
                            .map(|d| {
                                d.iter()
                                    .any(|dep| dep == target_unit || dep == target_crate)
                            })
                            .unwrap_or(false);
                        let in_extern_units = self
                            .reader_extern_units
                            .get(i)
                            .map(|m| {
                                m.keys().any(|k| k == target_crate)
                                    || m.values().any(|uk| uk == target_unit)
                            })
                            .unwrap_or(false);
                        // A consumer may reach the target through a unit it depends on (an item
                        // used through a re-export): the dependency graph is walked from it.
                        if !in_deps
                            && !in_extern_units
                            && !target_builds
                                .iter()
                                .any(|&t| self.is_reader_in_dep_graph(&[i], t))
                        {
                            // Referring crate has dependency info but depends neither on
                            // target_unit or target_crate nor, through another unit, on the
                            // target. Exclude!
                            continue;
                        }
                    } else {
                        // Legacy reader without dependency info: exclude if multiple versions of target_crate exist in workspace
                        let target_crate_versions = self
                            .readers
                            .iter()
                            .filter(|r| r.crate_name() == target_crate)
                            .count();
                        if target_crate_versions > 1 && reader_crate == target_crate {
                            continue;
                        }
                    }
                }
            }

            // References with canonical target_path (querying both local and global path representations)
            let source_root = self
                .source_roots
                .get(i)
                .and_then(|r| r.as_deref())
                .or(self.workspace_root.as_deref());

            let mut query_path_vec = Vec::new();
            if local_path == global_path {
                query_path_vec.push(target_path);
            } else if is_same_package {
                query_path_vec.push(local_path);
                query_path_vec.push(global_path);
            } else {
                query_path_vec.push(global_path);
            }
            let spellings = super::qualified_path::consumer_spellings(local_path, target_crate);
            query_path_vec.extend(spellings.iter().map(String::as_str));
            for &q_path in &query_path_vec {
                let refs = reader.find_references(q_path);
                for r in refs {
                    let r_target_unit = reader.target_unit(r);
                    if let Some(target_unit) = target_unit_key {
                        if !r_target_unit.is_empty() {
                            if !self.matches_unit_memoized(
                                &mut match_memo,
                                i,
                                r_target_unit,
                                target_unit,
                                target_crate,
                            ) {
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
                        } else if let Some((
                            ref target_def_file,
                            ref target_def_full,
                            target_def_line,
                        )) = target_def_coords
                        {
                            if r.target_line() != 0 && r.target_line() != target_def_line {
                                continue;
                            }
                            if r.target_file_offset() != 0 {
                                let ref_file = reader.get_str(r.target_file_offset());
                                if !ref_file.is_empty() && !target_def_file.is_empty() {
                                    let r_p = Path::new(ref_file);
                                    let t_p = Path::new(target_def_file);
                                    let t_full_p = Path::new(target_def_full);
                                    if r_p != t_p
                                        && r_p != t_full_p
                                        && !ref_file.ends_with(target_def_file)
                                        && !target_def_file.ends_with(ref_file)
                                    {
                                        continue;
                                    }
                                }
                            }
                        }
                    }

                    let span = r.span();
                    let file = reader.get_file(span.file_id());
                    let resolved_path = {
                        let p = Path::new(file);
                        if p.is_absolute() {
                            p.to_path_buf()
                        } else if let Some(root) = source_root {
                            crate::format::join_resolving_overlap(root, p)
                        } else {
                            p.to_path_buf()
                        }
                    };
                    let ref_key = (
                        resolved_path,
                        span.start_line(),
                        span.start_col(),
                        span.end_line(),
                        span.end_col(),
                    );
                    if !seen_refs.insert(ref_key) {
                        continue;
                    }
                    all_refs.push(WorkspaceReference {
                        crate_name: reader_crate,
                        file,
                        span,
                        kind: DefKind::from(r.kind),
                        is_write: r.is_write != 0,
                        is_macro_expansion: r.is_macro_expansion(),
                        source_root,
                        unit_key: reader_unit,
                    });
                }
            }

            // Also check if reader refers to target_path via an extern alias (e.g. `a1::foo` instead of `a::foo`)
            if let Some(extern_map) = self.reader_extern_units.get(i) {
                for (alias, uk) in extern_map {
                    if alias != target_crate {
                        if let Some(target_unit) = target_unit_key {
                            if !self.matches_unit_memoized(
                                &mut match_memo,
                                i,
                                uk.as_str(),
                                target_unit,
                                target_crate,
                            ) {
                                continue;
                            }
                        }
                        let aliased_path = format!("{alias}::{local_path}");
                        let mut aliased =
                            super::qualified_path::consumer_spellings(local_path, alias);
                        aliased.push(aliased_path);
                        for r in aliased.iter().flat_map(|p| reader.find_references(p)) {
                            let r_target_unit = reader.target_unit(r);
                            if let Some(target_unit) = target_unit_key {
                                if !r_target_unit.is_empty() {
                                    if !self.matches_unit_memoized(
                                        &mut match_memo,
                                        i,
                                        r_target_unit,
                                        target_unit,
                                        target_crate,
                                    ) {
                                        continue;
                                    }
                                } else if let Some((
                                    ref target_def_file,
                                    ref target_def_full,
                                    target_def_line,
                                )) = target_def_coords
                                {
                                    if r.target_line() != 0 && r.target_line() != target_def_line {
                                        continue;
                                    }
                                    if r.target_file_offset() != 0 {
                                        let ref_file = reader.get_str(r.target_file_offset());
                                        if !ref_file.is_empty() && !target_def_file.is_empty() {
                                            let r_p = Path::new(ref_file);
                                            let t_p = Path::new(target_def_file);
                                            let t_full_p = Path::new(target_def_full);
                                            if r_p != t_p
                                                && r_p != t_full_p
                                                && !ref_file.ends_with(target_def_file)
                                                && !target_def_file.ends_with(ref_file)
                                            {
                                                continue;
                                            }
                                        }
                                    }
                                }
                            }

                            let span = r.span();
                            let file = reader.get_file(span.file_id());
                            let resolved_path = {
                                let p = Path::new(file);
                                if p.is_absolute() {
                                    p.to_path_buf()
                                } else if let Some(root) = source_root {
                                    crate::format::join_resolving_overlap(root, p)
                                } else {
                                    p.to_path_buf()
                                }
                            };
                            let ref_key = (
                                resolved_path,
                                span.start_line(),
                                span.start_col(),
                                span.end_line(),
                                span.end_col(),
                            );
                            if !seen_refs.insert(ref_key) {
                                continue;
                            }
                            all_refs.push(WorkspaceReference {
                                crate_name: reader_crate,
                                file,
                                span,
                                kind: DefKind::from(r.kind),
                                is_write: r.is_write != 0,
                                is_macro_expansion: r.is_macro_expansion(),
                                source_root,
                                unit_key: reader_unit,
                            });
                        }
                    }
                }
            }
        }

        all_refs
    }
}
