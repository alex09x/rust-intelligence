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
