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

mod cross_definition;
mod owner_types;
mod references;
mod search;
#[cfg(test)]
mod tests;

use super::manifest::{LocationReferences, WorkspaceHover, WorkspaceReference, WorkspaceSymbol};
use super::std_map;
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
}
