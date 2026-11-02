/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Rename-specific reference expansion (`#[cfg]` siblings, proc-macro quote tokens, field shorthands, `Owner::Child`).

use super::rename_check::detect_enclosing_fn_bounds;
use super::*;
use crate::workspace::cursor_path::{is_ident_byte, is_in_string_or_comment};
use crate::workspace::rename_parse::has_self_receiver;
use crate::TargetKind;
use std::collections::HashSet;

impl WorkspaceIndex {
    pub(super) fn expand_rename_type_and_sibling_refs<'a>(&'a self, def: &ResolvedDefinition<'a>, out_refs: &mut Vec<WorkspaceReference<'a>>) {
        let mut seen_locs: HashSet<(String, u32, u32)> = out_refs.iter().map(|r| (r.file.to_string(), r.span.start_line(), r.span.start_col())).collect();
        seen_locs.insert((def.file.to_string(), def.start_line, def.start_col));
        let make_ref =
            |crate_name: &'a str, file: &'a str, span: crate::format::Span, kind: DefKind, is_macro_expansion: bool, source_root: Option<&'a Path>, unit_key: Option<&'a str>| {
                WorkspaceReference { crate_name, file, span, kind, is_write: false, is_macro_expansion, source_root, unit_key }
            };

        let owner_raw = def.path.rsplit_once("::").map(|(p, _)| super::trait_impl_refs::strip_generics(p).trim_end_matches("::")).unwrap_or("");
        let owner_short = owner_raw.rsplit("::").next().unwrap_or(owner_raw);
        let has_self = def.signature.is_some_and(has_self_receiver);
        let fn_bounds = detect_enclosing_fn_bounds(self, def);
        let is_fn_local_dup = fn_bounds.is_some();
        if let Some((fn_start, fn_end)) = fn_bounds {
            out_refs.retain(|r| r.file == def.file && r.span.start_line() >= fn_start && r.span.start_line() <= fn_end);
            seen_locs.retain(|(f, ln, _)| f == def.file && *ln >= fn_start && *ln <= fn_end);
        }

        let def_sr = def.reader_idx.and_then(|idx| self.source_roots.get(idx).and_then(|r| r.as_deref())).or(self.workspace_root.as_deref());
        let def_pairs = self.resolved_reader_file_ids(def.file);
        if def_pairs.len() > 1 && def.start_line > 0 && (out_refs.is_empty() || def.kind != DefKind::Fn) {
            for &(i, fid) in &def_pairs {
                if Some(i) == def.reader_idx || self.is_reader_external(i) {
                    continue;
                }
                let sr = self.source_roots.get(i).and_then(|r| r.as_deref()).or(self.workspace_root.as_deref());
                if sr != def_sr {
                    continue;
                }
                let uk = self.unit_keys.get(i).and_then(|u| u.as_deref());
                let other_r = &self.readers[i];
                let o_defs = other_r.definitions();
                for &(_, sl, d_idx) in self.unit_match.defs_in_file_up_to(i, fid, def.start_line).iter().rev().take_while(|&&(_, sl, _)| sl == def.start_line) {
                    let Some(d) = o_defs.get(d_idx as usize) else { continue };
                    if sl == def.start_line && DefKind::from(d.kind) == def.kind && other_r.get_str(d.name_offset()) == def.name {
                        let op = other_r.get_str(d.path_offset());
                        let ol = op.strip_prefix(&format!("{}::", other_r.crate_name())).unwrap_or(op);
                        for r in other_r.find_references(op).into_iter().chain(other_r.find_references(ol)) {
                            let (sp, f) = (r.span(), other_r.get_file(r.span().file_id()));
                            if DefKind::from(r.kind) == def.kind
                                && r.target_line() == def.start_line
                                && fn_bounds.is_none_or(|(fs, fe)| f == def.file && sp.start_line() >= fs && sp.start_line() <= fe)
                                && seen_locs.insert((f.to_string(), sp.start_line(), sp.start_col()))
                            {
                                out_refs.push(make_ref(other_r.crate_name(), f, sp, DefKind::from(r.kind), r.is_macro_expansion(), sr, uk));
                            }
                        }
                    }
                }
            }
        }

        let cache = self.rename_cache();
        let name_chars = def.name.chars().count() as u32;
        let is_non_lib =
            def.reader_idx.and_then(|ri| self.target_kind_at(ri)).is_some_and(|tk| matches!(tk, TargetKind::Bin | TargetKind::Example | TargetKind::Test | TargetKind::Bench));
        if !is_fn_local_dup && !is_non_lib && matches!(def.kind, DefKind::Enum | DefKind::AssocFn) && !def.name.is_empty() {
            for pmf in &cache.proc_macro_files {
                let (Some(pm_ri), Some(pm_fid)) = (pmf.reader_idx, pmf.file_id) else { continue };
                if Some(pm_ri) == def.reader_idx
                    || !pmf.content.contains("quote")
                    || !pmf.content.contains(def.name)
                    || (def.kind == DefKind::AssocFn && has_self && !owner_short.is_empty() && !pmf.content.contains(owner_short))
                {
                    continue;
                }
                let pm_reader = &self.readers[pm_ri];
                let pm_file = pm_reader.get_file(pm_fid);
                let sr = self.source_roots.get(pm_ri).and_then(|r| r.as_deref()).or(self.workspace_root.as_deref());
                let uk = self.unit_keys.get(pm_ri).and_then(|u| u.as_deref());
                for (l_idx, ln) in pmf.content.lines().enumerate() {
                    let (code, ln_1) = (ln.split("//").next().unwrap_or(ln), (l_idx + 1) as u32);
                    for (b_pos, _) in code.match_indices(def.name) {
                        let end_b = b_pos + def.name.len();
                        if (b_pos > 0 && is_ident_byte(code.as_bytes()[b_pos - 1])) || (end_b < code.len() && is_ident_byte(code.as_bytes()[end_b])) {
                            continue;
                        }
                        let (before, after) = (code[..b_pos].trim_end(), code[end_b..].trim_start());
                        let pm_ok = if def.kind == DefKind::Enum {
                            before.ends_with("::") && after.starts_with("::") && pmf.content.contains("::error::")
                        } else {
                            (!has_self && !owner_short.is_empty() && before.ends_with(&format!("::{owner_short}::")) && after.starts_with('('))
                                || (has_self && after.starts_with("::<") && (before.ends_with('.') || before.ends_with("=>")))
                        };
                        let col_1 = ln[..b_pos].chars().count() as u32 + 1;
                        if pm_ok && seen_locs.insert((pm_file.to_string(), ln_1, col_1)) {
                            let sp = crate::format::Span::new(pm_fid, ln_1, col_1, ln_1, col_1 + name_chars);
                            out_refs.push(make_ref(pm_reader.crate_name(), pm_file, sp, def.kind, false, sr, uk));
                        }
                    }
                }
            }
        }

        if def.kind == DefKind::AssocFn && has_self && !owner_short.is_empty() {
            self.expand_assoc_fn_field_and_macro_dot_refs(def, owner_raw, owner_short, &cache, &mut seen_locs, out_refs);
        }

        if let Some(r_idx) = def.reader_idx.filter(|&ri| !self.is_reader_external(ri)) {
            let reader = &self.readers[r_idx];
            let source_root = self.source_roots.get(r_idx).and_then(|r| r.as_deref()).or(self.workspace_root.as_deref());
            let unit_key = self.unit_keys.get(r_idx).and_then(|u| u.as_deref());
            if !is_fn_local_dup {
                if let Some(fid) = reader.find_file_id(def.file) {
                    let r_defs = reader.definitions();
                    for &(_, sl, d_idx) in self.unit_match.defs_in_file_up_to(r_idx, fid, u32::MAX) {
                        if let Some(d) =
                            r_defs.get(d_idx as usize).filter(|d| sl != def.start_line && DefKind::from(d.kind) == def.kind && reader.get_str(d.path_offset()) == def.path)
                        {
                            let sp = d.span();
                            if seen_locs.insert((def.file.to_string(), sp.start_line(), sp.start_col())) {
                                out_refs.push(make_ref(reader.crate_name(), def.file, sp, def.kind, d.is_macro_expansion(), source_root, unit_key));
                            }
                        }
                    }
                }
                if def.kind == DefKind::Struct && self.read_source_file(def.file).is_some_and(|c| c.contains("macro_rules!") && c.contains(&format!("::<{}>", def.name))) {
                    if let (Some(fid), Some(fc)) = (reader.find_file_id(def.file), self.read_source_file(def.file)) {
                        let fls: Vec<_> = fc.lines().collect();
                        for (li, &ln) in fls.iter().enumerate() {
                            for (bp, _) in ln.match_indices(def.name) {
                                let eb = bp + def.name.len();
                                if (bp == 0 || !is_ident_byte(ln.as_bytes()[bp - 1]))
                                    && (eb >= ln.len() || !is_ident_byte(ln.as_bytes()[eb]))
                                    && !is_in_string_or_comment(&fls, li, bp)
                                {
                                    let (l1, c1) = ((li + 1) as u32, ln[..bp].chars().count() as u32 + 1);
                                    if seen_locs.insert((def.file.to_string(), l1, c1)) {
                                        out_refs.push(make_ref(
                                            reader.crate_name(),
                                            def.file,
                                            crate::format::Span::new(fid, l1, c1, l1, c1 + name_chars),
                                            def.kind,
                                            false,
                                            source_root,
                                            unit_key,
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if def.kind == DefKind::AssocFn && has_self && !owner_short.is_empty() {
                if let Some(blocks) = cache.macro_blocks_by_crate.get(reader.crate_name()) {
                    for &(b_ri, ref blk) in blocks {
                        if b_ri == r_idx && blk.idents.contains(owner_short) {
                            let m_file = reader.get_file(blk.file_id);
                            for dc in blk.dot_calls.iter().filter(|dc| dc.is_turbofish && dc.method == def.name) {
                                if seen_locs.insert((m_file.to_string(), dc.line_1based, dc.col_1based)) {
                                    out_refs.push(make_ref(
                                        reader.crate_name(),
                                        m_file,
                                        crate::format::Span::new(blk.file_id, dc.line_1based, dc.col_1based, dc.line_1based, dc.col_1based + name_chars),
                                        def.kind,
                                        false,
                                        source_root,
                                        unit_key,
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }

        if def.kind == DefKind::Field && !def.name.is_empty() {
            self.expand_field_shorthand_refs(def, &mut seen_locs, out_refs);
            return;
        }

        if !matches!(def.kind, DefKind::Enum | DefKind::Struct | DefKind::Union | DefKind::Trait | DefKind::Mod) || def.name.is_empty() {
            return;
        }
        self.expand_rename_type_prefixes_and_cfg(def, fn_bounds, is_fn_local_dup, &mut seen_locs, out_refs);
    }

    fn expand_assoc_fn_field_and_macro_dot_refs<'a>(
        &'a self,
        def: &ResolvedDefinition<'a>,
        owner_raw: &str,
        owner_short: &str,
        cache: &crate::workspace::rename_cache::RenameWorkspaceCache,
        seen_locs: &mut HashSet<(String, u32, u32)>,
        out_refs: &mut Vec<WorkspaceReference<'a>>,
    ) {
        let def_crate = def.reader_idx.map(|ri| self.readers[ri].crate_name()).unwrap_or_else(|| super::qualified_path::extract_root_crate(def.path));
        let owner_local = owner_raw.strip_prefix(&format!("{def_crate}::")).unwrap_or(owner_raw);
        let owner_global = format!("{def_crate}::{owner_local}");
        let (owner_ty_pat, owner_mod_pat, dot_pat, name_chars) =
            (format!("::{owner_short}<"), format!("::{owner_short}::"), format!(".{}(", def.name), def.name.chars().count() as u32);
        let source_readers: Vec<usize> = def.reader_idx.map_or_else(|| self.readers_for_crate_fast(def_crate).into_owned(), |ri| vec![ri]);

        for (i, other_r) in self.readers.iter().enumerate() {
            if Some(i) == def.reader_idx || self.is_reader_external(i) || (!source_readers.is_empty() && !self.is_reader_in_dep_graph(&source_readers, i)) {
                continue;
            }
            let sr = self.source_roots.get(i).and_then(|r| r.as_deref()).or(self.workspace_root.as_deref());
            let uk = self.unit_keys.get(i).and_then(|u| u.as_deref());
            let mut m_fids: Vec<u32> = other_r.find_references(&owner_global).into_iter().chain(other_r.find_references(owner_local)).map(|r| r.span().file_id()).collect();
            m_fids.sort_unstable();
            m_fids.dedup();
            for m_fid in m_fids {
                let m_file = other_r.get_file(m_fid);
                let Some(mc) = self.read_source_file(m_file).filter(|c| c.contains(&owner_ty_pat) && c.contains(&dot_pat)) else { continue };
                let mc_lines: Vec<&str> = mc.lines().collect();
                for (f_idx, &fln) in mc_lines.iter().enumerate().filter(|(_, l)| l.contains(&owner_ty_pat) && l.contains(':')) {
                    let fld = fln.split(':').next().unwrap_or("").trim();
                    if !fld.is_empty() && fld.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                        let call_pat = format!("self.{fld}.{}(", def.name);
                        for (l_idx, &ln) in mc_lines.iter().enumerate().take((f_idx + 20).min(mc_lines.len())).skip(f_idx + 1).filter(|(_, l)| l.contains(&call_pat)) {
                            let ln_1 = (l_idx + 1) as u32;
                            for (b_pos, _) in ln.match_indices(&call_pat) {
                                let col_1 = ln[..b_pos + 6 + fld.len()].chars().count() as u32 + 1;
                                if seen_locs.insert((m_file.to_string(), ln_1, col_1)) {
                                    out_refs.push(WorkspaceReference {
                                        crate_name: other_r.crate_name(),
                                        file: m_file,
                                        span: crate::format::Span::new(m_fid, ln_1, col_1, ln_1, col_1 + name_chars),
                                        kind: def.kind,
                                        is_write: false,
                                        is_macro_expansion: false,
                                        source_root: sr,
                                        unit_key: uk,
                                    });
                                }
                            }
                        }
                    }
                }
            }
            if let Some(blocks) = cache.macro_blocks_by_crate.get(other_r.crate_name()) {
                for &(b_ri, ref blk) in blocks {
                    if b_ri != i || blk.file_path == def.file || !blk.body.contains(&owner_mod_pat) || !blk.body.contains(&dot_pat) {
                        continue;
                    }
                    let m_file = other_r.get_file(blk.file_id);
                    let mc_lines: Vec<&str> = blk.body.lines().collect();
                    for (k, &ln) in mc_lines.iter().enumerate() {
                        if k > 0 && ln.trim_start().starts_with(&dot_pat) && mc_lines[k - 1].contains(&owner_mod_pat) && mc_lines[k - 1].trim_end().ends_with(')') {
                            let ln_1 = blk.start_line_1based + k as u32;
                            let col_1 = ln.find(&dot_pat).map(|b| ln[..b + 1].chars().count() as u32 + 1).unwrap_or(0);
                            if col_1 > 0 && seen_locs.insert((m_file.to_string(), ln_1, col_1)) {
                                out_refs.push(WorkspaceReference {
                                    crate_name: other_r.crate_name(),
                                    file: m_file,
                                    span: crate::format::Span::new(blk.file_id, ln_1, col_1, ln_1, col_1 + name_chars),
                                    kind: def.kind,
                                    is_write: false,
                                    is_macro_expansion: false,
                                    source_root: sr,
                                    unit_key: uk,
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    fn expand_field_shorthand_refs<'a>(&'a self, def: &ResolvedDefinition<'a>, seen_locs: &mut HashSet<(String, u32, u32)>, out_refs: &mut Vec<WorkspaceReference<'a>>) {
        let Some((parent_raw, _)) = def.path.rsplit_once("::") else { return };
        let parent_type = parent_raw.rsplit("::").next().unwrap_or("");
        if parent_type.is_empty() {
            return;
        }
        let def_crate = def.reader_idx.map(|ri| self.readers[ri].crate_name()).unwrap_or_else(|| super::qualified_path::extract_root_crate(def.path));
        let parent_local = parent_raw.strip_prefix(&format!("{def_crate}::")).unwrap_or(parent_raw);
        let parent_global = format!("{def_crate}::{parent_local}");
        let source_readers: Vec<usize> = def.reader_idx.map_or_else(|| self.readers_for_crate_fast(def_crate).into_owned(), |ri| vec![ri]);

        for (i, reader) in self.readers.iter().enumerate().filter(|&(i, _)| !self.is_reader_external(i)) {
            let same_crate = reader.crate_name() == def_crate;
            if !same_crate && !source_readers.is_empty() && !self.is_reader_in_dep_graph(&source_readers, i) {
                continue;
            }
            let source_root = self.source_roots.get(i).and_then(|r| r.as_deref()).or(self.workspace_root.as_deref());
            let unit_key = self.unit_keys.get(i).and_then(|u| u.as_deref());
            let mut local_spans: Vec<crate::format::Span> =
                reader.find_references(def.name).into_iter().filter(|r| DefKind::from(r.kind) == DefKind::Local && !r.is_macro_expansion()).map(|r| r.span()).collect();

            let mut cand_fids: Vec<u32> = reader
                .find_references(&parent_global)
                .into_iter()
                .chain(same_crate.then(|| reader.find_references(parent_local)).into_iter().flatten())
                .map(|r| r.span().file_id())
                .chain(local_spans.iter().map(|s| s.file_id()))
                .chain(reader.find_file_id(def.file))
                .collect();
            cand_fids.sort_unstable();
            cand_fids.dedup();
            let defs = reader.definitions();
            for fid in cand_fids {
                for &(_, _, d_idx) in self.unit_match.defs_in_file_up_to(i, fid, u32::MAX) {
                    if let Some(d) =
                        defs.get(d_idx as usize).filter(|d| DefKind::from(d.kind) == DefKind::Local && !d.is_macro_expansion() && reader.get_str(d.name_offset()) == def.name)
                    {
                        local_spans.push(d.span());
                    }
                }
            }

            for sp in local_spans {
                let file = reader.get_file(sp.file_id());
                if seen_locs.contains(&(file.to_string(), sp.start_line(), sp.start_col())) {
                    continue;
                }
                let Some(content) = self.read_source_file(file) else { continue };
                let (lines, l_idx) = (content.lines().collect::<Vec<&str>>(), sp.start_line().saturating_sub(1) as usize);
                let Some(&line) = lines.get(l_idx) else { continue };
                let sb: usize = line.chars().take(sp.start_col().saturating_sub(1) as usize).map(char::len_utf8).sum();
                let eb = sb + def.name.len();
                let (before, after) = (line.get(..sb).unwrap_or("").trim_end(), line.get(eb..).unwrap_or("").trim_start());
                if line.get(sb..eb) != Some(def.name)
                    || !(before.ends_with('{') || before.ends_with(','))
                    || !(after.starts_with('}') || after.starts_with(',') || after.is_empty())
                    || after.starts_with(':')
                {
                    continue;
                }
                let mut joined = lines[l_idx.saturating_sub(4)..l_idx].iter().map(|l| l.split("//").next().unwrap_or(l)).collect::<Vec<_>>().join(" ");
                joined.push(' ');
                joined.push_str(&line[..sb]);
                if joined.rsplit_once('{').is_some_and(|(head, _)| head.trim_end().ends_with(parent_type) || (file == def.file && head.trim_end().ends_with("Self"))) {
                    seen_locs.insert((file.to_string(), sp.start_line(), sp.start_col()));
                    out_refs.push(WorkspaceReference {
                        crate_name: reader.crate_name(),
                        file,
                        span: sp,
                        kind: DefKind::Field,
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
