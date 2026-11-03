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

pub mod blast_radius;
mod cross_definition;
pub(crate) mod decl_lookup;
mod exact_symbols;
mod impl_lookup;
pub(crate) mod impl_parse;
mod owner_types;
pub(crate) mod qualified_path;
mod reference_record;
mod references;
pub(crate) mod rename_check;
mod rename_macro_ext;
mod rename_pm_enum;
mod rename_refs_ext;
mod search;
#[cfg(test)]
mod tests;
pub mod trait_impl_refs;
mod trait_rename_ext;
mod workspace_symbol;

pub use blast_radius::{BlastRadiusIndex, DirectBlastRadius};

use super::manifest::{LocationReferences, WorkspaceHover, WorkspaceReference, WorkspaceSymbol};
use super::std_map;
use super::WorkspaceIndex;
use crate::format::{DefEntry, DefKind};
use crate::reader::{IndexReader, ResolvedDefinition};
use std::path::{Path, PathBuf};

#[inline]
pub(crate) fn strip_crate_prefix<'a>(s: &'a str, crate_name: &str) -> Option<&'a str> {
    s.strip_prefix(crate_name)?.strip_prefix("::")
}

impl WorkspaceIndex {
    pub(crate) fn make_resolved_definition<'a>(&'a self, reader_idx: usize, def: &DefEntry) -> ResolvedDefinition<'a> {
        let reader = &self.readers[reader_idx];
        let span = def.span();
        ResolvedDefinition {
            name: reader.get_str(def.name_offset()),
            path: reader.get_str(def.path_offset()),
            kind: DefKind::from(def.kind),
            signature: if def.sig_offset() != 0 { Some(reader.get_str(def.sig_offset())) } else { None },
            file: reader.get_file(span.file_id()),
            start_line: span.start_line(),
            start_col: span.start_col(),
            end_line: span.end_line(),
            end_col: span.end_col(),
            is_macro_expansion: def.is_macro_expansion(),
            is_reexport: def.is_reexport(),
            source_root: if self.reader_is_dependency.get(reader_idx).copied().unwrap_or(false) {
                self.source_roots.get(reader_idx).and_then(|r| r.as_deref())
            } else {
                self.source_roots.get(reader_idx).and_then(|r| r.as_deref()).or(self.workspace_root.as_deref())
            },
            unit_key: self.unit_keys.get(reader_idx).and_then(|u| u.as_deref()),
            reader_idx: Some(reader_idx),
        }
    }

    /// Finds a definition by full or relative canonical path across the workspace.
    pub fn find_definition_by_path<'a>(&'a self, path: &str) -> Option<ResolvedDefinition<'a>> {
        self.find_definition_by_path_depth(path, 0)
    }

    pub(crate) fn find_definition_by_path_depth<'a>(&'a self, path: &str, depth: u8) -> Option<ResolvedDefinition<'a>> {
        if depth > 4 {
            return None;
        }
        let resolve_def = |res: ResolvedDefinition<'a>| -> ResolvedDefinition<'a> {
            if res.kind == DefKind::Use && depth < 4 {
                if let Some(sig) = res.signature {
                    let sig_clean = sig.trim_end_matches("::*");
                    let rel_sig = sig_clean.strip_prefix("self::").unwrap_or(sig_clean);
                    if let Some((owner_mod, _)) = res.path.rsplit_once("::") {
                        if let Some(target) = self.find_definition_by_path_depth(&format!("{owner_mod}::{rel_sig}"), depth + 1) {
                            if target.kind != DefKind::Use && target.kind != DefKind::Local {
                                return target;
                            }
                        }
                    }
                    if let Some(r_idx) = res.reader_idx {
                        let cname = self.readers[r_idx].crate_name();
                        if !rel_sig.starts_with(&format!("{cname}::")) {
                            if let Some(target) = self.find_definition_by_path_depth(&format!("{cname}::{rel_sig}"), depth + 1) {
                                if target.kind != DefKind::Use && target.kind != DefKind::Local {
                                    return target;
                                }
                            }
                        }
                    }
                    if let Some(target) = self.find_definition_by_path_depth(sig_clean, depth + 1) {
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
            let first_seg = candidate_str.split("::").next().unwrap_or(candidate_str);
            let first_readers = self.readers_for_crate_fast(first_seg);
            let has_first_crate = candidate_str.contains("::") && !first_readers.is_empty();

            // Fast path: if path starts with `<crate_name>::` or equals `<crate_name>`, check that crate first
            for &i in first_readers.iter() {
                let reader = &self.readers[i];
                let sub = strip_crate_prefix(candidate_str, first_seg).unwrap_or(candidate_str);
                if let Some(def) =
                    reader.find_definition_by_path(candidate_str).or_else(|| (sub != candidate_str).then(|| reader.find_definition_by_path(sub)).flatten()).or_else(|| {
                        (sub != candidate_str && !sub.contains("::"))
                            .then(|| reader.definitions().iter().find(|d| DefKind::from(d.kind) == DefKind::Macro && reader.get_str(d.name_offset()) == sub))
                            .flatten()
                    })
                {
                    return Some(resolve_def(self.make_resolved_definition(i, def)));
                }
                if candidate_str == first_seg && self.target_kinds.get(i).copied().flatten().is_none_or(|k| k.is_lib()) {
                    if let Some(root_def) = reader.find_definition_by_path("") {
                        let mut resolved = self.make_resolved_definition(i, root_def);
                        resolved.name = reader.crate_name();
                        resolved.path = reader.crate_name();
                        return Some(resolved);
                    }
                }
            }

            // General search across remaining crates
            if depth == 0 || !has_first_crate {
                for (i, reader) in self.readers.iter().enumerate() {
                    if reader.crate_name() == first_seg {
                        continue;
                    }
                    if let Some(def) = reader.find_definition_by_path(candidate_str) {
                        return Some(resolve_def(self.make_resolved_definition(i, def)));
                    }
                }
            }
        }

        // Fallback: candidate may be `owner::item_name` where item is registered under owner
        for candidate in super::std_map::std_path_candidates(path) {
            let candidate_str = candidate.as_str();
            if let Some((owner, item_name)) = candidate_str.rsplit_once("::") {
                let first_seg = owner.split("::").next().unwrap_or(owner);
                let first_readers = self.readers_for_crate_fast(first_seg);
                let has_first_crate = !first_readers.is_empty();
                for &i in first_readers.iter() {
                    let reader = &self.readers[i];
                    for def in reader.find_definitions_by_owner(owner) {
                        if reader.get_str(def.name_offset()) == item_name {
                            return Some(resolve_def(self.make_resolved_definition(i, def)));
                        }
                    }
                    if let Some(sub_owner) = strip_crate_prefix(owner, first_seg) {
                        for def in reader.find_definitions_by_owner(sub_owner) {
                            if reader.get_str(def.name_offset()) == item_name {
                                return Some(resolve_def(self.make_resolved_definition(i, def)));
                            }
                        }
                    }
                }
                if depth == 0 || !has_first_crate {
                    for (i, reader) in self.readers.iter().enumerate() {
                        if reader.crate_name() == first_seg {
                            continue;
                        }
                        for def in reader.find_definitions_by_owner(owner) {
                            if reader.get_str(def.name_offset()) == item_name {
                                return Some(resolve_def(self.make_resolved_definition(i, def)));
                            }
                        }
                    }
                }
            }
        }

        if let Some(var_def) = self.find_variant_ref_definition(path, depth) {
            return Some(var_def);
        }

        self.resolve_path_via_use_prefix(path, depth)
    }

    /// Returns the source root path for a loaded crate, if it is an external dependency.
    pub fn crate_source_root(&self, crate_name: &str) -> Option<&Path> {
        for &i in self.readers_for_crate_fast(crate_name).iter() {
            if let Some(Some(root)) = self.source_roots.get(i) {
                return Some(root.as_path());
            }
        }
        None
    }

    /// Returns the crate name that contains or owns the specified file path, if known.
    pub fn crate_name_for_file(&self, file_path: &str) -> Option<&str> {
        self.reader_index_for_file(file_path)
            .or_else(|| self.candidate_reader_indices_for_file(file_path).into_iter().next())
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

    /// Finds a generic type parameter (`DefKind::TyParam`) in scope at `(file, line, col)`.
    pub fn find_type_param_in_scope<'a>(&'a self, file: &str, name: &str, line: u32, col: u32) -> Option<ResolvedDefinition<'a>> {
        for i in self.candidate_reader_indices_for_file(file) {
            let reader = &self.readers[i];
            let Some(target_file) = self.target_file_for_reader(i, file) else {
                continue;
            };

            if let Some((def, _)) = reader.find_type_param_in_scope(target_file.as_ref(), name, line, col) {
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
                    let def_idx = (def_entry as *const _ as usize - reader.definitions().as_ptr() as usize) / std::mem::size_of::<DefEntry>();
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
