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

/// Whether a definition that starts on `line` can be the one `res` points at. A name alone does
/// not identify the target of a use of an enum variant, which the index holds no definition for:
/// the struct or import of the same name stands in the same file. Such a target with a known
/// line only matches a definition on that line; any other target keeps the match by name.
fn is_on_line_of(res: &ResolvedDefinition<'_>, line: u32) -> bool {
    !matches!(res.kind, DefKind::Variant | DefKind::VariantCtor) || res.start_line == 0 || res.start_line == line
}

impl WorkspaceIndex {
    /// Resolves a cross-crate definition target with compilation-unit and path/signature awareness.
    pub(super) fn find_cross_definition<'a>(&'a self, referrer_idx: usize, res: &ResolvedDefinition<'_>) -> Option<ResolvedDefinition<'a>> {
        let target_path = res.path;
        let extern_alias = target_path.split("::").next().unwrap_or(target_path);

        // 1. Determine target unit key for this extern alias if known
        let target_unit: Option<&str> = res.unit_key.or_else(|| self.reader_extern_units.get(referrer_idx).and_then(|m| m.get(extern_alias).map(|s| s.as_str())));

        // 2. Identify candidate readers: either matching the exact target unit key,
        // or matching extern_alias among referrer's direct dependencies
        let mut candidate_readers = Vec::new();
        if let Some(target_uk) = target_unit {
            for &j in self.readers_for_unit_fast(target_uk).iter() {
                candidate_readers.push((j, &self.readers[j]));
            }
        } else if let Some(deps) = self.reader_dependencies.get(referrer_idx) {
            for dep_uk in deps {
                for &j in self.readers_for_unit_fast(dep_uk).iter() {
                    let r = &self.readers[j];
                    if (r.crate_name() == extern_alias || target_path.starts_with(&format!("{}::", r.crate_name()))) && !candidate_readers.iter().any(|&(idx, _)| idx == j) {
                        candidate_readers.push((j, r));
                    }
                }
            }
            candidate_readers.sort_by_key(|&(j, _)| j);
        }

        if !candidate_readers.is_empty() {
            let mut resolved_candidates = Vec::new();
            for &(j, r) in &candidate_readers {
                let cand_def = if let Some(def) = r.find_definition_by_path(target_path) {
                    Some(def)
                } else if target_path.starts_with(&format!("{extern_alias}::")) {
                    let mapped_path = format!("{}::{}", r.crate_name(), &target_path[extern_alias.len() + 2..]);
                    r.find_definition_by_path(&mapped_path)
                } else if res.file != "external" && !res.file.is_empty() {
                    let file_id_opt = (0..r.header().files_count).find(|&fid| {
                        let f_name = r.get_file(fid);
                        f_name == res.file || res.file.ends_with(f_name) || f_name.ends_with(res.file)
                    });
                    if let Some(fid) = file_id_opt {
                        r.definitions().iter().find(|d| d.span().file_id() == fid && r.get_str(d.name_offset()) == res.name && is_on_line_of(res, d.span().start_line()))
                    } else {
                        None
                    }
                } else {
                    None
                };
                if let Some(def) = cand_def {
                    let def_span = def.span();
                    let file = r.get_file(def_span.file_id());
                    resolved_candidates.push(ResolvedDefinition {
                        name: r.get_str(def.name_offset()),
                        path: r.get_str(def.path_offset()),
                        kind: DefKind::from(def.kind),
                        signature: (def.sig_offset() != 0).then(|| r.get_str(def.sig_offset())),

                        file,
                        start_line: def_span.start_line(),
                        start_col: def_span.start_col(),
                        end_line: def_span.end_line(),
                        end_col: def_span.end_col(),
                        is_macro_expansion: def.is_macro_expansion(),
                        is_reexport: def.is_reexport(),
                        source_root: if self.reader_is_dependency.get(j).copied().unwrap_or(false) {
                            self.source_roots.get(j).and_then(|r| r.as_deref())
                        } else {
                            self.source_roots.get(j).and_then(|r| r.as_deref()).or(self.workspace_root.as_deref())
                        },
                        unit_key: self.unit_keys.get(j).and_then(|u| u.as_deref()),
                        reader_idx: Some(j),
                    });
                }
            }

            if resolved_candidates.len() == 1 {
                return Some(resolved_candidates.remove(0));
            } else if resolved_candidates.len() > 1 {
                for cand in &resolved_candidates {
                    let line_matches = res.start_line != 0 && cand.start_line == res.start_line;
                    let file_matches = res.file != "external"
                        && !res.file.is_empty()
                        && (cand.file == res.file
                            || res.file.ends_with(cand.file)
                            || cand.file.ends_with(res.file)
                            || res.file.ends_with(&*cand.resolved_path().to_string_lossy())
                            || cand.resolved_path().to_string_lossy().ends_with(res.file));
                    if line_matches && file_matches {
                        return Some(cand.clone());
                    }
                }
                for cand in &resolved_candidates {
                    if res.start_line != 0 && cand.start_line == res.start_line {
                        return Some(cand.clone());
                    }
                }
                return Some(resolved_candidates.remove(0));
            }
        }

        // 3. Fallback when referrer has no dependency metadata: search all readers matching target_crate
        let target_crate = extern_alias;
        let mut candidates = Vec::new();
        for &j in self.readers_for_crate_fast(target_crate).iter() {
            let r = &self.readers[j];
            if let Some(def) = r.find_definition_by_path(target_path) {
                let def_span = def.span();
                candidates.push(ResolvedDefinition {
                    name: r.get_str(def.name_offset()),
                    path: r.get_str(def.path_offset()),
                    kind: DefKind::from(def.kind),
                    signature: (def.sig_offset() != 0).then(|| r.get_str(def.sig_offset())),

                    file: r.get_file(def_span.file_id()),
                    start_line: def_span.start_line(),
                    start_col: def_span.start_col(),
                    end_line: def_span.end_line(),
                    end_col: def_span.end_col(),
                    is_macro_expansion: def.is_macro_expansion(),
                    is_reexport: def.is_reexport(),
                    source_root: if self.reader_is_dependency.get(j).copied().unwrap_or(false) {
                        self.source_roots.get(j).and_then(|r| r.as_deref())
                    } else {
                        self.source_roots.get(j).and_then(|r| r.as_deref()).or(self.workspace_root.as_deref())
                    },
                    unit_key: self.unit_keys.get(j).and_then(|u| u.as_deref()),
                    reader_idx: Some(j),
                });
            }
        }

        if candidates.is_empty() && res.file != "external" && !res.file.is_empty() {
            let target_p = Path::new(res.file);
            let fallback_indices: std::borrow::Cow<'_, [usize]> = if let Some(target_uk) = target_unit {
                self.readers_for_unit_fast(target_uk)
            } else if !target_crate.is_empty() {
                self.readers_for_crate_fast(target_crate)
            } else {
                std::borrow::Cow::Owned((0..self.readers.len()).collect())
            };
            for &j in fallback_indices.iter() {
                let r = &self.readers[j];
                let file_id_opt = (0..r.header().files_count).find(|&fid| {
                    let f_name = r.get_file(fid);
                    if target_p.is_absolute() {
                        let cand_p = if let Some(root) = self.source_roots.get(j).and_then(|s| s.as_deref()).or(self.workspace_root.as_deref()) {
                            crate::format::join_resolving_overlap(root, Path::new(f_name))
                        } else {
                            PathBuf::from(f_name)
                        };
                        cand_p == target_p || target_p.ends_with(f_name)
                    } else {
                        f_name == res.file || f_name.ends_with(res.file)
                    }
                });
                if let Some(fid) = file_id_opt {
                    for def in r.definitions() {
                        let d_span = def.span();
                        if d_span.file_id() == fid {
                            let d_name = r.get_str(def.name_offset());
                            if d_name == res.name && is_on_line_of(res, d_span.start_line()) {
                                candidates.push(ResolvedDefinition {
                                    name: d_name,
                                    path: r.get_str(def.path_offset()),
                                    kind: DefKind::from(def.kind),
                                    signature: (def.sig_offset() != 0).then(|| r.get_str(def.sig_offset())),
                                    file: r.get_file(fid),
                                    start_line: d_span.start_line(),
                                    start_col: d_span.start_col(),
                                    end_line: d_span.end_line(),
                                    end_col: d_span.end_col(),
                                    is_macro_expansion: def.is_macro_expansion(),
                                    is_reexport: def.is_reexport(),
                                    source_root: if self.reader_is_dependency.get(j).copied().unwrap_or(false) {
                                        self.source_roots.get(j).and_then(|s| s.as_deref())
                                    } else {
                                        self.source_roots.get(j).and_then(|s| s.as_deref()).or(self.workspace_root.as_deref())
                                    },
                                    unit_key: self.unit_keys.get(j).and_then(|u| u.as_deref()),
                                    reader_idx: Some(j),
                                });
                            }
                        }
                    }
                }
            }
        }

