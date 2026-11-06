/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Compiler-backed `textDocument/implementation` resolution over `DefKind::Impl` and trait items.

use super::impl_parse::{
    contains_ident_token, has_multiple_generic_params, is_builtin_derive_trait, is_impl_inside_fn, is_nominal_type_syntax, parse_impl_def_path, strip_tail_generics,
};
use super::trait_impl_refs::reader_path_matches_trait;
use super::*;
use crate::format::{join_resolving_overlap, Span};
use std::collections::HashSet;

/// An implementation already reported: resolved file, start line and start column. The column keeps apart the impls
/// one `#[derive(PartialEq, Debug)]` line produces; the same impl seen from several compilation units shares all three.
type SeenLoc = (String, u32, u32);

impl WorkspaceIndex {
    /// Returns true if any workspace shard contains compiler-emitted `DefKind::Impl` definitions.
    pub fn has_compiler_impl_defs(&self) -> bool {
        self.readers.iter().enumerate().any(|(i, r)| !self.is_reader_external(i) && r.definitions().iter().any(|d| DefKind::from(d.kind) == DefKind::Impl))
    }

    /// Finds all implementations for a nominal type, trait, or trait associated item.
    pub fn find_implementations<'a>(&'a self, def: &ResolvedDefinition<'a>, _file: &Path) -> Vec<WorkspaceReference<'a>> {
        let mut out = Vec::new();
        let mut seen_locs: HashSet<SeenLoc> = HashSet::new();

        if matches!(def.kind, DefKind::AssocFn | DefKind::AssocConst | DefKind::AssocTy) {
            self.find_trait_item_implementations(def, &mut seen_locs, &mut out);
        } else if matches!(def.kind, DefKind::Struct | DefKind::Enum | DefKind::Union | DefKind::TyAlias | DefKind::Trait) {
            self.find_type_or_trait_implementations(def, &mut seen_locs, &mut out);
        }

        if !self.has_compiler_impl_defs() {
            for r in self.find_references(def.path) {
                if r.kind == DefKind::Impl {
                    let resolved = r.resolved_path().to_string_lossy().into_owned();
                    if seen_locs.insert((resolved, r.span.start_line(), r.span.start_col())) {
                        out.push(r);
                    }
                }
            }
        }

        out
    }

