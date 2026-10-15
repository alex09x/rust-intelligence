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
}
