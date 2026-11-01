/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Trait-associated item rename expansion: external trait impl references,
//! associated type projections (`Self::Assoc`, `T::Assoc`, `<T as Trait>::Assoc`),
//! and associated type equality bindings (`Trait<Assoc = ...>`).

use super::trait_impl_refs::{path_matches_trait, reader_path_matches_trait, strip_generics};
use super::*;
use crate::workspace::cursor_path::{is_ident_byte, is_in_string_or_comment, strip_generics_and_refs};
use crate::workspace::cursor_path_assoc::{scan_enclosing_impl_or_trait_headers, scan_type_param_bounds};
use std::collections::HashSet;

fn trait_name_matches(candidate: &str, raw_parent: &str, local_trait: &str, global_trait: &str, trait_short: &str, std_cands: &[String]) -> bool {
    let base = strip_generics_and_refs(strip_generics(candidate));
    if base.is_empty() {
        return false;
    }
    if path_matches_trait(base, raw_parent, local_trait, global_trait, std_cands) {
        return true;
    }
    let cand_short = base.rsplit("::").next().unwrap_or(base);
    if cand_short == trait_short && (!base.contains("::") || trait_short != "Error") {
        return true;
    }
    if trait_short == "Iterator" && matches!(cand_short, "DoubleEndedIterator" | "ExactSizeIterator" | "FusedIterator") {
        return true;
    }
    if trait_short == "Deref" && cand_short == "DerefMut" {
        return true;
    }
    false
}

fn find_enclosing_angle_trait<'a>(lines: &[&'a str], line_idx: usize, pos_byte: usize) -> Option<&'a str> {
    let min_ln = line_idx.saturating_sub(3);
    let mut depth = 0i32;
    for l in (min_ln..=line_idx).rev() {
        let s = if l == line_idx { &lines[l][..pos_byte.min(lines[l].len())] } else { lines[l] };
        let code = s.split("//").next().unwrap_or(s);
        let bytes = code.as_bytes();
        let mut i = bytes.len();
        while i > 0 {
            i -= 1;
            match bytes[i] {
                b'>' => {
                    if i == 0 || bytes[i - 1] != b'-' {
                        depth += 1;
                    }
                }
                b'<' => {
                    if depth > 0 {
                        depth -= 1;
                    } else {
                        let before = code[..i].trim_end().trim_end_matches("::");
                        let mut k = before.len();
                        while k > 0 && (is_ident_byte(before.as_bytes()[k - 1]) || before.as_bytes()[k - 1] == b':') {
                            k -= 1;
                        }
                        let tr = before[k..].trim_matches(':');
                        return (!tr.is_empty()).then_some(tr);
                    }
                }
                b';' | b'{' | b'}' if depth == 0 => return None,
                _ => {}
            }
        }
    }
    None
}

fn extract_qualified_as_trait<'a>(lines: &[&'a str], line_idx: usize, close_gt_byte: usize) -> Option<&'a str> {
    let s = &lines[line_idx][..close_gt_byte.min(lines[line_idx].len())];
    let mut depth = 0i32;
    let bytes = s.as_bytes();
    let mut i = bytes.len();
    while i > 0 {
        i -= 1;
        match bytes[i] {
            b'>' => depth += 1,
            b'<' => {
                if depth > 0 {
                    depth -= 1;
                } else {
                    let inner = &s[i + 1..];
                    let (_, tr) = inner.rsplit_once(" as ")?;
                    return Some(tr.trim());
                }
            }
            _ => {}
        }
    }
    None
}