    fn find_type_or_trait_implementations<'a>(&'a self, def: &ResolvedDefinition<'a>, seen_locs: &mut HashSet<SeenLoc>, out: &mut Vec<WorkspaceReference<'a>>) {
        let is_trait_query = def.kind == DefKind::Trait;
        let raw_base = strip_tail_generics(def.path);
        let def_crate = def.reader_idx.and_then(|idx| self.readers.get(idx).map(|r| r.crate_name())).unwrap_or_else(|| super::qualified_path::extract_root_crate(raw_base));
        let is_def_external = def.reader_idx.is_some_and(|idx| self.is_reader_external(idx));
        let local_target = strip_crate_prefix(raw_base, def_crate).unwrap_or(raw_base);
        let global_target = format!("{def_crate}::{local_target}");
        let std_cands = super::std_map::std_path_candidates(&global_target);
        let def_resolved_file = def.resolved_path();
        let def_file_readers: HashSet<usize> = self.candidate_reader_indices_for_file(def_resolved_file.to_str().unwrap_or(def.file)).into_iter().collect();
        let def_is_fn_local = def.reader_idx.is_some_and(|idx| is_impl_inside_fn(&self.readers[idx], def.path, def.path));

        for (i, reader) in self.readers.iter().enumerate() {
            let r_crate = reader.crate_name();
            if self.is_reader_external(i) && !(is_def_external && r_crate == def_crate) {
                continue;
            }
            let is_local_reader = Some(i) == def.reader_idx || (r_crate == def_crate && (is_def_external || def_file_readers.contains(&i)));
            let source_root = if self.reader_is_dependency.get(i).copied().unwrap_or(false) {
                self.source_roots.get(i).and_then(|r| r.as_deref())
            } else {
                self.source_roots.get(i).and_then(|r| r.as_deref()).or(self.workspace_root.as_deref())
            };
            let unit_key = self.unit_keys.get(i).and_then(|u| u.as_deref());
            let dup_line_bounds = if is_local_reader {
                let same_kind_lines: Vec<(u32, u32)> = reader
                    .definitions()
                    .iter()
                    .filter(|d| {
                        DefKind::from(d.kind) == def.kind && !d.is_reexport() && reader.get_str(d.path_offset()) == def.path && reader.get_file(d.span().file_id()) == def.file
                    })
                    .map(|d| (d.span().start_line(), d.span().end_line()))
                    .collect();
                if same_kind_lines.len() > 1 {
                    let lower = same_kind_lines.iter().filter(|&&(sl, _)| sl < def.start_line).map(|&(_, el)| el + 1).max().unwrap_or(1).max(def.start_line.saturating_sub(8));
                    let upper = same_kind_lines.iter().filter(|&&(sl, _)| sl > def.start_line).map(|&(sl, _)| sl.saturating_sub(4)).min().unwrap_or(u32::MAX);
                    Some((lower, upper))
                } else {
                    None
                }
            } else {
                None
            };

            for impl_def in reader.definitions() {
                if DefKind::from(impl_def.kind) != DefKind::Impl {
                    continue;
                }
                let raw_impl = reader.get_str(impl_def.path_offset());
                let (trait_opt, self_ty) = parse_impl_def_path(raw_impl);
                // Marker impls the compiler adds for derives (`derive(PartialEq)` → `StructuralPartialEq`) are not
                // implementations anyone wrote; rust-analyzer does not list them.
                if trait_opt.is_some_and(|t| matches!(strip_tail_generics(t).rsplit("::").next(), Some("StructuralPartialEq" | "TrivialClone"))) {
                    continue;
                }
                let matched = if is_trait_query {
                    let Some(tp) = trait_opt else { continue };
                    let cand_base = strip_tail_generics(tp);
                    if !Self::impl_target_matches(reader, cand_base, is_local_reader, def_crate, raw_base, local_target, &global_target, &std_cands) {
                        continue;
                    }
                    !(!def_is_fn_local && is_impl_inside_fn(reader, raw_impl, self_ty))
                } else {
                    if !is_nominal_type_syntax(self_ty) {
                        continue;
                    }
                    let cand_base = strip_tail_generics(self_ty);
                    Self::impl_target_matches(reader, cand_base, is_local_reader, def_crate, raw_base, local_target, &global_target, &std_cands)
                };
                if !matched {
                    continue;
                }
                let impl_file = reader.get_file(impl_def.span().file_id());
                if let Some((lower_line, upper_line)) = dup_line_bounds {
                    if impl_file == def.file && (impl_def.span().start_line() < lower_line || impl_def.span().start_line() >= upper_line) {
                        continue;
                    }
                }
                let resolved_file_buf = {
                    let p = Path::new(impl_file);
                    if p.is_absolute() {
                        p.to_path_buf()
                    } else if let Some(root) = source_root {
                        join_resolving_overlap(root, p)
                    } else {
                        p.to_path_buf()
                    }
                };
                let adj_span = self.resolve_impl_target_span(reader, impl_def, trait_opt, self_ty, def, &resolved_file_buf);
                // One impl seen through several units counts once; several impls at one place (`#[derive(A, B)]`)
                // count each.
                let seen_key = format!("{}#{raw_impl}", resolved_file_buf.to_string_lossy());
                if seen_locs.insert((seen_key, adj_span.start_line(), adj_span.start_col())) {
                    out.push(WorkspaceReference {
                        crate_name: r_crate,
                        file: impl_file,
                        span: adj_span,
                        kind: DefKind::Impl,
                        is_write: false,
                        is_macro_expansion: impl_def.is_macro_expansion(),
                        source_root,
                        unit_key,
                    });
                }
            }
        }
    }

    #[inline]
    #[allow(clippy::too_many_arguments)]
    fn impl_target_matches(
        reader: &IndexReader<memmap2::Mmap>,
        cand_base: &str,
        is_local_reader: bool,
        def_crate: &str,
        raw_base: &str,
        local_target: &str,
        global_target: &str,
        std_cands: &[String],
    ) -> bool {
        if is_local_reader {
            cand_base == local_target || cand_base == global_target || cand_base == raw_base || std_cands.iter().any(|c| c == cand_base)
        } else {
            if reader.crate_name() != def_crate && reader.find_definition_by_path(cand_base).is_some_and(|d| !d.is_reexport()) {
                return false;
            }
            cand_base == global_target || std_cands.iter().any(|c| c == cand_base)
        }
    }

    fn resolve_impl_target_span(
        &self,
        reader: &IndexReader<memmap2::Mmap>,
        impl_def: &DefEntry,
        trait_opt: Option<&str>,
        self_ty: &str,
        queried_def: &ResolvedDefinition<'_>,
        resolved_file: &Path,
    ) -> Span {
        let span = impl_def.span();
        let s_line = span.start_line();
        let e_line = span.end_line();
        let file_id = span.file_id();
        let self_ty_base = strip_tail_generics(self_ty);
        let self_ty_short = self_ty_base.rsplit("::").next().unwrap_or(self_ty_base);

        if impl_def.is_macro_expansion() {
            if trait_opt.is_none() || is_builtin_derive_trait(trait_opt) || has_multiple_generic_params(self_ty) {
                return span;
            }
            if matches!(queried_def.kind, DefKind::Struct | DefKind::Enum | DefKind::Union)
                && !queried_def.is_macro_expansion
                && s_line <= queried_def.end_line
                && queried_def.start_line <= s_line + 30
                && queried_def.resolved_path() == resolved_file
            {
                return Span::new(file_id, queried_def.start_line, queried_def.start_col, queried_def.end_line, queried_def.end_col);
            }
            let r_crate = reader.crate_name();
            let self_ty_local = strip_crate_prefix(self_ty_base, r_crate).unwrap_or(self_ty_base);
            if let Some(tdef) = reader.find_definition_by_path(self_ty_local).or_else(|| reader.find_definition_by_path(self_ty_base)) {
                let ts = tdef.span();
                if !tdef.is_macro_expansion() && ts.file_id() == file_id && s_line <= ts.end_line() && ts.start_line() <= e_line.max(s_line) + 30 {
                    return ts;
                }
            }
            for d in reader.definitions() {
                let ds = d.span();
                if ds.file_id() != file_id || d.is_macro_expansion() {
                    continue;
                }
                if matches!(DefKind::from(d.kind), DefKind::Struct | DefKind::Enum | DefKind::Union | DefKind::Trait)
                    && s_line <= ds.end_line()
                    && ds.start_line() <= e_line.max(s_line) + 20
                    && reader.get_str(d.name_offset()) == self_ty_short
                {
                    return ds;
                }
            }
            if let Ok(content) = std::fs::read_to_string(resolved_file) {
                let first_line = content.lines().nth(s_line.saturating_sub(1) as usize).unwrap_or("").trim();
                if (first_line.contains("#[from]") || first_line.starts_with("#[error"))
                    && matches!(queried_def.kind, DefKind::Enum | DefKind::Struct)
                    && s_line > queried_def.start_line
                    && s_line <= queried_def.start_line + 30
                    && queried_def.resolved_path() == resolved_file
                {
                    return Span::new(file_id, queried_def.start_line, queried_def.start_col, queried_def.end_line, queried_def.end_col);
                }
                if first_line.starts_with("#[") {
                    if !first_line.starts_with("#[derive") {
                        for (idx, line_txt) in content.lines().enumerate().skip(s_line as usize).take(3) {
                            let tr = line_txt.trim_start();
                            if (tr.starts_with("impl ") || tr.starts_with("impl<")) && contains_ident_token(tr, self_ty_short) {
                                let ln = (idx + 1) as u32;
                                return Span::new(file_id, ln, 1, ln, 1);
                            }
                        }
                    }
                    for d in reader.definitions() {
                        let ds = d.span();
                        if ds.file_id() == file_id
                            && !d.is_macro_expansion()
                            && matches!(DefKind::from(d.kind), DefKind::Struct | DefKind::Enum | DefKind::Union | DefKind::Trait)
                            && ds.start_line() > s_line
                            && ds.start_line() <= s_line + 12
                        {
                            return ds;
                        }
                    }
                }
                if e_line > s_line || first_line.ends_with('(') || first_line.ends_with('{') {
                    let scan_end = e_line.max(s_line + 5);
                    for (idx, line_txt) in content.lines().enumerate().skip(s_line.saturating_sub(1) as usize).take((scan_end - s_line + 1) as usize) {
                        let tr = line_txt.trim_start();
                        if tr.starts_with("::") && contains_ident_token(tr, self_ty_short) {
                            let ln = (idx + 1) as u32;
                            return Span::new(file_id, ln, 1, ln, 1);
                        }
                    }
                }
            }
            return span;
        }

        if e_line > s_line && !self_ty_short.is_empty() {
            if let Ok(content) = std::fs::read_to_string(resolved_file) {
                let max_scan = (e_line - s_line + 1).min(12) as usize;
                let mut seen_for = trait_opt.is_none();
                for (idx, line_txt) in content.lines().enumerate().skip(s_line.saturating_sub(1) as usize).take(max_scan) {
                    let trimmed = line_txt.trim();
                    if trimmed.starts_with("where") && idx + 1 > s_line as usize {
                        break;
                    }
                    if !seen_for && (trimmed.starts_with("for ") || line_txt.contains(" for ")) {
                        seen_for = true;
                    }
                    if seen_for && contains_ident_token(line_txt, self_ty_short) {
                        let ln = (idx + 1) as u32;
                        return Span::new(file_id, ln, 1, ln, 1);
                    }
                    if trimmed.contains('{') {
                        break;
                    }
                }
            }
        }
        span
    }

    fn find_trait_item_implementations<'a>(&'a self, def: &ResolvedDefinition<'a>, seen_locs: &mut HashSet<SeenLoc>, out: &mut Vec<WorkspaceReference<'a>>) {
        let Some((raw_parent, item_name)) = def.path.rsplit_once("::") else { return };
        let Some((_, local_trait, global_trait)) = self.resolve_workspace_trait_container(raw_parent, def.reader_idx, false) else {
            return;
        };
        let std_cands = super::std_map::std_path_candidates(&global_trait);

        for (i, reader) in self.readers.iter().enumerate() {
            if self.is_reader_external(i) {
                continue;
            }
            let source_root = self.source_roots.get(i).and_then(|r| r.as_deref()).or(self.workspace_root.as_deref());
            let unit_key = self.unit_keys.get(i).and_then(|u| u.as_deref());
            for de in reader.deref_entries() {
                let tgt = reader.get_str(de.target_type_offset);
                if !reader_path_matches_trait(reader, tgt, raw_parent, &local_trait, &global_trait, &std_cands) {
                    continue;
                }
                for impl_def in reader.find_definitions_by_owner(reader.get_str(de.source_type_offset)) {
                    if DefKind::from(impl_def.kind) != def.kind || impl_def.is_macro_expansion() || reader.get_str(impl_def.name_offset()) != item_name {
                        continue;
                    }
                    let impl_path = reader.get_str(impl_def.path_offset());
                    if let Some(impl_trait) = super::qualified_path::extract_impl_trait(impl_path) {
                        if !reader_path_matches_trait(reader, impl_trait, raw_parent, &local_trait, &global_trait, &std_cands) {
                            continue;
                        }
                    }
                    let span = impl_def.span();
                    let file = reader.get_file(span.file_id());
                    let r = WorkspaceReference { crate_name: reader.crate_name(), file, span, kind: def.kind, is_write: false, is_macro_expansion: false, source_root, unit_key };
                    if seen_locs.insert((r.resolved_path().to_string_lossy().into_owned(), span.start_line(), span.start_col())) {
                        out.push(r);
                    }
                }
            }
            for impl_def in reader.definitions() {
                if DefKind::from(impl_def.kind) != def.kind || impl_def.is_macro_expansion() || reader.get_str(impl_def.name_offset()) != item_name {
                    continue;
                }
                let impl_path = reader.get_str(impl_def.path_offset());
                let Some(impl_trait) = super::qualified_path::extract_impl_trait(impl_path) else {
                    continue;
                };
                if !reader_path_matches_trait(reader, impl_trait, raw_parent, &local_trait, &global_trait, &std_cands) {
                    continue;
                }
                let span = impl_def.span();
                let file = reader.get_file(span.file_id());
                let r = WorkspaceReference { crate_name: reader.crate_name(), file, span, kind: def.kind, is_write: false, is_macro_expansion: false, source_root, unit_key };
                if seen_locs.insert((r.resolved_path().to_string_lossy().into_owned(), span.start_line(), span.start_col())) {
                    out.push(r);
                }
            }
        }
    }
}
