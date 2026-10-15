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
use crate::reader::{IndexReader, ResolvedDefinition};
use std::path::{Path, PathBuf};

impl WorkspaceIndex {
    pub(crate) fn make_resolved_definition<'a>(
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
            is_reexport: def.is_reexport(),
            source_root: if self
                .reader_is_dependency
                .get(reader_idx)
                .copied()
                .unwrap_or(false)
            {
                self.source_roots.get(reader_idx).and_then(|r| r.as_deref())
            } else {
                self.source_roots
                    .get(reader_idx)
                    .and_then(|r| r.as_deref())
                    .or(self.workspace_root.as_deref())
            },
            unit_key: self.unit_keys.get(reader_idx).and_then(|u| u.as_deref()),
            reader_idx: Some(reader_idx),
        }
    }

    /// Finds a definition by full or relative canonical path across the workspace.
    pub fn find_definition_by_path<'a>(&'a self, path: &str) -> Option<ResolvedDefinition<'a>> {
        let resolve_def = |res: ResolvedDefinition<'a>| -> ResolvedDefinition<'a> {
            if res.kind == DefKind::Use {
                if let Some(sig) = res.signature {
                    if let Some(target) = self.find_definition_by_path(sig) {
                        if target.kind != DefKind::Use && target.kind != DefKind::Local {
                            return target;
                        }
                    }
                }
                if res.start_line > 1 || res.start_col > 1 {
                    if let Some(target) =
                        self.find_definition_at(res.file, res.start_line, res.start_col)
                    {
                        if target.kind != DefKind::Use && target.kind != DefKind::Local {
                            return target;
                        }
                    }
                }
            }
            res
        };

        for candidate in super::std_map::std_path_candidates(path) {
            let candidate_str = candidate.as_str();

            // Fast path: if path starts with `<crate_name>::` or equals `<crate_name>`, check that crate first
            for (i, reader) in self.readers.iter().enumerate() {
                let prefix = format!("{}::", reader.crate_name());
                if candidate_str.starts_with(&prefix) || candidate_str == reader.crate_name() {
                    let sub = candidate_str.strip_prefix(&prefix).unwrap_or(candidate_str);
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

        // Fallback: candidate may be `owner::item_name` where item is registered under owner
        for candidate in super::std_map::std_path_candidates(path) {
            let candidate_str = candidate.as_str();
            if let Some((owner, item_name)) = candidate_str.rsplit_once("::") {
                for (i, reader) in self.readers.iter().enumerate() {
                    let prefix = format!("{}::", reader.crate_name());
                    let sub_owner = owner.strip_prefix(&prefix).unwrap_or(owner);
                    for def in reader.find_definitions_by_owner(owner) {
                        if reader.get_str(def.name_offset()) == item_name {
                            return Some(resolve_def(self.make_resolved_definition(i, def)));
                        }
                    }
                    if sub_owner != owner {
                        for def in reader.find_definitions_by_owner(sub_owner) {
                            if reader.get_str(def.name_offset()) == item_name {
                                return Some(resolve_def(self.make_resolved_definition(i, def)));
                            }
                        }
                    }
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
        self.reader_index_for_file(file_path)
            .and_then(|i| self.readers.get(i).map(|r| r.crate_name()))
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
        let mut fallback_mod = None;
        for i in self.candidate_reader_indices_for_file(file) {
            let reader = &self.readers[i];
            let Some(target_file) = self.target_file_for_reader(i, file) else {
                continue;
            };

            if let Some(mut res) = reader.find_definition_at(target_file.as_ref(), line, col) {
                if res.kind == DefKind::Mod && (res.start_line != line || col < res.start_col) {
                    if fallback_mod.is_none() {
                        fallback_mod = Some(res);
                    }
                    continue;
                }

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
                    res.source_root = if self.reader_is_dependency.get(i).copied().unwrap_or(false)
                    {
                        self.source_roots.get(i).and_then(|r| r.as_deref())
                    } else {
                        self.source_roots
                            .get(i)
                            .and_then(|r| r.as_deref())
                            .or(self.workspace_root.as_deref())
                    };
                    res.unit_key = self.unit_keys.get(i).and_then(|u| u.as_deref());
                    res.reader_idx = Some(i);
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
                    res.source_root = if self.reader_is_dependency.get(i).copied().unwrap_or(false)
                    {
                        self.source_roots.get(i).and_then(|r| r.as_deref())
                    } else {
                        self.source_roots
                            .get(i)
                            .and_then(|r| r.as_deref())
                            .or(self.workspace_root.as_deref())
                    };
                }
                if res.reader_idx.is_none() {
                    res.reader_idx = Some(i);
                }
                return Some(res);
            }
        }

        fallback_mod
    }

    /// Finds a generic type parameter (`DefKind::TyParam`) in scope at `(file, line, col)`.
    pub fn find_type_param_in_scope<'a>(
        &'a self,
        file: &str,
        name: &str,
        line: u32,
        col: u32,
    ) -> Option<ResolvedDefinition<'a>> {
        for i in self.candidate_reader_indices_for_file(file) {
            let reader = &self.readers[i];
            let Some(target_file) = self.target_file_for_reader(i, file) else {
                continue;
            };

            if let Some((def, _)) =
                reader.find_type_param_in_scope(target_file.as_ref(), name, line, col)
            {
                return Some(self.make_resolved_definition(i, def));
            }
        }
        None
    }

    /// Finds all owners/bounds associated with a definition.
    pub fn find_owners_of_definition<'a>(&'a self, def: &ResolvedDefinition<'a>) -> Vec<&'a str> {
        for i in self.candidate_reader_indices_for_file(def.file) {
            let reader = &self.readers[i];
            let Some(target_file) = self.target_file_for_reader(i, def.file) else {
                continue;
            };

            if let Some(_fid) = reader.find_file_id(target_file.as_ref()) {
                if let Some(def_entry) = reader.find_definition_by_path(def.path) {
                    let def_idx = (def_entry as *const _ as usize
                        - reader.definitions().as_ptr() as usize)
                        / std::mem::size_of::<DefEntry>();
                    let owners = reader.find_owners_by_def_index(def_idx as u32);
                    if !owners.is_empty() {
                        return owners;
                    }
                }
            }
        }
        Vec::new()
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
                        is_reexport: def.is_reexport(),
                        source_root: if self.reader_is_dependency.get(j).copied().unwrap_or(false) {
                            self.source_roots.get(j).and_then(|r| r.as_deref())
                        } else {
                            self.source_roots
                                .get(j)
                                .and_then(|r| r.as_deref())
                                .or(self.workspace_root.as_deref())
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
                        is_reexport: def.is_reexport(),
                        source_root: if self.reader_is_dependency.get(j).copied().unwrap_or(false) {
                            self.source_roots.get(j).and_then(|r| r.as_deref())
                        } else {
                            self.source_roots
                                .get(j)
                                .and_then(|r| r.as_deref())
                                .or(self.workspace_root.as_deref())
                        },
                        unit_key: self.unit_keys.get(j).and_then(|u| u.as_deref()),
                        reader_idx: Some(j),
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
                                    is_reexport: def.is_reexport(),
                                    source_root: if self
                                        .reader_is_dependency
                                        .get(j)
                                        .copied()
                                        .unwrap_or(false)
                                    {
                                        self.source_roots.get(j).and_then(|s| s.as_deref())
                                    } else {
                                        self.source_roots
                                            .get(j)
                                            .and_then(|s| s.as_deref())
                                            .or(self.workspace_root.as_deref())
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
    #[inline]
    pub(crate) fn matches_unit(
        &self,
        reader_idx: usize,
        ref_target_unit: &str,
        target_unit: &str,
        target_crate: &str,
    ) -> bool {
        self.unit_match.matches_unit(
            reader_idx,
            ref_target_unit,
            target_unit,
            target_crate,
            &self.reader_extern_units,
        )
    }

    #[inline]
    fn matches_unit_memoized<'a>(
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
            } else if is_same_package {
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
                        for r in reader.find_references(&aliased_path) {
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

    /// Finds all references to a symbol across every crate in the workspace.
    pub fn find_references<'a>(&'a self, target_path: &str) -> Vec<WorkspaceReference<'a>> {
        let canonical = self
            .find_definition_by_path(target_path)
            .map(|d| d.path)
            .unwrap_or(target_path);
        self.find_references_for_unit(canonical, None)
    }

    /// Inferred expression type at cursor location.
    pub fn type_at(&self, file: &str, line: u32, col: u32) -> Option<&str> {
        for i in self.candidate_reader_indices_for_file(file) {
            let reader = &self.readers[i];
            let Some(target_file) = self.target_file_for_reader(i, file) else {
                continue;
            };
            if let Some(ty) = reader.type_at(target_file.as_ref(), line, col) {
                return Some(ty);
            }
        }
        None
    }

    /// Inferred expression type and optional canonical owner path at cursor location.
    pub fn type_entry_at(&self, file: &str, line: u32, col: u32) -> Option<(&str, Option<&str>)> {
        for i in self.candidate_reader_indices_for_file(file) {
            let reader = &self.readers[i];
            let Some(target_file) = self.target_file_for_reader(i, file) else {
                continue;
            };
            if let Some(res) = reader.type_entry_at(target_file.as_ref(), line, col) {
                return Some(res);
            }
        }
        None
    }

    /// Finds all definitions belonging to an owner (canonical type path or module path) across the workspace.
    pub fn find_definitions_by_owner<'a>(&'a self, owner: &str) -> Vec<ResolvedDefinition<'a>> {
        self.find_definitions_by_owner_internal(owner, None)
    }

    /// Finds all definitions belonging to an owner within the scope accessible from `file`.
    ///
    /// Non-library targets (benchmarks, tests, examples, binaries) that do not contain `file`
    /// are excluded from the search to prevent symbol leakage into standard library and crate queries.
    pub fn find_definitions_by_owner_for_file<'a>(
        &'a self,
        file: &str,
        owner: &str,
    ) -> Vec<ResolvedDefinition<'a>> {
        self.find_definitions_by_owner_internal(owner, Some(file))
    }

    fn find_definitions_by_owner_internal<'a>(
        &'a self,
        owner: &str,
        file_opt: Option<&str>,
    ) -> Vec<ResolvedDefinition<'a>> {
        let mut results = Vec::new();
        let mut seen = std::collections::HashSet::new();

        let reader_indices: Vec<usize> = match file_opt {
            Some(file) => self.candidate_reader_indices_for_file(file),
            None => (0..self.readers.len()).collect(),
        };

        let candidates = super::std_map::std_path_candidates(owner);
        for cand in &candidates {
            let cand_str = cand.as_str();
            let prev_len = results.len();
            for &i in &reader_indices {
                let reader = &self.readers[i];
                let prefix = format!("{}::", reader.crate_name());
                let sub = cand_str.strip_prefix(&prefix).unwrap_or(cand_str);
                let with_prefix = format!("{prefix}{cand_str}");
                let is_sysroot = matches!(reader.crate_name(), "core" | "alloc" | "std");
                let allow_with_prefix = !is_sysroot
                    || cand_str.contains("::")
                    || cand_str.starts_with("core")
                    || cand_str.starts_with("std")
                    || cand_str.starts_with("alloc");

                let prefix_defs = if allow_with_prefix {
                    Some(reader.find_definitions_by_owner(&with_prefix))
                } else {
                    None
                };

                for def in reader
                    .find_definitions_by_owner(cand_str)
                    .into_iter()
                    .chain(reader.find_definitions_by_owner(sub))
                    .chain(prefix_defs.into_iter().flatten())
                {
                    let key = (
                        i,
                        def.path_offset(),
                        def.span.start_line,
                        def.span.start_col,
                    );
                    if seen.insert(key) {
                        results.push(self.make_resolved_definition(i, def));
                    }
                }
            }
            if cand_str == owner && results.len() > prev_len {
                break;
            }
        }

        results
    }

    /// Resolves the autoderef chain starting from `source` up to `max_depth` steps across the workspace.
    /// Handles multiple supertraits across workspace readers via breadth-first search.
    pub fn resolve_deref_chain(&self, source: &str, max_depth: usize) -> Vec<&str> {
        let mut chain = Vec::new();
        let mut queue = std::collections::VecDeque::new();
        let mut visited = std::collections::HashSet::new();
        visited.insert(source);
        queue.push_back((source, 0));

        while let Some((current, depth)) = queue.pop_front() {
            if depth >= max_depth {
                continue;
            }
            for reader in &self.readers {
                for target in reader.find_deref_targets(current) {
                    if !target.is_empty() && visited.insert(target) {
                        chain.push(target);
                        queue.push_back((target, depth + 1));
                    }
                }
            }
        }

        chain
    }

    /// Substring symbol search across all definitions in the workspace.
    pub fn search_symbols<'a>(&'a self, query: &str) -> Vec<WorkspaceSymbol<'a>> {
        self.search_symbols_filtered(query, |_, _| true)
    }

    /// Substring symbol search across non-dependency definitions in the workspace.
    pub fn search_local_symbols<'a>(&'a self, query: &str) -> Vec<WorkspaceSymbol<'a>> {
        self.search_symbols_filtered(query, |i, _| {
            !self.reader_is_dependency.get(i).copied().unwrap_or(false)
        })
    }

    /// Substring symbol search with custom reader predicate.
    pub fn search_symbols_filtered<'a, F>(
        &'a self,
        query: &str,
        mut filter: F,
    ) -> Vec<WorkspaceSymbol<'a>>
    where
        F: FnMut(usize, &IndexReader<memmap2::Mmap>) -> bool,
    {
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
                if !filter(i, reader) {
                    continue;
                }
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

    /// Finds the first symbol matching `query` and `predicate` without eagerly collecting all matches.
    pub fn find_first_symbol<'a, F>(
        &'a self,
        query: &str,
        mut predicate: F,
    ) -> Option<WorkspaceSymbol<'a>>
    where
        F: FnMut(&WorkspaceSymbol<'a>) -> bool,
    {
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
                    let signature = if def.sig_offset() != 0 {
                        Some(reader.get_str(def.sig_offset()))
                    } else {
                        None
                    };

                    let sym = WorkspaceSymbol {
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
                    };
                    if predicate(&sym) {
                        return Some(sym);
                    }
                }
            }
        }
        None
    }

    /// Iterates over matching definitions across workspace readers without eager allocation,
    /// searching preferred crate first and stopping when `cb` returns false or scan budget is exhausted.
    pub fn for_each_symbol_ascii<F>(
        &self,
        needle_lower: &[u8],
        preferred_crate: Option<&str>,
        mut cb: F,
    ) -> bool
    where
        F: FnMut(&IndexReader<memmap2::Mmap>, &DefEntry) -> bool,
    {
        let mut budget: usize = 2_000;
        let mut record_budget: usize = 30_000;
        if let Some(pref) = preferred_crate {
            for reader in &self.readers {
                if reader.crate_name() == pref
                    && !reader.for_each_symbol_ascii_budgeted(
                        needle_lower,
                        &mut budget,
                        &mut record_budget,
                        |def| cb(reader, def),
                    )
                {
                    return false;
                }
            }
        }
        for reader in &self.readers {
            if preferred_crate.is_some_and(|pref| reader.crate_name() == pref) {
                continue;
            }
            if !reader.for_each_symbol_ascii_budgeted(
                needle_lower,
                &mut budget,
                &mut record_budget,
                |def| cb(reader, def),
            ) {
                return false;
            }
        }
        true
    }

    /// Iterates over matching definitions specifically for a single target crate without eager allocation.
    pub fn for_each_symbol_in_crate<F>(
        &self,
        target_crate: &str,
        needle_lower: &[u8],
        mut cb: F,
    ) -> bool
    where
        F: FnMut(&IndexReader<memmap2::Mmap>, &DefEntry) -> bool,
    {
        let mut budget: usize = 2_000;
        let mut record_budget: usize = 30_000;
        for reader in &self.readers {
            if reader.crate_name() == target_crate
                && !reader.for_each_symbol_ascii_budgeted(
                    needle_lower,
                    &mut budget,
                    &mut record_budget,
                    |def| cb(reader, def),
                )
            {
                return false;
            }
        }
        true
    }

    /// Returns true if any index reader in the workspace has the given crate name.
    pub fn has_crate(&self, target_crate: &str) -> bool {
        self.readers.iter().any(|r| r.crate_name() == target_crate)
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
            target_reader_idx: def.reader_idx,
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
#[cfg(test)]
mod tests;