impl WorkspaceIndex {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn expand_trait_qualified_refs<'a>(
        &'a self,
        def: &ResolvedDefinition<'a>,
        item_name: &'a str,
        raw_parent: &str,
        local_trait: &str,
        global_trait: &str,
        std_cands: &[String],
        seen_locs: &mut HashSet<(String, u32, u32)>,
        out_refs: &mut Vec<WorkspaceReference<'a>>,
    ) {
        for (i, reader) in self.readers.iter().enumerate() {
            if self.is_reader_external(i) {
                continue;
            }
            let source_root = self.source_roots.get(i).and_then(|r| r.as_deref()).or(self.workspace_root.as_deref());
            let unit_key = self.unit_keys.get(i).and_then(|u| u.as_deref());
            for r in reader.references() {
                if DefKind::from(r.kind) != def.kind || !reader.is_name_reference(r) {
                    continue;
                }
                let tp = reader.get_str(r.target_path_offset());
                let Some(prefix) = tp.strip_suffix(item_name).and_then(|p| p.strip_suffix("::")) else {
                    continue;
                };
                let matches_tr = if let Some(impl_tr) = super::qualified_path::extract_impl_trait(tp) {
                    reader_path_matches_trait(reader, impl_tr, raw_parent, local_trait, global_trait, std_cands)
                } else if !prefix.starts_with('<') && !prefix.contains("<impl ") {
                    reader_path_matches_trait(reader, prefix, raw_parent, local_trait, global_trait, std_cands)
                } else {
                    false
                };
                if matches_tr {
                    let span = r.span();
                    let file = reader.get_file(span.file_id());
                    if seen_locs.insert((file.to_string(), span.start_line(), span.start_col())) {
                        out_refs.push(WorkspaceReference {
                            crate_name: reader.crate_name(),
                            file,
                            span,
                            kind: DefKind::from(r.kind),
                            is_write: r.is_write != 0,
                            is_macro_expansion: r.is_macro_expansion(),
                            source_root,
                            unit_key,
                        });
                    }
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn expand_assoc_ty_projections<'a>(
        &'a self,
        def: &ResolvedDefinition<'a>,
        item_name: &'a str,
        raw_parent: &str,
        local_trait: &str,
        global_trait: &str,
        std_cands: &[String],
        assoc_files: &mut Vec<(usize, u32)>,
        seen_locs: &mut HashSet<(String, u32, u32)>,
        out_refs: &mut Vec<WorkspaceReference<'a>>,
    ) {
        let trait_short = local_trait.rsplit("::").next().unwrap_or(local_trait);
        for (i, reader) in self.readers.iter().enumerate() {
            if self.is_reader_external(i) {
                continue;
            }
            let defs = reader.definitions();
            for d in defs {
                if d.sig_offset() != 0 {
                    let sig = reader.get_str(d.sig_offset());
                    if sig.contains(trait_short) || sig.contains(item_name) {
                        assoc_files.push((i, d.span().file_id()));
                    }
                }
            }
            for oe in reader.defs_by_owner() {
                if reader.get_str(oe.owner_path_offset()).contains(trait_short) {
                    if let Some(d) = defs.get(oe.def_index() as usize) {
                        assoc_files.push((i, d.span().file_id()));
                    }
                }
            }
            for r in reader.references() {
                if reader.get_str(r.target_path_offset()).ends_with(trait_short) {
                    assoc_files.push((i, r.span().file_id()));
                }
            }
        }
        assoc_files.sort_unstable();
        assoc_files.dedup();

        let pat = format!("::{item_name}");
        let item_chars = item_name.chars().count() as u32;
        let mut scanned_files: HashSet<&str> = HashSet::new();
        for &(r_idx, fid) in assoc_files.iter() {
            let reader = &self.readers[r_idx];
            let file = reader.get_file(fid);
            if !scanned_files.insert(file) {
                continue;
            }
            let Some(content) = self.read_source_file(file) else {
                continue;
            };
            let lines: Vec<&str> = content.lines().collect();
            let source_root = self.source_roots.get(r_idx).and_then(|r| r.as_deref()).or(self.workspace_root.as_deref());
            let unit_key = self.unit_keys.get(r_idx).and_then(|u| u.as_deref());
            for (l_idx, &line) in lines.iter().enumerate() {
                let line_1based = (l_idx + 1) as u32;
                for (b_pos, _) in line.match_indices(&pat) {
                    if b_pos == 0 || !(line.as_bytes()[b_pos - 1] == b'>' || is_ident_byte(line.as_bytes()[b_pos - 1])) {
                        continue;
                    }
                    let id_start = b_pos + 2;
                    let id_end = id_start + item_name.len();
                    if id_end < line.len() && is_ident_byte(line.as_bytes()[id_end]) {
                        continue;
                    }
                    if is_in_string_or_comment(&lines, l_idx, id_start) {
                        continue;
                    }
                    let col_1based = line[..id_start].chars().count() as u32 + 1;
                    if seen_locs.contains(&(file.to_string(), line_1based, col_1based)) {
                        continue;
                    }
                    let mut matches_target = false;
                    if line.as_bytes()[b_pos - 1] == b'>' {
                        if let Some(tr) = extract_qualified_as_trait(&lines, l_idx, b_pos - 1) {
                            matches_target = trait_name_matches(tr, raw_parent, local_trait, global_trait, trait_short, std_cands);
                        }
                    }
                    if !matches_target {
                        if let Some(rdef) = self.resolve_cursor_path_at(file, line_1based, col_1based) {
                            matches_target = rdef.kind == DefKind::AssocTy
                                && rdef.name == item_name
                                && (rdef.path == def.path
                                    || rdef.path == format!("{local_trait}::{item_name}")
                                    || rdef.path == format!("{global_trait}::{item_name}")
                                    || rdef.path.ends_with(&format!("::{local_trait}::{item_name}"))
                                    || super::qualified_path::extract_impl_trait(rdef.path)
                                        .is_some_and(|tr| path_matches_trait(tr, raw_parent, local_trait, global_trait, std_cands)));
                        }
                    }
                    if !matches_target && is_ident_byte(line.as_bytes()[b_pos - 1]) {
                        let mut p_start = b_pos;
                        while p_start > 0 && is_ident_byte(line.as_bytes()[p_start - 1]) {
                            p_start -= 1;
                        }
                        let prefix = &line[p_start..b_pos];
                        if prefix == "Self" {
                            matches_target = scan_enclosing_impl_or_trait_headers(&lines, l_idx)
                                .iter()
                                .any(|h| trait_name_matches(h, raw_parent, local_trait, global_trait, trait_short, std_cands));
                        } else if !prefix.is_empty() && prefix.as_bytes()[0].is_ascii_uppercase() && (p_start < 2 || &line[p_start - 2..p_start] != "::") {
                            matches_target =
                                scan_type_param_bounds(&lines, l_idx, prefix).iter().any(|b| trait_name_matches(b, raw_parent, local_trait, global_trait, trait_short, std_cands));
                        }
                    }
                    if matches_target && seen_locs.insert((file.to_string(), line_1based, col_1based)) {
                        out_refs.push(WorkspaceReference {
                            crate_name: reader.crate_name(),
                            file,
                            span: crate::format::Span::new(fid, line_1based, col_1based, line_1based, col_1based + item_chars),
                            kind: DefKind::AssocTy,
                            is_write: false,
                            is_macro_expansion: false,
                            source_root,
                            unit_key,
                        });
                    }
                }

                if line.contains('=') && line.contains(item_name) {
                    for (id_start, _) in line.match_indices(item_name) {
                        let id_end = id_start + item_name.len();
                        if (id_start > 0 && is_ident_byte(line.as_bytes()[id_start - 1])) || (id_end < line.len() && is_ident_byte(line.as_bytes()[id_end])) {
                            continue;
                        }
                        let after = line[id_end..].trim_start();
                        if !after.starts_with('=') || after.starts_with("==") || after.starts_with("=>") {
                            continue;
                        }
                        let before_trim = line[..id_start].trim_end();
                        let prev_delim = if let Some(b) = before_trim.as_bytes().last().copied() {
                            Some(b)
                        } else if l_idx > 0 {
                            lines[l_idx - 1].split("//").next().unwrap_or("").trim_end().as_bytes().last().copied()
                        } else {
                            None
                        };
                        if !matches!(prev_delim, Some(b'<') | Some(b',')) || is_in_string_or_comment(&lines, l_idx, id_start) {
                            continue;
                        }
                        let col_1based = line[..id_start].chars().count() as u32 + 1;
                        if seen_locs.contains(&(file.to_string(), line_1based, col_1based)) {
                            continue;
                        }
                        if let Some(tr) = find_enclosing_angle_trait(&lines, l_idx, id_start) {
                            if trait_name_matches(tr, raw_parent, local_trait, global_trait, trait_short, std_cands)
                                && seen_locs.insert((file.to_string(), line_1based, col_1based))
                            {
                                out_refs.push(WorkspaceReference {
                                    crate_name: reader.crate_name(),
                                    file,
                                    span: crate::format::Span::new(fid, line_1based, col_1based, line_1based, col_1based + item_chars),
                                    kind: DefKind::AssocTy,
                                    is_write: false,
                                    is_macro_expansion: false,
                                    source_root,
                                    unit_key,
                                });
                            }
                        }
                    }
                }
            }
        }
    }
}
