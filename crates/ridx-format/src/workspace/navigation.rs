/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Cross-crate code navigation, symbol search, hover, and reference tracking.

use super::manifest::{LocationReferences, WorkspaceHover, WorkspaceReference, WorkspaceSymbol};
use super::WorkspaceIndex;
use crate::format::{DefEntry, DefKind};
use crate::reader::ResolvedDefinition;
use std::path::{Path, PathBuf};

impl WorkspaceIndex {
    fn make_resolved_definition<'a>(
        &'a self,
        reader_idx: usize,
        def: &DefEntry,
    ) -> ResolvedDefinition<'a> {
        let reader = &self.readers[reader_idx];
        let span = def.span();
        ResolvedDefinition {
            name: reader.get_str(def.name_offset()),
            path: reader.get_str(def.path_offset()),
            kind: DefKind::from(def.kind),
            signature: if def.sig_offset() != 0 {
                Some(reader.get_str(def.sig_offset()))
            } else {
                None
            },
            file: reader.get_file(span.file_id()),
            start_line: span.start_line(),
            start_col: span.start_col(),
            end_line: span.end_line(),
            end_col: span.end_col(),
            is_macro_expansion: def.is_macro_expansion(),
            source_root: self
                .source_roots
                .get(reader_idx)
                .and_then(|r| r.as_deref())
                .or(self.workspace_root.as_deref()),
            unit_key: self.unit_keys.get(reader_idx).and_then(|u| u.as_deref()),
        }
    }

    /// Finds a definition by full or relative canonical path across the workspace.
    pub fn find_definition_by_path<'a>(&'a self, path: &str) -> Option<ResolvedDefinition<'a>> {
        let resolve_def = |res: ResolvedDefinition<'a>| -> ResolvedDefinition<'a> {
            if res.kind == DefKind::Use {
                if let Some(target) =
                    self.find_definition_at(res.file, res.start_line, res.start_col)
                {
                    if target.kind != DefKind::Use {
                        return target;
                    }
                }
            }
            res
        };

        for candidate in super::std_map::std_path_candidates(path) {
            let candidate_str = candidate.as_str();

            // Fast path: if path starts with `<crate_name>::`, check that crate first
            for (i, reader) in self.readers.iter().enumerate() {
                let prefix = format!("{}::", reader.crate_name());
                if candidate_str.starts_with(&prefix) {
                    let sub = &candidate_str[prefix.len()..];
                    if let Some(def) = reader
                        .find_definition_by_path(candidate_str)
                        .or_else(|| reader.find_definition_by_path(sub))
                    {
                        return Some(resolve_def(self.make_resolved_definition(i, def)));
                    }
                }
            }

            // General search across all crates
            for (i, reader) in self.readers.iter().enumerate() {
                let prefix = format!("{}::", reader.crate_name());
                let sub = candidate_str.strip_prefix(&prefix).unwrap_or(candidate_str);
                if let Some(def) = reader
                    .find_definition_by_path(candidate_str)
                    .or_else(|| reader.find_definition_by_path(sub))
                {
                    return Some(resolve_def(self.make_resolved_definition(i, def)));
                }
            }
        }

        None
    }

    /// Returns the source root path for a loaded crate, if it is an external dependency.
    pub fn crate_source_root(&self, crate_name: &str) -> Option<&Path> {
        for (i, reader) in self.readers.iter().enumerate() {
            if reader.crate_name() == crate_name {
                if let Some(Some(root)) = self.source_roots.get(i) {
                    return Some(root.as_path());
                }
            }
        }
        None
    }

    /// Returns the crate name that contains or owns the specified file path, if known.
    pub fn crate_name_for_file(&self, file_path: &str) -> Option<&str> {
        let p = Path::new(file_path);
        let is_abs = p.is_absolute();
        for (i, reader) in self.readers.iter().enumerate() {
            if let Some(Some(root)) = self.source_roots.get(i) {
                if is_abs && p.starts_with(root) {
                    return Some(reader.crate_name());
                }
            }
        }
        for (i, hashes) in self.reader_file_hashes.iter().enumerate() {
            if hashes.contains_key(file_path) {
                if let Some(reader) = self.readers.get(i) {
                    return Some(reader.crate_name());
                }
            }
        }
        for reader in &self.readers {
            let cname = reader.crate_name();
            if file_path.contains(&format!("/{cname}/"))
                || file_path.contains(&format!("\\{cname}\\"))
            {
                return Some(cname);
            }
        }
        None
    }

    /// Resolves a file path for a given crate.
    /// If the path is relative and the crate is an external dependency with a source root,
    /// joins the source root with the relative path.
    pub fn resolve_path_for_crate(&self, crate_name: &str, file: &str) -> PathBuf {
        let p = Path::new(file);
        if p.is_absolute() {
            return p.to_path_buf();
        }
        if let Some(root) = self.crate_source_root(crate_name) {
            return crate::format::join_resolving_overlap(root, p);
        }
        if let Some(ref ws_root) = self.workspace_root {
            return crate::format::join_resolving_overlap(ws_root, p);
        }
        if let Some(ref root_dir) = self.root_dir {
            return crate::format::join_resolving_overlap(root_dir, p);
        }
        p.to_path_buf()
    }

    /// Finds a definition at a cursor location, seamlessly resolving cross-crate references.
    pub fn find_definition_at<'a>(
        &'a self,
        file: &str,
        line: u32,
        col: u32,
    ) -> Option<ResolvedDefinition<'a>> {
        let p = Path::new(file);
        let is_abs = p.is_absolute();
        let belongs_to_dep = if is_abs {
            self.source_roots.iter().any(|r| {
                if let Some(r) = r.as_ref() {
                    p.starts_with(r)
                } else {
                    false
                }
            })
        } else {
            false
        };

        for (i, reader) in self.readers.iter().enumerate() {
            let target_file = if let Some(Some(root)) = self.source_roots.get(i) {
                if is_abs {
                    if let Ok(rel) = p.strip_prefix(root) {
                        rel.to_str().unwrap_or(file)
                    } else {
                        // The queried file is absolute and outside this dependency's source_root.
                        // Skip this reader to prevent suffix collision.
                        continue;
                    }
                } else {
                    file
                }
            } else {
                if belongs_to_dep {
                    continue;
                }
                file
            };

            if let Some(mut res) = reader.find_definition_at(target_file, line, col) {
                let crate_prefix = format!("{}::", reader.crate_name());
                let current_uk = self
                    .unit_keys
                    .get(i)
                    .and_then(|u| u.as_deref())
                    .unwrap_or_else(|| reader.unit_key());
                let is_explicit_remote = res.unit_key.is_some() && res.unit_key != Some(current_uk);
                let is_local = !is_explicit_remote
                    && (res.path.starts_with(&crate_prefix) || res.path == reader.crate_name());
                if is_local && res.file != "external" {
                    res.source_root = self
                        .source_roots
                        .get(i)
                        .and_then(|r| r.as_deref())
                        .or(self.workspace_root.as_deref());
                    res.unit_key = self.unit_keys.get(i).and_then(|u| u.as_deref());
                    return Some(res);
                }
                // If it resolves to a cross-crate target, attempt resolution in
                // workspace peer crates / dependencies to retrieve the true indexed definition and signature
                if let Some(cross_def) = self.find_cross_definition(i, &res) {
                    return Some(cross_def);
                }
                if res.unit_key.is_none() {
                    res.unit_key = self.unit_keys.get(i).and_then(|u| u.as_deref());
                }
                if res.source_root.is_none() {
                    res.source_root = self
                        .source_roots
                        .get(i)
                        .and_then(|r| r.as_deref())
                        .or(self.workspace_root.as_deref());
                }
                return Some(res);
            }
        }

        None
    }

    /// Resolves a cross-crate definition target with compilation-unit and path/signature awareness.
    fn find_cross_definition<'a>(
        &'a self,
        referrer_idx: usize,
        res: &ResolvedDefinition<'_>,
    ) -> Option<ResolvedDefinition<'a>> {
        let target_path = res.path;
        let extern_alias = target_path.split("::").next().unwrap_or(target_path);

        // 1. Determine target unit key for this extern alias if known
        let target_unit: Option<&str> = res.unit_key.or_else(|| {
            self.reader_extern_units
                .get(referrer_idx)
                .and_then(|m| m.get(extern_alias).map(|s| s.as_str()))
        });

        // 2. Identify candidate readers: either matching the exact target unit key,
        // or matching extern_alias among referrer's direct dependencies
        let mut candidate_readers = Vec::new();
        if let Some(target_uk) = target_unit {
            for (j, r) in self.readers.iter().enumerate() {
                if self.unit_keys.get(j).and_then(|u| u.as_deref()) == Some(target_uk) {
                    candidate_readers.push((j, r));
                }
            }
        } else if let Some(deps) = self.reader_dependencies.get(referrer_idx) {
            for (j, r) in self.readers.iter().enumerate() {
                if let Some(Some(uk)) = self.unit_keys.get(j).map(|u| u.as_deref()) {
                    if deps.iter().any(|d| d == uk)
                        && (r.crate_name() == extern_alias
                            || target_path.starts_with(&format!("{}::", r.crate_name())))
                    {
                        candidate_readers.push((j, r));
                    }
                }
            }
        }

        if !candidate_readers.is_empty() {
            let mut resolved_candidates = Vec::new();
            for &(j, r) in &candidate_readers {
                let cand_def = if let Some(def) = r.find_definition_by_path(target_path) {
                    Some(def)
                } else if target_path.starts_with(&format!("{extern_alias}::")) {
                    let mapped_path = format!(
                        "{}::{}",
                        r.crate_name(),
                        &target_path[extern_alias.len() + 2..]
                    );
                    r.find_definition_by_path(&mapped_path)
                } else if res.file != "external" && !res.file.is_empty() {
                    let file_id_opt = (0..r.header().files_count).find(|&fid| {
                        let f_name = r.get_file(fid);
                        f_name == res.file
                            || res.file.ends_with(f_name)
                            || f_name.ends_with(res.file)
                    });
                    if let Some(fid) = file_id_opt {
                        r.definitions().iter().find(|d| {
                            d.span().file_id() == fid && r.get_str(d.name_offset()) == res.name
                        })
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
                        source_root: self
                            .source_roots
                            .get(j)
                            .and_then(|r| r.as_deref())
                            .or(self.workspace_root.as_deref()),
                        unit_key: self.unit_keys.get(j).and_then(|u| u.as_deref()),
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
        for (j, r) in self.readers.iter().enumerate() {
            if r.crate_name() == target_crate {
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
                        source_root: self
                            .source_roots
                            .get(j)
                            .and_then(|r| r.as_deref())
                            .or(self.workspace_root.as_deref()),
                        unit_key: self.unit_keys.get(j).and_then(|u| u.as_deref()),
                    });
                }
            }
        }

        if candidates.is_empty() && res.file != "external" && !res.file.is_empty() {
            let target_p = Path::new(res.file);
            for (j, r) in self.readers.iter().enumerate() {
                if let Some(target_uk) = target_unit {
                    if self.unit_keys.get(j).and_then(|u| u.as_deref()) != Some(target_uk) {
                        continue;
                    }
                } else if !target_crate.is_empty() && r.crate_name() != target_crate {
                    continue;
                }
                let file_id_opt = (0..r.header().files_count).find(|&fid| {
                    let f_name = r.get_file(fid);
                    if target_p.is_absolute() {
                        let cand_p = if let Some(root) = self
                            .source_roots
                            .get(j)
                            .and_then(|s| s.as_deref())
                            .or(self.workspace_root.as_deref())
                        {
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
                            if d_name == res.name {
                                candidates.push(ResolvedDefinition {
                                    name: d_name,
                                    path: r.get_str(def.path_offset()),
                                    kind: DefKind::from(def.kind),
                                    signature: (def.sig_offset() != 0)
                                        .then(|| r.get_str(def.sig_offset())),
                                    file: r.get_file(fid),
                                    start_line: d_span.start_line(),
                                    start_col: d_span.start_col(),
                                    end_line: d_span.end_line(),
                                    end_col: d_span.end_col(),
                                    is_macro_expansion: def.is_macro_expansion(),
                                    source_root: self
                                        .source_roots
                                        .get(j)
                                        .and_then(|s| s.as_deref())
                                        .or(self.workspace_root.as_deref()),
                                    unit_key: self.unit_keys.get(j).and_then(|u| u.as_deref()),
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
                    || res
                        .file
                        .ends_with(&*c_def.resolved_path().to_string_lossy())
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
    pub fn has_unmerged_reexport_references(
        &self,
        target_path: &str,
        found_refs: &[WorkspaceReference<'_>],
    ) -> bool {
        let aliases = self.collect_reexport_aliases(target_path);
        if aliases.is_empty() {
            return false;
        }
        for alias in &aliases {
            for r in &self.readers {
                for aref in r.find_references(alias) {
                    let span = aref.span();
                    let file = r.get_file(span.file_id());
                    let merged = found_refs.iter().any(|fr| {
                        fr.file == file
                            && fr.span.start_line() == span.start_line()
                            && fr.span.start_col() == span.start_col()
                    });
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
    fn matches_unit(
        &self,
        reader_idx: usize,
        ref_target_unit: &str,
        target_unit: &str,
        target_crate: &str,
    ) -> bool {
        if ref_target_unit == target_unit {
            return true;
        }
        let target_unit_in_crate = self.readers.iter().enumerate().any(|(j, r)| {
            r.crate_name() == target_crate
                && self.unit_keys.get(j).and_then(|u| u.as_deref()) == Some(target_unit)
        }) || self
            .reader_extern_units
            .iter()
            .any(|m| m.get(target_crate).map(|u| u.as_str()) == Some(target_unit));
        if !target_unit_in_crate {
            return false;
        }

        let is_reader_extern_target = self
            .reader_extern_units
            .get(reader_idx)
            .and_then(|m| m.get(target_crate))
            .map(|u| u.as_str())
            == Some(ref_target_unit);
        let target_versions_count = self
            .readers
            .iter()
            .filter(|r| r.crate_name() == target_crate)
            .map(|r| r.crate_version())
            .filter(|v| !v.is_empty())
            .collect::<std::collections::HashSet<_>>()
            .len();
        let belongs_to_target_crate = is_reader_extern_target
            || self.readers.iter().enumerate().any(|(j, r)| {
                r.crate_name() == target_crate
                    && self.unit_keys.get(j).and_then(|u| u.as_deref()) == Some(ref_target_unit)
            });
        belongs_to_target_crate && target_versions_count <= 1
    }

    /// Finds all references to a symbol, optionally scoped to a target dependency unit.
    pub fn find_references_for_unit<'a>(
        &'a self,
        target_path: &str,
        target_unit_key: Option<&str>,
    ) -> Vec<WorkspaceReference<'a>> {
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
        let target_def_coords = if let Some(target_unit) = target_unit_key {
            let mut coords = None;
            for (j, r) in self.readers.iter().enumerate() {
                if self.unit_keys.get(j).and_then(|u| u.as_deref()) == Some(target_unit) {
                    target_crate = r.crate_name();
                    if let Some(def) = r.find_definition_by_path(target_path) {
                        let span = def.span();
                        let file = r.get_file(span.file_id());
                        let full_file = if let Some(Some(root)) = self.source_roots.get(j) {
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
                        break;
                    }
                    if let Some(rest) = target_path.strip_prefix(&format!("{target_crate}::")) {
                        let mapped = format!("{}::{}", r.crate_name(), rest);
                        if let Some(def) = r.find_definition_by_path(&mapped) {
                            let span = def.span();
                            let file = r.get_file(span.file_id());
                            let full_file = self
                                .source_roots
                                .get(j)
                                .and_then(|r| r.as_ref())
                                .or(self.workspace_root.as_ref())
                                .map(|root| {
                                    crate::format::join_resolving_overlap(root, Path::new(file))
                                        .to_string_lossy()
                                        .to_string()
                                })
                                .unwrap_or_else(|| file.to_string());

                            coords = Some((file.to_string(), full_file, span.start_line()));
                            break;
                        }
                    }
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

        for (i, reader) in self.readers.iter().enumerate() {
            let reader_unit = self.unit_keys.get(i).and_then(|u| u.as_deref());
            let reader_crate = reader.crate_name();

            let is_target_unit =
                if let (Some(target_unit), Some(ru)) = (target_unit_key, reader_unit) {
                    ru == target_unit
                } else {
                    reader_crate == target_crate
                };

            if !is_target_unit {
                // The reader is an external/referring crate (or a different unit of the same crate name).
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
                        if !in_deps && !in_extern_units {
                            // Referring crate has dependency info but does NOT depend on target_unit or target_crate. Exclude!
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
            } else if is_target_unit {
                query_path_vec.push(local_path);
                query_path_vec.push(global_path);
            } else {
                query_path_vec.push(global_path);
            }
            for &q_path in &query_path_vec {
                let refs = reader.find_references(q_path);
                for r in refs {
                    let r_target_unit = reader.target_unit(r);
                    if let Some(target_unit) = target_unit_key {
                        if !r_target_unit.is_empty() {
                            if !self.matches_unit(i, r_target_unit, target_unit, target_crate) {
                                continue;
                            }
                        } else if reader_crate == target_crate {
                            if let Some(ru) = reader_unit {
                                if ru != target_unit {
                                    continue;
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
                    let ref_key = (
                        file,
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
                            if !self.matches_unit(i, uk, target_unit, target_crate) {
                                continue;
                            }
                        }
                        let aliased_path = format!("{alias}::{local_path}");
                        for r in reader.find_references(&aliased_path) {
                            let r_target_unit = reader.target_unit(r);
                            if let Some(target_unit) = target_unit_key {
                                if !r_target_unit.is_empty() {
                                    if !self.matches_unit(
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
                            let ref_key = (
                                file,
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

    /// Finds all references to a symbol across every crate in the workspace.
    pub fn find_references<'a>(&'a self, target_path: &str) -> Vec<WorkspaceReference<'a>> {
        self.find_references_for_unit(target_path, None)
    }

    /// Inferred expression type at cursor location.
    pub fn type_at(&self, file: &str, line: u32, col: u32) -> Option<&str> {
        let p = Path::new(file);
        let is_abs = p.is_absolute();
        let belongs_to_dep = if is_abs {
            self.source_roots.iter().any(|r| {
                if let Some(r) = r.as_ref() {
                    p.starts_with(r)
                } else {
                    false
                }
            })
        } else {
            false
        };

        for (i, reader) in self.readers.iter().enumerate() {
            let target_file = if let Some(Some(root)) = self.source_roots.get(i) {
                if is_abs {
                    if let Ok(rel) = p.strip_prefix(root) {
                        rel.to_str().unwrap_or(file)
                    } else {
                        continue;
                    }
                } else {
                    file
                }
            } else {
                if belongs_to_dep {
                    continue;
                }
                file
            };
            if let Some(ty) = reader.type_at(target_file, line, col) {
                return Some(ty);
            }
        }
        None
    }

    /// Inferred expression type and optional canonical owner path at cursor location.
    pub fn type_entry_at(&self, file: &str, line: u32, col: u32) -> Option<(&str, Option<&str>)> {
        let p = Path::new(file);
        let is_abs = p.is_absolute();
        let belongs_to_dep = if is_abs {
            self.source_roots.iter().any(|r| {
                if let Some(r) = r.as_ref() {
                    p.starts_with(r)
                } else {
                    false
                }
            })
        } else {
            false
        };

        for (i, reader) in self.readers.iter().enumerate() {
            let target_file = if let Some(Some(root)) = self.source_roots.get(i) {
                if is_abs {
                    if let Ok(rel) = p.strip_prefix(root) {
                        rel.to_str().unwrap_or(file)
                    } else {
                        continue;
                    }
                } else {
                    file
                }
            } else {
                if belongs_to_dep {
                    continue;
                }
                file
            };
            if let Some(res) = reader.type_entry_at(target_file, line, col) {
                return Some(res);
            }
        }
        None
    }

    /// Finds all definitions belonging to an owner (canonical type path or module path) across the workspace.
    pub fn find_definitions_by_owner<'a>(&'a self, owner: &str) -> Vec<ResolvedDefinition<'a>> {
        let mut results = Vec::new();
        let mut seen = std::collections::HashSet::new();

        let candidates = super::std_map::std_path_candidates(owner);
        for cand in &candidates {
            let cand_str = cand.as_str();
            for (i, reader) in self.readers.iter().enumerate() {
                let prefix = format!("{}::", reader.crate_name());
                let sub = cand_str.strip_prefix(&prefix).unwrap_or(cand_str);
                let with_prefix = format!("{prefix}{cand_str}");

                for def in reader
                    .find_definitions_by_owner(cand_str)
                    .into_iter()
                    .chain(reader.find_definitions_by_owner(sub))
                    .chain(reader.find_definitions_by_owner(&with_prefix))
                {
                    let key = (i, def.path_offset(), def.span.start_line, def.span.start_col);
                    if seen.insert(key) {
                        results.push(self.make_resolved_definition(i, def));
                    }
                }
            }
        }

        results
    }

    /// Resolves the autoderef chain starting from `source` up to `max_depth` steps across the workspace.
    pub fn resolve_deref_chain(&self, source: &str, max_depth: usize) -> Vec<&str> {
        let mut chain = Vec::new();
        let mut current = source;
        let mut visited = std::collections::HashSet::new();
        visited.insert(source);

        for _ in 0..max_depth {
            let mut found = None;
            for reader in &self.readers {
                if let Some(target) = reader.find_deref_target(current) {
                    found = Some(target);
                    break;
                }
            }
            if let Some(target) = found {
                if target.is_empty() || !visited.insert(target) {
                    break;
                }
                chain.push(target);
                current = target;
            } else {
                break;
            }
        }

        chain
    }

    /// Substring symbol search across all definitions in the workspace.
    pub fn search_symbols<'a>(&'a self, query: &str) -> Vec<WorkspaceSymbol<'a>> {
        let mut matches = Vec::new();
        let mut seen = std::collections::HashSet::new();

        let queries = super::std_map::std_path_candidates(query);
        for q in &queries {
            let q_ascii_lower = if q.is_ascii() {
                Some(q.to_ascii_lowercase())
            } else {
                None
            };
            for (i, reader) in self.readers.iter().enumerate() {
                let crate_name = reader.crate_name();
                let defs = match &q_ascii_lower {
                    Some(lower) => reader.search_symbols_ascii(lower.as_bytes()),
                    None => reader.search_symbols(q),
                };
                let source_root = self
                    .source_roots
                    .get(i)
                    .and_then(|r| r.as_deref())
                    .or(self.workspace_root.as_deref());
                let unit_key = self.unit_keys.get(i).and_then(|u| u.as_deref());

                for def in defs {
                    let name = reader.get_str(def.name_offset());
                    let path = reader.get_str(def.path_offset());
                    let span = def.span();
                    let file = reader.get_file(span.file_id());
                    let sym_key = (path, file, span.start_line(), span.start_col());
                    if !seen.insert(sym_key) {
                        continue;
                    }
                    let signature = if def.sig_offset() != 0 {
                        Some(reader.get_str(def.sig_offset()))
                    } else {
                        None
                    };

                    matches.push(WorkspaceSymbol {
                        crate_name,
                        name,
                        path,
                        kind: DefKind::from(def.kind),
                        file,
                        span,
                        signature,
                        is_macro_expansion: def.is_macro_expansion(),
                        source_root,
                        unit_key,
                    });
                }
            }
        }

        matches
    }

    /// Finds all references to the symbol at cursor location across the workspace.
    pub fn find_references_at<'a>(
        &'a self,
        file: &str,
        line: u32,
        col: u32,
    ) -> Option<LocationReferences<'a>> {
        let def = self.find_definition_at(file, line, col)?;
        let mut refs = self.find_references_for_unit(def.path, def.unit_key);
        // For local bindings whose references are stored by local name, try by short name if needed
        if refs.is_empty() && def.kind == DefKind::Local && def.name != def.path {
            refs = self.find_references_for_unit(def.name, def.unit_key);
        }
        Some(LocationReferences {
            target_symbol: def.name,
            target_kind: def.kind,
            target_path: def.path,
            target_file: def.file,
            target_line: def.start_line,
            target_col: def.start_col,
            target_end_line: def.end_line,
            target_end_col: def.end_col,
            target_source_root: def.source_root,
            target_unit_key: def.unit_key,
            references: refs,
        })
    }

    /// Complete hover information (definition + signature + inferred type) at cursor location.
    pub fn hover_at<'a>(&'a self, file: &str, line: u32, col: u32) -> Option<WorkspaceHover<'a>> {
        let def = self.find_definition_at(file, line, col);
        let ty = self.type_at(file, line, col);
        if def.is_none() && ty.is_none() {
            return None;
        }
        Some(WorkspaceHover {
            definition: def,
            inferred_type: ty,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::DefKind;
    use crate::writer::{IndexWriter, RawDefinition, RawReference, RawType};
    use std::collections::HashMap;
    use std::fs::{self, File};

    struct TestDir(PathBuf);
    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn new_test_dir(prefix: &str) -> TestDir {
        static CNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let id = CNT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!("{}_{}_{}", prefix, std::process::id(), id));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        TestDir(path)
    }

    #[test]
    fn test_navigation_comprehensive() {
        let test_dir = new_test_dir("ridx_nav_test");
        let index_path = test_dir.0.join("sample.ridx");

        let mut writer = IndexWriter::new("sample", "0.1.0");
        writer.set_unit_key("unit1");

        writer.add_definition(RawDefinition {
            def_id: 1,
            kind: DefKind::Fn,
            visibility: 0,
            is_macro_expansion: false,
            name: "sample_fn".to_string(),
            path: "sample::sample_fn".to_string(),
            signature: Some("fn()".to_string()),
            file: "src/lib.rs".to_string(),
            start_line: 10,
            start_col: 1,
            end_line: 12,
            end_col: 2,
        });

        writer.add_definition(RawDefinition {
            def_id: 2,
            kind: DefKind::Local,
            visibility: 0,
            is_macro_expansion: false,
            name: "local_var".to_string(),
            path: "sample::sample_fn::local_var".to_string(),
            signature: None,
            file: "src/lib.rs".to_string(),
            start_line: 11,
            start_col: 5,
            end_line: 11,
            end_col: 14,
        });

        writer.add_reference(RawReference {
            file: "src/main.rs".to_string(),
            start_line: 20,
            start_col: 5,
            end_line: 20,
            end_col: 14,
            target_def_id: Some(1),
            target_path: "sample::sample_fn".to_string(),
            target_unit: Some("unit1".to_string()),
            target_file: Some("src/lib.rs".to_string()),
            target_line: 10,
            target_col: 1,
            kind: DefKind::Fn,
            is_write: false,
            is_macro_expansion: false,
        });

        writer.add_reference(RawReference {
            file: "src/lib.rs".to_string(),
            start_line: 11,
            start_col: 20,
            end_line: 11,
            end_col: 29,
            target_def_id: Some(2),
            target_path: "local_var".to_string(),
            target_unit: None,
            target_file: Some("src/lib.rs".to_string()),
            target_line: 11,
            target_col: 5,
            kind: DefKind::Local,
            is_write: false,
            is_macro_expansion: false,
        });

        writer.add_type(RawType {
            file: "src/lib.rs".to_string(),
            start_line: 10,
            start_col: 1,
            end_line: 10,
            end_col: 9,
            type_name: "fn()".to_string(),
        });

        let mut file = File::create(&index_path).unwrap();
        writer.write_to(&mut file).unwrap();

        let ws = WorkspaceIndex::open(&test_dir.0).unwrap();

        // 1. find_definition_by_path
        let def_prefixed = ws.find_definition_by_path("sample::sample_fn");
        assert!(def_prefixed.is_some());
        assert_eq!(def_prefixed.unwrap().name, "sample_fn");

        let def_unprefixed = ws.find_definition_by_path("sample_fn");
        assert!(def_unprefixed.is_none());

        // 2. crate_source_root
        assert!(ws.crate_source_root("sample").is_none());
        assert!(ws.crate_source_root("non_existent").is_none());

        // 3. resolve_path_for_crate
        let abs = ws.resolve_path_for_crate("sample", "/abs/path.rs");
        assert_eq!(abs, PathBuf::from("/abs/path.rs"));

        let rel = ws.resolve_path_for_crate("sample", "src/lib.rs");
        assert!(rel.ends_with("src/lib.rs"));

        // 4. find_definition_at
        let def_at_def = ws.find_definition_at("src/lib.rs", 10, 2);
        assert!(def_at_def.is_some());
        assert_eq!(def_at_def.unwrap().name, "sample_fn");

        let def_at_ref = ws.find_definition_at("src/main.rs", 20, 6);
        assert!(def_at_ref.is_some());
        assert_eq!(def_at_ref.unwrap().name, "sample_fn");

        let def_none = ws.find_definition_at("src/lib.rs", 99, 99);
        assert!(def_none.is_none());

        // 5. find_references and find_references_for_unit
        let refs = ws.find_references("sample::sample_fn");
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].file, "src/main.rs");

        // 6. type_at
        assert_eq!(ws.type_at("src/lib.rs", 10, 2), Some("fn()"));
        assert_eq!(ws.type_at("src/lib.rs", 99, 99), None);

        // 7. search_symbols
        let syms = ws.search_symbols("sample_fn");
        assert_eq!(syms.len(), 2);
        assert_eq!(syms[0].name, "sample_fn");

        // 8. find_references_at
        let ref_at = ws.find_references_at("src/lib.rs", 10, 2);
        assert!(ref_at.is_some());
        assert_eq!(ref_at.unwrap().references.len(), 1);

        let local_ref_at = ws.find_references_at("src/lib.rs", 11, 6);
        assert!(local_ref_at.is_some());
        assert_eq!(local_ref_at.unwrap().references.len(), 1);

        assert!(ws.find_references_at("src/lib.rs", 99, 99).is_none());

        // 9. hover_at
        let hover = ws.hover_at("src/lib.rs", 10, 2);
        assert!(hover.is_some());
        let h = hover.unwrap();
        assert!(h.definition.is_some());
        assert_eq!(h.inferred_type, Some("fn()"));

        assert!(ws.hover_at("src/lib.rs", 99, 99).is_none());

        // 10. find_references_for_unit
        let refs_scoped = ws.find_references_for_unit("sample::sample_fn", Some("unit1"));
        assert_eq!(refs_scoped.len(), 1);

        let refs_scoped_none = ws.find_references_for_unit("sample::sample_fn", Some("other_unit"));
        assert_eq!(refs_scoped_none.len(), 0);

        // 11. find_definition_by_path
        assert!(ws.find_definition_by_path("sample::sample_fn").is_some());
        assert!(ws.find_definition_by_path("non_existent").is_none());
    }

    #[test]
    fn test_cross_crate_navigation() {
        let test_dir = new_test_dir("ridx_cross_nav");

        // Crate 1: dep_crate
        let mut dep_writer = IndexWriter::new("dep_crate", "0.1.0");
        dep_writer.set_unit_key("dep_unit");
        dep_writer.add_definition(RawDefinition {
            def_id: 1,
            kind: DefKind::Fn,
            visibility: 1,
            is_macro_expansion: false,
            name: "dep_func".to_string(),
            path: "dep_crate::dep_func".to_string(),
            signature: Some("pub fn dep_func()".to_string()),
            file: "src/dep.rs".to_string(),
            start_line: 5,
            start_col: 1,
            end_line: 7,
            end_col: 2,
        });
        let mut f1 = File::create(test_dir.0.join("dep_crate.ridx")).unwrap();
        dep_writer.write_to(&mut f1).unwrap();

        // Crate 2: app_crate
        let mut app_writer = IndexWriter::new("app_crate", "0.1.0");
        app_writer.set_unit_key("app_unit");
        app_writer.add_reference(RawReference {
            file: "src/main.rs".to_string(),
            start_line: 15,
            start_col: 5,
            end_line: 15,
            end_col: 13,
            target_def_id: None,
            target_path: "dep_crate::dep_func".to_string(),
            target_unit: Some("dep_unit".to_string()),
            target_file: None,
            target_line: 0,
            target_col: 0,
            kind: DefKind::Fn,
            is_write: false,
            is_macro_expansion: false,
        });
        let mut f2 = File::create(test_dir.0.join("app_crate.ridx")).unwrap();
        app_writer.write_to(&mut f2).unwrap();

        let ws = WorkspaceIndex::open(&test_dir.0).unwrap();
        let def = ws.find_definition_at("src/main.rs", 15, 6);
        assert!(def.is_some());
        let d = def.unwrap();
        assert_eq!(d.name, "dep_func");
        assert_eq!(d.file, "src/dep.rs");
    }

    #[test]
    fn test_cross_crate_navigation_candidate_disambiguation() {
        let test_dir = new_test_dir("ridx_cross_disambig");

        let mut dep1 = IndexWriter::new("multi_crate", "0.1.0");
        dep1.set_unit_key("multi_unit_1");
        dep1.add_definition(RawDefinition {
            def_id: 1,
            kind: DefKind::Fn,
            visibility: 0,
            is_macro_expansion: false,
            name: "target_func".to_string(),
            path: "multi_crate::target_func".to_string(),
            signature: None,
            file: "src/lib.rs".to_string(),
            start_line: 10,
            start_col: 1,
            end_line: 10,
            end_col: 20,
        });
        let mut f1 = File::create(test_dir.0.join("multi_crate_1.ridx")).unwrap();
        dep1.write_to(&mut f1).unwrap();

        let mut dep2 = IndexWriter::new("multi_crate", "0.2.0");
        dep2.set_unit_key("multi_unit_2");
        dep2.add_definition(RawDefinition {
            def_id: 1,
            kind: DefKind::Fn,
            visibility: 0,
            is_macro_expansion: false,
            name: "target_func".to_string(),
            path: "multi_crate::target_func".to_string(),
            signature: None,
            file: "src/lib.rs".to_string(),
            start_line: 50,
            start_col: 1,
            end_line: 50,
            end_col: 20,
        });
        let mut f2 = File::create(test_dir.0.join("multi_crate_2.ridx")).unwrap();
        dep2.write_to(&mut f2).unwrap();

        let mut caller = IndexWriter::new("caller_crate", "0.1.0");
        caller.add_reference(RawReference {
            file: "src/main.rs".to_string(),
            start_line: 5,
            start_col: 5,
            end_line: 5,
            end_col: 16,
            target_def_id: None,
            target_path: "multi_crate::target_func".to_string(),
            target_unit: None,
            target_file: Some("src/lib.rs".to_string()),
            target_line: 50,
            target_col: 1,
            kind: DefKind::Fn,
            is_write: false,
            is_macro_expansion: false,
        });
        let mut f3 = File::create(test_dir.0.join("caller_crate.ridx")).unwrap();
        caller.write_to(&mut f3).unwrap();

        let ws = WorkspaceIndex::open(&test_dir.0).unwrap();
        let def = ws.find_definition_at("src/main.rs", 5, 6);
        assert!(def.is_some());
        let d = def.unwrap();
        assert_eq!(d.start_line, 50);
    }

    #[test]
    fn test_navigation_edge_cases() {
        let test_dir = new_test_dir("ridx_nav_edges");

        let mut w = IndexWriter::new("edge_crate", "0.1.0");
        w.add_definition(RawDefinition {
            def_id: 1,
            kind: DefKind::Fn,
            visibility: 0,
            is_macro_expansion: false,
            name: "bare_fn".to_string(),
            path: "bare_fn".to_string(),
            signature: Some("fn bare_fn()".to_string()),
            file: "src/lib.rs".to_string(),
            start_line: 1,
            start_col: 1,
            end_line: 1,
            end_col: 15,
        });
        let mut f = File::create(test_dir.0.join("edge.ridx")).unwrap();
        w.write_to(&mut f).unwrap();

        let ws = WorkspaceIndex::open(&test_dir.0).unwrap();

        // 1. find_definition_by_path with bare path
        let def = ws.find_definition_by_path("bare_fn");
        assert!(def.is_some());
        assert_eq!(def.unwrap().name, "bare_fn");

        // 2. resolve_path_for_crate
        let abs = Path::new("/abs/path/to/file.rs");
        assert_eq!(
            ws.resolve_path_for_crate("edge_crate", abs.to_str().unwrap()),
            abs
        );
        let rel_res = ws.resolve_path_for_crate("edge_crate", "src/lib.rs");
        assert!(rel_res.ends_with("src/lib.rs"));

        // 3. crate_source_root
        assert!(ws.crate_source_root("nonexistent").is_none());

        // 4. find_references_for_unit
        let refs = ws.find_references_for_unit("bare_fn", Some("nonexistent_unit"));
        assert!(refs.is_empty());
    }

    #[test]
    fn test_cross_crate_navigation_disambiguation_and_coords() {
        let test_dir = new_test_dir("ridx_nav_disambig");

        // Crate 1: caller
        let mut w1 = IndexWriter::new("caller", "0.1.0");
        w1.set_unit_key("caller:0.1.0");
        w1.add_reference(RawReference {
            file: "src/main.rs".to_string(),
            start_line: 10,
            start_col: 5,
            end_line: 10,
            end_col: 15,
            target_def_id: None,
            target_path: "dep::target_fn".to_string(),
            target_unit: Some("dep:0.1.0".to_string()),
            target_file: Some("src/lib.rs".to_string()),
            target_line: 20,
            target_col: 1,
            kind: DefKind::Fn,
            is_write: false,
            is_macro_expansion: false,
        });

        // Crate 2: dep (two versions or candidates)
        let mut w2 = IndexWriter::new("dep", "0.1.0");
        w2.set_unit_key("dep:0.1.0");
        w2.add_definition(RawDefinition {
            def_id: 1,
            kind: DefKind::Fn,
            visibility: 0,
            is_macro_expansion: false,
            name: "target_fn".to_string(),
            path: "dep::target_fn".to_string(),
            signature: Some("fn target_fn()".to_string()),
            file: "src/lib.rs".to_string(),
            start_line: 20,
            start_col: 1,
            end_line: 20,
            end_col: 15,
        });

        let mut w3 = IndexWriter::new("dep", "0.2.0");
        w3.set_unit_key("dep:0.2.0");
        w3.add_definition(RawDefinition {
            def_id: 1,
            kind: DefKind::Fn,
            visibility: 0,
            is_macro_expansion: false,
            name: "target_fn".to_string(),
            path: "dep::target_fn".to_string(),
            signature: Some("fn target_fn_v2()".to_string()),
            file: "src/lib.rs".to_string(),
            start_line: 25,
            start_col: 1,
            end_line: 25,
            end_col: 15,
        });

        let mut f1 = File::create(test_dir.0.join("caller.ridx")).unwrap();
        w1.write_to(&mut f1).unwrap();
        drop(f1);

        let mut f2 = File::create(test_dir.0.join("dep1.ridx")).unwrap();
        w2.write_to(&mut f2).unwrap();
        drop(f2);

        let mut f3 = File::create(test_dir.0.join("dep2.ridx")).unwrap();
        w3.write_to(&mut f3).unwrap();
        drop(f3);

        let ws = WorkspaceIndex::open(&test_dir.0).unwrap();

        // 1. Definition at cursor in caller resolving to exact dep unit
        let def = ws.find_definition_at("src/main.rs", 10, 8);
        assert!(def.is_some());
        let d = def.unwrap();
        assert_eq!(d.name, "target_fn");
        assert_eq!(d.signature, Some("fn target_fn()"));

        // 2. find_references for dep::target_fn
        let refs = ws.find_references("dep::target_fn");
        assert_eq!(refs.len(), 1);

        // 3. find_references_for_unit with exact matching unit
        let refs_exact = ws.find_references_for_unit("dep::target_fn", Some("dep:0.1.0"));
        assert_eq!(refs_exact.len(), 1);

        // 4. find_references_for_unit with non-matching unit
        let refs_none = ws.find_references_for_unit("dep::target_fn", Some("dep:0.2.0"));
        assert_eq!(refs_none.len(), 0);

        // 5. type_at
        let ty = ws.type_at("src/main.rs", 10, 8);
        assert_eq!(ty, None);
    }

    #[test]
    fn test_cross_crate_navigation_candidates_and_type_at() {
        let test_dir = new_test_dir("ridx_cand_nav");
        let manifest_file = test_dir.0.join("ridx_manifest.json");

        let dep_root = test_dir.0.join("dep_src");
        fs::create_dir_all(dep_root.join("src")).unwrap();
        fs::write(dep_root.join("src/lib.rs"), "pub fn my_fn() {}\n").unwrap();

        let mut w_caller = IndexWriter::new("caller", "0.1.0");
        w_caller.set_unit_key("caller_key");
        // Reference to unresolvable cross-crate symbol (hits fallback lines 181-192 in find_definition_at)
        w_caller.add_reference(RawReference {
            file: "src/main.rs".to_string(),
            start_line: 15,
            start_col: 5,
            end_line: 15,
            end_col: 15,
            target_def_id: None,
            target_path: "missing_dep::missing_fn".to_string(),
            target_unit: None,
            target_file: Some("src/lib.rs".to_string()),
            target_line: 10,
            target_col: 1,
            kind: DefKind::Fn,
            is_write: false,
            is_macro_expansion: false,
        });

        // Reference to ambiguous candidates (hits lines 293-298)
        w_caller.add_reference(RawReference {
            file: "src/main.rs".to_string(),
            start_line: 30,
            start_col: 5,
            end_line: 30,
            end_col: 15,
            target_def_id: None,
            target_path: "ambig::func".to_string(),
            target_unit: None,
            target_file: Some("other/file.rs".to_string()),
            target_line: 50,
            target_col: 1,
            kind: DefKind::Fn,
            is_write: false,
            is_macro_expansion: false,
        });

        // Reference matching line 50 and file src/a.rs (hits lines 289-291)
        w_caller.add_reference(RawReference {
            file: "src/main.rs".to_string(),
            start_line: 35,
            start_col: 5,
            end_line: 35,
            end_col: 15,
            target_def_id: None,
            target_path: "ambig::func".to_string(),
            target_unit: None,
            target_file: Some("src/a.rs".to_string()),
            target_line: 50,
            target_col: 1,
            kind: DefKind::Fn,
            is_write: false,
            is_macro_expansion: false,
        });

        // Reference where target_line does not match (hits line 511 continue)
        w_caller.add_reference(RawReference {
            file: "src/main.rs".to_string(),
            start_line: 40,
            start_col: 5,
            end_line: 40,
            end_col: 15,
            target_def_id: None,
            target_path: "ambig::func".to_string(),
            target_unit: None,
            target_file: Some("src/a.rs".to_string()),
            target_line: 99,
            target_col: 1,
            kind: DefKind::Fn,
            is_write: false,
            is_macro_expansion: false,
        });

        // Reference with existing target_unit but missing symbol (hits lines 300-302)
        w_caller.add_reference(RawReference {
            file: "src/main.rs".to_string(),
            start_line: 45,
            start_col: 5,
            end_line: 45,
            end_col: 15,
            target_def_id: None,
            target_path: "ambig::nonexistent_symbol".to_string(),
            target_unit: Some("ambig_key1".to_string()),
            target_file: None,
            target_line: 0,
            target_col: 0,
            kind: DefKind::Fn,
            is_write: false,
            is_macro_expansion: false,
        });

        // Add a type in caller
        w_caller.add_type(RawType {
            file: "src/main.rs".to_string(),
            start_line: 15,
            start_col: 5,
            end_line: 15,
            end_col: 15,
            type_name: "MyType".to_string(),
        });

        let var_dir = test_dir.0.join("variants/var_nav");
        fs::create_dir_all(&var_dir).unwrap();

        let mut f_caller = File::create(var_dir.join("caller.ridx")).unwrap();
        w_caller.write_to(&mut f_caller).unwrap();
        drop(f_caller);

        // Ambig shard 1
        let mut w_a1 = IndexWriter::new("ambig", "1.0.0");
        w_a1.set_unit_key("ambig_key1");
        w_a1.add_definition(RawDefinition {
            def_id: 1,
            kind: DefKind::Fn,
            visibility: 0,
            is_macro_expansion: false,
            name: "func".to_string(),
            path: "ambig::func".to_string(),
            signature: Some("fn func() -> i32".to_string()),
            file: "src/a.rs".to_string(),
            start_line: 50,
            start_col: 1,
            end_line: 52,
            end_col: 2,
        });
        let mut f_a1 = File::create(var_dir.join("ambig1.ridx")).unwrap();
        w_a1.write_to(&mut f_a1).unwrap();
        drop(f_a1);

        // Ambig shard 2
        let mut w_a2 = IndexWriter::new("ambig", "2.0.0");
        w_a2.set_unit_key("ambig_key2");
        w_a2.add_definition(RawDefinition {
            def_id: 1,
            kind: DefKind::Fn,
            visibility: 0,
            is_macro_expansion: false,
            name: "func".to_string(),
            path: "ambig::func".to_string(),
            signature: Some("fn func() -> i64".to_string()),
            file: "src/b.rs".to_string(),
            start_line: 50,
            start_col: 1,
            end_line: 52,
            end_col: 2,
        });
        w_a2.add_definition(RawDefinition {
            def_id: 2,
            kind: DefKind::Struct,
            visibility: 0,
            is_macro_expansion: false,
            name: "NoSig".to_string(),
            path: "ambig::NoSig".to_string(),
            signature: None,
            file: "src/b.rs".to_string(),
            start_line: 60,
            start_col: 1,
            end_line: 62,
            end_col: 2,
        });
        let mut f_a2 = File::create(var_dir.join("ambig2.ridx")).unwrap();
        w_a2.write_to(&mut f_a2).unwrap();
        drop(f_a2);

        // Dep shard with source_root
        let mut w_dep = IndexWriter::new("dep_crate", "0.1.0");
        w_dep.set_unit_key("dep_key");
        w_dep.add_type(RawType {
            file: "src/lib.rs".to_string(),
            start_line: 1,
            start_col: 1,
            end_line: 1,
            end_col: 10,
            type_name: "DepType".to_string(),
        });
        let dep_file = test_dir.0.join("dep.ridx");
        let mut f_dep = File::create(&dep_file).unwrap();
        w_dep.write_to(&mut f_dep).unwrap();
        drop(f_dep);

        // Write variant manifest linking dep_crate with source_root
        let manifest_data = serde_json::json!({
            "version": "2.0",
            "format_version": crate::CURRENT_FORMAT_VERSION,
            "extractor_version": crate::EXTRACTOR_VERSION,
            "active_variant_id": "var_nav",
            "variant_dir": "variants/var_nav",
            "workspace_root": test_dir.0.display().to_string(),
            "shards": [
                { "name": "caller", "version": "0.1.0", "file": "caller.ridx", "is_dependency": false },
                { "name": "ambig", "version": "1.0.0", "file": "ambig1.ridx", "is_dependency": false },
                { "name": "ambig", "version": "2.0.0", "file": "ambig2.ridx", "is_dependency": false }
            ],
            "dependency_shards": [
                {
                    "name": "dep_crate",
                    "version": "0.1.0",
                    "unit_key": "dep_key",
                    "object_path": dep_file.display().to_string(),
                    "source_root": dep_root.display().to_string(),
                    "is_dependency": true,
                    "dependencies": [],
                    "extern_units": {},
                    "file_hashes": {}
                }
            ]
        });
        fs::write(
            &manifest_file,
            serde_json::to_string(&manifest_data).unwrap(),
        )
        .unwrap();

        let ws = WorkspaceIndex::open(&test_dir.0).unwrap();

        // 1. Fallback unresolvable cross-crate definition at cursor (lines 181-192)
        let unres_def = ws.find_definition_at("src/main.rs", 15, 8);
        assert!(unres_def.is_some());
        let ud = unres_def.unwrap();
        assert_eq!(ud.path, "missing_dep::missing_fn");
        assert_eq!(ud.unit_key, Some("caller_key"));

        // 2. Ambiguous candidate resolution matching line 50 (lines 293-298)
        let ambig_def = ws.find_definition_at("src/main.rs", 30, 8);
        assert!(ambig_def.is_some());

        // 2b. Line and file match (lines 289-291)
        let exact_def = ws.find_definition_at("src/main.rs", 35, 8);
        assert!(exact_def.is_some());
        assert_eq!(exact_def.unwrap().file, "src/a.rs");

        // 2c. Ambiguous candidate without line match (line 298)
        let no_line_match_def = ws.find_definition_at("src/main.rs", 40, 8);
        assert!(no_line_match_def.is_some());

        // 2d. Target unit mismatch returns None from find_cross_definition, falling back to raw ref (lines 300-302)
        let unit_mismatch_def = ws.find_definition_at("src/main.rs", 45, 8);
        assert!(unit_mismatch_def.is_some());
        assert_eq!(unit_mismatch_def.unwrap().path, "ambig::nonexistent_symbol");

        // 2e. Definition with signature None (line 65)
        let nosig = ws.find_definition_by_path("ambig::NoSig");
        assert!(nosig.is_some());
        assert_eq!(nosig.unwrap().signature, None);

        // 3. type_at with absolute path under dep source root (lines 625-627)
        let abs_dep_path = dep_root.join("src/lib.rs");
        let ty_dep = ws.type_at(abs_dep_path.to_str().unwrap(), 1, 5);
        assert_eq!(ty_dep, Some("DepType"));

        // 4. type_at with absolute path outside dep root (line 629, 638, 644)
        let abs_other = test_dir.0.join("other/src/lib.rs");
        let ty_other = ws.type_at(abs_other.to_str().unwrap(), 1, 5);
        assert_eq!(ty_other, None);

        // 5. type_at with relative path (line 632)
        let ty_caller = ws.type_at("src/main.rs", 15, 8);
        assert_eq!(ty_caller, Some("MyType"));

        // 6. find_references_for_unit with coordinate matching (lines 504-528)
        let refs1 = ws.find_references_for_unit("ambig::func", Some("ambig_key1"));
        assert_eq!(refs1.len(), 1);
        assert_eq!(refs1[0].span.start_line(), 35);

        // 7. resolve_path_for_crate with root_dir and None workspace_root (lines 111-115)
        let mut ws_no_ws_root = ws;
        ws_no_ws_root.workspace_root = None;
        ws_no_ws_root.root_dir = Some(test_dir.0.clone());
        let resolved = ws_no_ws_root.resolve_path_for_crate("unknown", "src/lib.rs");
        assert_eq!(resolved, test_dir.0.join("src/lib.rs"));

        ws_no_ws_root.root_dir = None;
        let resolved_plain = ws_no_ws_root.resolve_path_for_crate("unknown", "src/lib.rs");
        assert_eq!(resolved_plain, PathBuf::from("src/lib.rs"));
    }

    #[test]
    fn test_navigation_references_and_candidate_resolution() {
        let test_dir = new_test_dir("ws_nav_refs_test");
        let target_file1 = test_dir.0.join("t1.ridx");
        let target_file2 = test_dir.0.join("t2.ridx");
        let consumer_file = test_dir.0.join("cons.ridx");

        // Target unit 1
        let mut w1 = IndexWriter::new("target_krate", "0.1.0");
        w1.set_unit_key("unit_1");
        w1.set_generation("gen_1");
        w1.add_definition(RawDefinition {
            def_id: 1,
            kind: DefKind::Fn,
            visibility: 0,
            is_macro_expansion: false,
            name: "target_fn".to_string(),
            path: "target_krate::target_fn".to_string(),
            signature: Some("fn target_fn()".to_string()),
            file: "src/target.rs".to_string(),
            start_line: 10,
            start_col: 1,
            end_line: 10,
            end_col: 20,
        });
        let mut f1 = File::create(&target_file1).unwrap();
        w1.write_to(&mut f1).unwrap();
        drop(f1);

        // Target unit 2 (same crate, different unit)
        let mut w2 = IndexWriter::new("target_krate", "0.2.0");
        w2.set_unit_key("unit_2");
        w2.set_generation("gen_2");
        w2.add_definition(RawDefinition {
            def_id: 1,
            kind: DefKind::Fn,
            visibility: 0,
            is_macro_expansion: false,
            name: "target_fn".to_string(),
            path: "target_krate::target_fn".to_string(),
            signature: Some("fn target_fn()".to_string()),
            file: "src/target_alt.rs".to_string(),
            start_line: 10,
            start_col: 1,
            end_line: 10,
            end_col: 20,
        });
        // Same crate also has an internal reference to target_fn
        w2.add_reference(RawReference {
            file: "src/target_alt.rs".to_string(),
            start_line: 25,
            start_col: 5,
            end_line: 25,
            end_col: 15,
            target_def_id: Some(1),
            target_path: "target_krate::target_fn".to_string(),
            target_unit: None,
            target_file: Some("src/target_alt.rs".to_string()),
            target_line: 10,
            target_col: 0,
            kind: DefKind::Fn,
            is_write: false,
            is_macro_expansion: false,
        });
        let mut f2 = File::create(&target_file2).unwrap();
        w2.write_to(&mut f2).unwrap();
        drop(f2);

        // Consumer crate referencing target_krate directly and via alias
        let mut wc = IndexWriter::new("consumer_krate", "0.1.0");
        wc.set_unit_key("unit_consumer");
        wc.set_generation("gen_consumer");
        // Ref with mismatched file (lines 519-525)
        wc.add_reference(RawReference {
            file: "src/main.rs".to_string(),
            start_line: 30,
            start_col: 1,
            end_line: 30,
            end_col: 10,
            target_def_id: None,
            target_path: "target_krate::target_fn".to_string(),
            target_unit: None,
            target_file: Some("mismatched_file.rs".to_string()),
            target_line: 10,
            target_col: 0,
            kind: DefKind::Fn,
            is_write: false,
            is_macro_expansion: false,
        });
        // Ref via alias with mismatched target unit (lines 562-564)
        wc.add_reference(RawReference {
            file: "src/main.rs".to_string(),
            start_line: 31,
            start_col: 1,
            end_line: 31,
            end_col: 10,
            target_def_id: None,
            target_path: "t_alias::target_fn".to_string(),
            target_unit: Some("wrong_unit".to_string()),
            target_file: Some("src/target.rs".to_string()),
            target_line: 10,
            target_col: 0,
            kind: DefKind::Fn,
            is_write: false,
            is_macro_expansion: false,
        });
        // Ref via alias with mismatched line (lines 571-573)
        wc.add_reference(RawReference {
            file: "src/main.rs".to_string(),
            start_line: 32,
            start_col: 1,
            end_line: 32,
            end_col: 10,
            target_def_id: None,
            target_path: "t_alias::target_fn".to_string(),
            target_unit: None,
            target_file: Some("src/target.rs".to_string()),
            target_line: 99,
            target_col: 0,
            kind: DefKind::Fn,
            is_write: false,
            is_macro_expansion: false,
        });
        // Ref via alias with mismatched file (lines 581-585)
        wc.add_reference(RawReference {
            file: "src/main.rs".to_string(),
            start_line: 33,
            start_col: 1,
            end_line: 33,
            end_col: 10,
            target_def_id: None,
            target_path: "t_alias::target_fn".to_string(),
            target_unit: None,
            target_file: Some("unrelated.rs".to_string()),
            target_line: 10,
            target_col: 0,
            kind: DefKind::Fn,
            is_write: false,
            is_macro_expansion: false,
        });
        // Ref via alias that matches! (lines 587-603)
        wc.add_reference(RawReference {
            file: "src/main.rs".to_string(),
            start_line: 34,
            start_col: 1,
            end_line: 34,
            end_col: 10,
            target_def_id: None,
            target_path: "t_alias::target_fn".to_string(),
            target_unit: None,
            target_file: Some("src/target.rs".to_string()),
            target_line: 10,
            target_col: 0,
            kind: DefKind::Fn,
            is_write: false,
            is_macro_expansion: false,
        });

        // Ambiguous ref: matching line and file ends_with target_alt.rs (lines 288-291)
        wc.add_reference(RawReference {
            file: "src/main.rs".to_string(),
            start_line: 35,
            start_col: 1,
            end_line: 35,
            end_col: 10,
            target_def_id: None,
            target_path: "target_krate::target_fn".to_string(),
            target_unit: None,
            target_file: Some("src/target_alt.rs".to_string()),
            target_line: 10,
            target_col: 0,
            kind: DefKind::Fn,
            is_write: false,
            is_macro_expansion: false,
        });
        // Ambiguous ref: matching line only (lines 293-296)
        wc.add_reference(RawReference {
            file: "src/main.rs".to_string(),
            start_line: 36,
            start_col: 1,
            end_line: 36,
            end_col: 10,
            target_def_id: None,
            target_path: "target_krate::target_fn".to_string(),
            target_unit: None,
            target_file: Some("unmatched.rs".to_string()),
            target_line: 10,
            target_col: 0,
            kind: DefKind::Fn,
            is_write: false,
            is_macro_expansion: false,
        });
        // Ambiguous ref: neither line nor file matching, first candidate fallback (line 298)
        wc.add_reference(RawReference {
            file: "src/main.rs".to_string(),
            start_line: 37,
            start_col: 1,
            end_line: 37,
            end_col: 10,
            target_def_id: None,
            target_path: "target_krate::target_fn".to_string(),
            target_unit: None,
            target_file: Some("unmatched.rs".to_string()),
            target_line: 999,
            target_col: 0,
            kind: DefKind::Fn,
            is_write: false,
            is_macro_expansion: false,
        });
        // Target unit found but symbol does not exist -> target unit fallback (lines 300-302)
        wc.add_reference(RawReference {
            file: "src/main.rs".to_string(),
            start_line: 38,
            start_col: 1,
            end_line: 38,
            end_col: 10,
            target_def_id: None,
            target_path: "t_alias::nonexistent_symbol".to_string(),
            target_unit: Some("unit_1".to_string()),
            target_file: Some("src/target.rs".to_string()),
            target_line: 10,
            target_col: 0,
            kind: DefKind::Fn,
            is_write: false,
            is_macro_expansion: false,
        });
        // Target def file mismatch in direct references loop (lines 520-527)
        wc.add_reference(RawReference {
            file: "src/main.rs".to_string(),
            start_line: 45,
            start_col: 1,
            end_line: 45,
            end_col: 10,
            target_def_id: None,
            target_path: "target_krate::target_fn".to_string(),
            target_unit: None,
            target_file: Some("src/mismatched_file.rs".to_string()),
            target_line: 10,
            target_col: 0,
            kind: DefKind::Fn,
            is_write: false,
            is_macro_expansion: false,
        });
        // Target def file mismatch in alias references loop (lines 580-590)
        wc.add_reference(RawReference {
            file: "src/main.rs".to_string(),
            start_line: 46,
            start_col: 1,
            end_line: 46,
            end_col: 10,
            target_def_id: None,
            target_path: "t_alias::target_fn".to_string(),
            target_unit: None,
            target_file: Some("src/mismatched_file.rs".to_string()),
            target_line: 10,
            target_col: 0,
            kind: DefKind::Fn,
            is_write: false,
            is_macro_expansion: false,
        });
        // Definition with signature: None (line 65)
        wc.add_definition(RawDefinition {
            def_id: 1,
            kind: DefKind::Mod,
            visibility: 0,
            is_macro_expansion: false,
            name: "no_sig_mod".to_string(),
            path: "consumer_krate::no_sig_mod".to_string(),
            signature: None,
            file: "src/main.rs".to_string(),
            start_line: 1,
            start_col: 1,
            end_line: 5,
            end_col: 1,
        });

        let mut fc = File::create(&consumer_file).unwrap();
        wc.write_to(&mut fc).unwrap();
        drop(fc);

        let mut ws = WorkspaceIndex::open(&test_dir.0).unwrap();
        // Setup extern mapping for consumer: t_alias -> unit_1
        let cons_idx = ws
            .readers
            .iter()
            .position(|r| r.crate_name() == "consumer_krate")
            .unwrap();
        let mut ext_map = HashMap::new();
        ext_map.insert("t_alias".to_string(), "unit_1".to_string());
        ws.reader_extern_units[cons_idx] = ext_map;
        ws.reader_dependencies[cons_idx] = vec!["unit_1".to_string(), "unit_2".to_string()]; // hits lines 224-235

        let t2_idx = ws
            .readers
            .iter()
            .position(|r| r.unit_key() == "unit_2")
            .unwrap();
        ws.reader_dependencies[t2_idx] = vec!["unit_1".to_string()]; // hits lines 499-503

        // find_definition_by_path with signature None (line 65)
        let def_no_sig = ws
            .find_definition_by_path("consumer_krate::no_sig_mod")
            .unwrap();
        assert_eq!(def_no_sig.signature, None);

        // Query references to target_krate::target_fn in unit_1
        // Reader for target unit 2 has reader_crate == target_crate but ru != target_unit (lines 499-503)
        let refs = ws.find_references_for_unit("target_krate::target_fn", Some("unit_1"));
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].span.start_line(), 34);

        // Hover with source_root set (lines 407-409)
        let t1_idx = ws
            .readers
            .iter()
            .position(|r| r.unit_key() == "unit_1")
            .unwrap();
        ws.source_roots[t1_idx] = Some(test_dir.0.clone());
        let hover = ws.hover_at("src/target.rs", 10, 5);
        assert!(hover.is_some());
        assert!(hover.unwrap().definition.is_some());

        // Target path starting with target_crate:: and mapping to r.crate_name() (lines 407-419)
        let mapped_refs = ws.find_references_for_unit("t_alias::target_fn", Some("unit_1"));
        assert!(!mapped_refs.is_empty());

        // Target def coords with no source root and no workspace root (line 396, 415)
        ws.source_roots[t1_idx] = None;
        let orig_ws_root = ws.workspace_root.take();
        let no_root_refs = ws.find_references_for_unit("target_krate::target_fn", Some("unit_1"));
        assert_eq!(no_root_refs.len(), 1);
        ws.workspace_root = orig_ws_root;

        // Query ambiguous references (lines 282-298)
        let def_alt = ws.find_definition_at("src/main.rs", 35, 5).unwrap();
        assert_eq!(def_alt.file, "src/target_alt.rs");

        let def_line_match = ws.find_definition_at("src/main.rs", 36, 5).unwrap();
        assert_eq!(def_line_match.start_line, 10);

        let def_fallback = ws.find_definition_at("src/main.rs", 37, 5).unwrap();
        assert_eq!(def_fallback.name, "target_fn");

        // Target unit not found fallback (lines 300-302)
        let def_unresolved = ws.find_definition_at("src/main.rs", 38, 5).unwrap();
        assert_eq!(def_unresolved.name, "nonexistent_symbol");

        // Definition with signature None (line 65)
        let d_no_sig = ws
            .find_definition_by_path("consumer_krate::no_sig_mod")
            .unwrap();
        assert_eq!(d_no_sig.signature, None);

        // find_references_for_unit with target_unit_key None to trigger target_def_coords filtering (lines 504-528, 565-590)
        let coord_refs = ws.find_references_for_unit("target_krate::target_fn", None);
        assert!(!coord_refs.is_empty());

        // Target unit filtering in extern_map and reader_unit != target_unit (lines 500-502, 555)
        let refs_wrong_unit =
            ws.find_references_for_unit("target_krate::target_fn", Some("nonexistent_unit"));
        assert!(refs_wrong_unit.is_empty());
    }
}