        if candidates.is_empty() {
            return None;
        }
        if candidates.len() == 1 {
            return Some(candidates.remove(0));
        }

        // 4. Disambiguate among multiple candidates:
        // Match by start_line and/or file if available from the reference
        let mut best_candidate = None;
        for c_def in &candidates {
            let line_matches = res.start_line != 0 && c_def.start_line == res.start_line;
            let file_matches = res.file != "external"
                && !res.file.is_empty()
                && (c_def.file == res.file
                    || res.file.ends_with(c_def.file)
                    || c_def.file.ends_with(res.file)
                    || res.file.ends_with(&*c_def.resolved_path().to_string_lossy())
                    || c_def.resolved_path().to_string_lossy().ends_with(res.file));
            if line_matches && file_matches {
                return Some(c_def.clone());
            }
            if (line_matches || file_matches) && best_candidate.is_none() {
                best_candidate = Some(c_def.clone());
            }
        }

        if let Some(cand) = best_candidate {
            return Some(cand);
        }

        // Ambiguous candidates with no exact unit or file/line match: do not guess!
        None
    }

    /// Collects any known re-export alias paths for a symbol within the workspace.
    pub fn collect_reexport_aliases(&self, target_path: &str) -> Vec<String> {
        let mut aliases = Vec::new();
        let target_name = target_path.rsplit("::").next().unwrap_or(target_path);

        for r in &self.readers {
            let crate_name = r.crate_name();
            let mut matches_crate = target_path.starts_with(&format!("{crate_name}::"));
            if !matches_crate {
                if let Some(def) = r.find_definition_by_path(target_path) {
                    if r.get_str(def.name_offset()) == target_name {
                        matches_crate = true;
                    }
                }
            }
            if !matches_crate {
                continue;
            }

            // 1. Root re-export candidate: `<crate_name>::<target_name>`
            let root_export = format!("{crate_name}::{target_name}");
            if root_export != target_path && !aliases.contains(&root_export) {
                aliases.push(root_export);
            }
        }
        aliases.sort();
        aliases.dedup();
        aliases
    }

    /// Checks whether there are references to a re-export of `target_path` that are
    /// not included in `found_refs`.
    pub fn has_unmerged_reexport_references(&self, target_path: &str, found_refs: &[WorkspaceReference<'_>]) -> bool {
        let aliases = self.collect_reexport_aliases(target_path);
        if aliases.is_empty() {
            return false;
        }
        for alias in &aliases {
            for r in &self.readers {
                for aref in r.find_references(alias) {
                    let span = aref.span();
                    let file = r.get_file(span.file_id());
                    let merged = found_refs.iter().any(|fr| fr.file == file && fr.span.start_line() == span.start_line() && fr.span.start_col() == span.start_col());
                    if !merged {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Checks whether `ref_target_unit` matches `target_unit` for `target_crate`.
    /// Handles exact unit match, as well as single-version workspaces where Cargo
    /// assigned different unit hashes (e.g. lib vs test vs dep compilation).
    #[inline]
    pub(crate) fn matches_unit(&self, reader_idx: usize, ref_target_unit: &str, target_unit: &str, target_crate: &str) -> bool {
        self.unit_match.matches_unit(reader_idx, ref_target_unit, target_unit, target_crate, &self.reader_extern_units)
    }

    #[inline]
    pub(super) fn matches_unit_memoized<'a>(
        &self,
        match_memo: &mut std::collections::HashMap<(usize, &'a str), bool>,
        reader_idx: usize,
        ref_target_unit: &'a str,
        target_unit: &str,
        target_crate: &str,
    ) -> bool {
        if ref_target_unit == target_unit {
            true
        } else if let Some(&cached) = match_memo.get(&(reader_idx, ref_target_unit)) {
            cached
        } else {
            let res = self.matches_unit(reader_idx, ref_target_unit, target_unit, target_crate);
            match_memo.insert((reader_idx, ref_target_unit), res);
            res
        }
    }
}
