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
    /// Finds all references to a symbol across every crate in the workspace.
    pub fn find_references<'a>(&'a self, target_path: &str) -> Vec<WorkspaceReference<'a>> {
        let canonical = self.find_definition_by_path(target_path).map(|d| d.path).unwrap_or(target_path);
        self.find_references_for_unit(canonical, None)
    }

    /// Inferred expression type at cursor location.
    pub fn type_at(&self, file: &str, line: u32, col: u32) -> Option<&str> {
        self.type_entry_at(file, line, col).map(|(ty, _)| ty)
    }

    /// Inferred expression type and optional canonical owner path at cursor location.
    pub fn type_entry_at(&self, file: &str, line: u32, col: u32) -> Option<(&str, Option<&str>)> {
        let pairs = self.resolved_reader_file_ids(file);
        self.type_entry_at_in_pairs(&pairs, line, col)
    }

    pub(crate) fn type_entry_at_in_pairs(&self, pairs: &[(usize, u32)], line: u32, col: u32) -> Option<(&str, Option<&str>)> {
        for &(i, fid) in pairs {
            if let Some(res) = self.readers[i].type_entry_at_file_id(fid, line, col) {
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
    pub fn find_definitions_by_owner_for_file<'a>(&'a self, file: &str, owner: &str) -> Vec<ResolvedDefinition<'a>> {
        self.find_definitions_by_owner_internal(owner, Some(file))
    }

    fn find_definitions_by_owner_internal<'a>(&'a self, owner: &str, file_opt: Option<&str>) -> Vec<ResolvedDefinition<'a>> {
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
                let allow_with_prefix = !is_sysroot || cand_str.contains("::") || cand_str.starts_with("core") || cand_str.starts_with("std") || cand_str.starts_with("alloc");

                let prefix_defs = if allow_with_prefix { Some(reader.find_definitions_by_owner(&with_prefix)) } else { None };

                for def in reader.find_definitions_by_owner(cand_str).into_iter().chain(reader.find_definitions_by_owner(sub)).chain(prefix_defs.into_iter().flatten()) {
                    let key = (i, def.path_offset(), def.span.start_line, def.span.start_col);
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

    pub(crate) fn find_variant_ref_definition<'a>(&'a self, path: &str, depth: u8) -> Option<ResolvedDefinition<'a>> {
        let last = path.rsplit("::").next().unwrap_or(path);
        if !last.as_bytes().first().is_some_and(|b| b.is_ascii_uppercase()) {
            return None;
        }
        for candidate in super::std_map::std_path_candidates(path) {
            let cand = candidate.as_str();
            let Some((first_seg, _)) = cand.split_once("::") else {
                continue;
            };
            let first_readers = self.readers_for_crate_fast(first_seg);
            let has_first_crate = !first_readers.is_empty();
            for &i in first_readers.iter() {
                let reader = &self.readers[i];
                let sub = super::strip_crate_prefix(cand, first_seg).unwrap_or(cand);
                for q in [Some(cand), (sub != cand).then_some(sub)].into_iter().flatten() {
                    for r in reader.find_references(q) {
                        if matches!(DefKind::from(r.kind), DefKind::Variant | DefKind::VariantCtor) && r.target_line() > 0 {
                            return Some(self.finalize_reader_resolution(i, reader.resolve_ref_definition(r)));
                        }
                    }
                }
            }
            if depth == 0 && !has_first_crate {
                for (i, reader) in self.readers.iter().enumerate() {
                    if reader.crate_name() == first_seg {
                        continue;
                    }
                    let sub = super::strip_crate_prefix(cand, first_seg).unwrap_or(cand);
                    for q in [Some(cand), (sub != cand).then_some(sub)].into_iter().flatten() {
                        for r in reader.find_references(q) {
                            if matches!(DefKind::from(r.kind), DefKind::Variant | DefKind::VariantCtor) && r.target_line() > 0 {
                                return Some(self.finalize_reader_resolution(i, reader.resolve_ref_definition(r)));
                            }
                        }
                    }
                }
            }
        }
        None
    }

    pub(crate) fn resolve_path_via_use_prefix<'a>(&'a self, path: &str, depth: u8) -> Option<ResolvedDefinition<'a>> {
        if depth >= 2 {
            return None;
        }
        let mut seen_q = std::collections::HashSet::new();
        for candidate in super::std_map::std_path_candidates(path) {
            let cand = candidate.as_str();
            let Some((first_seg, sub)) = cand.split_once("::") else {
                continue;
            };
            for &i in self.readers_for_crate_fast(first_seg).iter() {
                let reader = &self.readers[i];
                if !sub.contains("::") {
                    for def in reader.definitions() {
                        if DefKind::from(def.kind) == DefKind::Macro && reader.get_str(def.name_offset()) == sub {
                            return Some(self.make_resolved_definition(i, def));
                        }
                    }
                }
                for (idx, _) in sub.match_indices("::") {
                    let (sub_prefix, sub_rest) = (&sub[..idx], &sub[idx + 2..]);
                    for sfx in ["", "::*", "::{use#0}::*", "::{use#1}::*", "::{use#2}::*"] {
                        let key = format!("{sub_prefix}{sfx}");
                        let Some(def) = reader.find_definition_by_path(&key) else {
                            continue;
                        };
                        if DefKind::from(def.kind) != DefKind::Use || def.sig_offset() == 0 {
                            continue;
                        }
                        let raw_sig = reader.get_str(def.sig_offset()).trim_end_matches("::*");
                        if raw_sig.is_empty() {
                            continue;
                        }
                        let expanded = match raw_sig.strip_prefix("self::") {
                            Some(r) => format!("{first_seg}::{sub_prefix}::{r}"),
                            None => raw_sig.to_string(),
                        };
                        let q = format!("{expanded}::{sub_rest}");
                        if seen_q.insert(q.clone()) {
                            if let Some(found) = self.find_definition_by_path_depth(&q, depth + 1) {
                                return Some(found);
                            }
                        }
                    }
                }
                for glob_key in ["*", "{{glob}}", "{use#0}", "{use#0}::*", "{use#1}::*", "{use#2}::*"] {
                    let Some(def) = reader.find_definition_by_path(glob_key) else {
                        continue;
                    };
                    if DefKind::from(def.kind) != DefKind::Use || def.sig_offset() == 0 {
                        continue;
                    }
                    let sig = reader.get_str(def.sig_offset()).trim_end_matches("::*");
                    if sig.is_empty() || sig == first_seg {
                        continue;
                    }
                    let rel = sig.strip_prefix("self::").unwrap_or(sig);
                    let rel_first = rel.split("::").next().unwrap_or(rel);
                    let q = if !sig.starts_with("self::") && self.has_crate(rel_first) { format!("{rel}::{sub}") } else { format!("{first_seg}::{rel}::{sub}") };
                    if seen_q.insert(q.clone()) {
                        if let Some(found) = self.find_definition_by_path_depth(&q, depth + 1) {
                            return Some(found);
                        }
                    }
                }
            }
        }
        None
    }

    pub(crate) fn finalize_reader_resolution<'a>(&'a self, i: usize, mut res: ResolvedDefinition<'a>) -> ResolvedDefinition<'a> {
        let reader = &self.readers[i];
        if res.kind == DefKind::Use {
            if let Some(sig) = res.signature.map(|s| s.trim_end_matches("::*")).filter(|s| !s.is_empty()) {
                let rel = sig.strip_prefix("self::").unwrap_or(sig);
                let owner_cand = res.path.rsplit_once("::").map(|(m, _)| format!("{m}::{rel}"));
                if let Some(t) = owner_cand
                    .as_deref()
                    .and_then(|q| self.find_definition_by_path(q))
                    .or_else(|| self.find_definition_by_path(&format!("{}::{rel}", reader.crate_name())))
                    .or_else(|| self.find_definition_by_path(sig))
                {
                    if t.kind != DefKind::Use && t.kind != DefKind::Local {
                        return t;
                    }
                }
            }
        }
        let crate_prefix = format!("{}::", reader.crate_name());
        let current_uk = self.unit_keys.get(i).and_then(|u| u.as_deref()).unwrap_or_else(|| reader.unit_key());
        let is_explicit_remote = res.unit_key.is_some() && res.unit_key != Some(current_uk);
        let is_local = !is_explicit_remote && (res.path.starts_with(&crate_prefix) || res.path == reader.crate_name());
        if is_local && res.file != "external" {
            res.source_root = if self.reader_is_dependency.get(i).copied().unwrap_or(false) {
                self.source_roots.get(i).and_then(|r| r.as_deref())
            } else {
                self.source_roots.get(i).and_then(|r| r.as_deref()).or(self.workspace_root.as_deref())
            };
            res.unit_key = self.unit_keys.get(i).and_then(|u| u.as_deref());
            res.reader_idx = Some(i);
            return res;
        }
        if let Some(cross_def) = self.find_cross_definition(i, &res) {
            return cross_def;
        }
        if res.unit_key.is_none() {
            res.unit_key = self.unit_keys.get(i).and_then(|u| u.as_deref());
        }
        let owner =
            res.unit_key.filter(|uk| Some(*uk) != self.unit_keys.get(i).and_then(|u| u.as_deref())).and_then(|uk| self.readers_for_unit_fast(uk).first().copied()).unwrap_or(i);
        if res.source_root.is_none() {
            res.source_root = if self.reader_is_dependency.get(owner).copied().unwrap_or(false) {
                self.source_roots.get(owner).and_then(|r| r.as_deref())
            } else {
                self.source_roots.get(owner).and_then(|r| r.as_deref()).or(self.workspace_root.as_deref())
            };
        }
        if res.reader_idx.is_none() {
            res.reader_idx = Some(owner);
        }
        res
    }
}
