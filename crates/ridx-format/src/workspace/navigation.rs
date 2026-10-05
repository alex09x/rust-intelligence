//! Cross-crate code navigation, symbol search, hover, and reference tracking.

use super::manifest::{LocationReferences, WorkspaceHover, WorkspaceReference, WorkspaceSymbol};
use super::WorkspaceIndex;
use crate::format::DefKind;
use crate::reader::ResolvedDefinition;
use std::path::{Path, PathBuf};

impl WorkspaceIndex {
    /// Finds a definition by full or relative canonical path across the workspace.
    pub fn find_definition_by_path<'a>(&'a self, path: &str) -> Option<ResolvedDefinition<'a>> {
        // Fast path: if path starts with `<crate_name>::`, check that crate first
        for (i, reader) in self.readers.iter().enumerate() {
            let prefix = format!("{}::", reader.crate_name());
            if path.starts_with(&prefix) {
                if let Some(def) = reader.find_definition_by_path(path) {
                    let span = def.span();
                    return Some(ResolvedDefinition {
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
                            .get(i)
                            .and_then(|r| r.as_deref())
                            .or(self.workspace_root.as_deref()),
                        unit_key: self.unit_keys.get(i).and_then(|u| u.as_deref()),
                    });
                }
            }
        }

        // General search across all crates
        for (i, reader) in self.readers.iter().enumerate() {
            if let Some(def) = reader.find_definition_by_path(path) {
                let span = def.span();
                return Some(ResolvedDefinition {
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
                        .get(i)
                        .and_then(|r| r.as_deref())
                        .or(self.workspace_root.as_deref()),
                    unit_key: self.unit_keys.get(i).and_then(|u| u.as_deref()),
                });
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
                if is_abs {
                    // Workspace member reader: skip if file belongs to any external dependency's source_root
                    let belongs_to_dep = self.source_roots.iter().any(|r| {
                        if let Some(r) = r.as_ref() {
                            p.starts_with(r)
                        } else {
                            false
                        }
                    });
                    if belongs_to_dep {
                        continue;
                    }
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
                        signature: if def.sig_offset() != 0 {
                            Some(r.get_str(def.sig_offset()))
                        } else {
                            None
                        },
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
            if target_unit.is_some() {
                return None;
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
                        signature: if def.sig_offset() != 0 {
                            Some(r.get_str(def.sig_offset()))
                        } else {
                            None
                        },
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

    /// Finds all references to a symbol, optionally scoped to a target dependency unit.
    pub fn find_references_for_unit<'a>(
        &'a self,
        target_path: &str,
        target_unit_key: Option<&str>,
    ) -> Vec<WorkspaceReference<'a>> {
        let mut all_refs = Vec::new();
        let target_crate = target_path.split("::").next().unwrap_or(target_path);

        // If target_unit_key is given, locate the definition in target_unit to get its line and file
        let target_def_coords = if let Some(target_unit) = target_unit_key {
            let mut coords = None;
            for (j, r) in self.readers.iter().enumerate() {
                if self.unit_keys.get(j).and_then(|u| u.as_deref()) == Some(target_unit) {
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
                    if let Some(rest) = target_path.strip_prefix(&format!("{}::", r.crate_name())) {
                        let mapped = format!("{}::{}", r.crate_name(), rest);
                        if let Some(def) = r.find_definition_by_path(&mapped) {
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
                    }
                }
            }
            coords
        } else {
            None
        };

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
                            .map(|d| d.iter().any(|dep| dep == target_unit))
                            .unwrap_or(false);
                        let in_extern_units = self
                            .reader_extern_units
                            .get(i)
                            .map(|m| m.values().any(|uk| uk == target_unit))
                            .unwrap_or(false);
                        if !in_deps && !in_extern_units {
                            // Referring crate has dependency info but does NOT depend on target_unit. Exclude!
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

            // References with canonical target_path
            let refs = reader.find_references(target_path);
            let source_root = self
                .source_roots
                .get(i)
                .and_then(|r| r.as_deref())
                .or(self.workspace_root.as_deref());

            for r in refs {
                let r_target_unit = reader.target_unit(r);
                if let Some(target_unit) = target_unit_key {
                    if !r_target_unit.is_empty() {
                        if r_target_unit != target_unit {
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

            // Also check if reader refers to target_path via an extern alias (e.g. `a1::foo` instead of `a::foo`)
            if let Some(extern_map) = self.reader_extern_units.get(i) {
                for (alias, uk) in extern_map {
                    if alias != target_crate
                        && target_path.starts_with(&format!("{target_crate}::"))
                    {
                        if let Some(target_unit) = target_unit_key {
                            if uk != target_unit {
                                continue;
                            }
                        }
                        let aliased_path =
                            format!("{alias}::{}", &target_path[target_crate.len() + 2..]);
                        for r in reader.find_references(&aliased_path) {
                            let r_target_unit = reader.target_unit(r);
                            if let Some(target_unit) = target_unit_key {
                                if !r_target_unit.is_empty() {
                                    if r_target_unit != target_unit {
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
                if is_abs {
                    let belongs_to_dep = self.source_roots.iter().any(|r| {
                        if let Some(r) = r.as_ref() {
                            p.starts_with(r)
                        } else {
                            false
                        }
                    });
                    if belongs_to_dep {
                        continue;
                    }
                }
                file
            };
            if let Some(ty) = reader.type_at(target_file, line, col) {
                return Some(ty);
            }
        }
        None
    }

    /// Substring symbol search across all definitions in the workspace.
    pub fn search_symbols<'a>(&'a self, query: &str) -> Vec<WorkspaceSymbol<'a>> {
        let mut matches = Vec::new();

        for (i, reader) in self.readers.iter().enumerate() {
            let crate_name = reader.crate_name();
            let defs = reader.search_symbols(query);
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
