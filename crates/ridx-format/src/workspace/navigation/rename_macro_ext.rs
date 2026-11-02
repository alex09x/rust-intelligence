/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Macro-based trait reference expansion and `#[cfg]` / `Type::assoc` prefix expansion for `textDocument/rename`.

use super::decl_lookup::{collect_enum_variant_idents_from_source, pm_declares_derive_for_trait};
use super::*;
use crate::workspace::cursor_path::{is_ident_byte, is_in_string_or_comment};
use crate::workspace::rename_parse::{extract_macro_rules_blocks, has_self_receiver};
use crate::workspace::TargetKind;
use std::collections::HashSet;

fn has_word(s: &str, w: &str) -> bool {
    !w.is_empty() && s.match_indices(w).any(|(b, _)| (b == 0 || !is_ident_byte(s.as_bytes()[b - 1])) && (b + w.len() >= s.len() || !is_ident_byte(s.as_bytes()[b + w.len()])))
}

impl WorkspaceIndex {
    pub(super) fn expand_trait_macro_rules_refs<'a>(
        &'a self,
        def: &ResolvedDefinition<'a>,
        item_name: &'a str,
        local_trait: &str,
        macro_files: &[(usize, u32, u32)],
        seen_locs: &mut HashSet<(String, u32, u32)>,
        out_refs: &mut Vec<WorkspaceReference<'a>>,
    ) {
        if !matches!(def.kind, DefKind::AssocTy | DefKind::AssocFn) || item_name.is_empty() {
            return;
        }
        let push_ref = |r_idx: usize, fid: u32, sp: crate::format::Span, is_mac: bool, seen: &mut HashSet<(String, u32, u32)>, out: &mut Vec<WorkspaceReference<'a>>| {
            let (reader, file) = (&self.readers[r_idx], self.readers[r_idx].get_file(fid));
            if seen.insert((file.to_string(), sp.start_line(), sp.start_col())) {
                let (source_root, unit_key) =
                    (self.source_roots.get(r_idx).and_then(|r| r.as_deref()).or(self.workspace_root.as_deref()), self.unit_keys.get(r_idx).and_then(|u| u.as_deref()));
                out.push(WorkspaceReference {
                    crate_name: reader.crate_name(),
                    file,
                    span: sp,
                    kind: def.kind,
                    is_write: false,
                    is_macro_expansion: is_mac,
                    source_root,
                    unit_key,
                });
            }
        };

        let cache = self.rename_cache();
        let trait_short = local_trait.rsplit("::").next().unwrap_or(local_trait);
        let name_chars = item_name.chars().count() as u32;
        let def_has_self = def.signature.is_some_and(has_self_receiver);

        let is_non_lib =
            def.reader_idx.and_then(|ri| self.target_kind_at(ri)).is_some_and(|tk| matches!(tk, TargetKind::Bin | TargetKind::Example | TargetKind::Test | TargetKind::Bench));
        let matching_pm_crates: HashSet<&str> = if is_non_lib {
            HashSet::new()
        } else {
            cache.proc_macro_files.iter().filter(|pmf| pm_declares_derive_for_trait(&pmf.content, trait_short)).map(|pmf| pmf.crate_name.as_str()).collect()
        };

        if !matching_pm_crates.is_empty() {
            for pmf in cache.proc_macro_files.iter().filter(|pmf| matching_pm_crates.contains(pmf.crate_name.as_str())) {
                let (Some(pm_ri), Some(pm_fid)) = (pmf.reader_idx, pmf.file_id) else { continue };
                if !pmf.content.contains("quote") || !pmf.content.contains(trait_short) || !pmf.content.contains(item_name) {
                    continue;
                }
                for (l_idx, ln) in pmf.content.lines().enumerate() {
                    let (code, ln_1) = (ln.split("//").next().unwrap_or(ln), (l_idx + 1) as u32);
                    for (b_pos, _) in code.match_indices(item_name) {
                        let end_b = b_pos + item_name.len();
                        if (b_pos > 0 && is_ident_byte(code.as_bytes()[b_pos - 1])) || (end_b < code.len() && is_ident_byte(code.as_bytes()[end_b])) {
                            continue;
                        }
                        let (before, after) = (code[..b_pos].trim_end(), code[end_b..].trim_start());
                        let pm_matched = if def.kind == DefKind::AssocFn {
                            (before.ends_with("fn") && (after.starts_with('(') || after.starts_with('<')) && pmf.content.contains(&format!("{trait_short} for ")))
                                || ((before.ends_with(">::") && before.contains(trait_short) || before.ends_with(&format!("::{trait_short}::")))
                                    && (after.starts_with('(') || after.starts_with("::<")))
                        } else {
                            (before.ends_with("type") && (after.starts_with('=') || after.starts_with('<') || after.starts_with(':')))
                                || (before.ends_with("Self::") && pmf.content.contains(&format!("type {item_name}")))
                                || (before.ends_with(">::") && before.contains(trait_short))
                        };
                        if pm_matched {
                            let col_1 = ln[..b_pos].chars().count() as u32 + 1;
                            push_ref(pm_ri, pm_fid, crate::format::Span::new(pm_fid, ln_1, col_1, ln_1, col_1 + name_chars), false, seen_locs, out_refs);
                        }
                    }
                }
            }
        }

        let def_file_has_macro = self.read_source_file(def.file).is_some_and(|c| c.contains("macro_rules!") && c.contains(item_name));
        let def_cname = def.reader_idx.map(|ri| self.readers[ri].crate_name()).unwrap_or("");
        let crate_macro_has_item = !trait_short.is_empty()
            && cache.macro_blocks_by_crate.get(def_cname).is_some_and(|blks| blks.iter().any(|(_, b)| b.idents.contains(trait_short) && b.idents.contains(item_name)));

        if macro_files.is_empty() && !def_file_has_macro && !crate_macro_has_item && !out_refs.iter().any(|r| r.is_macro_expansion && r.span.start_line() > 0) {
            return;
        }

        let def_is_lib_unit = def.reader_idx.and_then(|ri| self.target_kinds.get(ri).and_then(|tk| *tk)).is_none_or(|tk| tk == TargetKind::Lib);
        let mut candidate_files: Vec<(usize, u32)> = macro_files.iter().map(|&(i, f, _)| (i, f)).collect();
        for (i, reader) in self.readers.iter().enumerate().filter(|&(i, _)| !self.is_reader_external(i)) {
            candidate_files.extend(std::iter::once(def.file).chain(out_refs.iter().map(|r| r.file)).filter_map(|f| self.reader_find_file_id(i, f).map(|fid| (i, fid))));
            if def_is_lib_unit {
                if let Some(blks) = cache.macro_blocks_by_crate.get(reader.crate_name()) {
                    candidate_files.extend(blks.iter().filter(|&&(ri, ref b)| ri == i && b.idents.contains(item_name)).map(|&(ri, ref b)| (ri, b.file_id)));
                }
            }
        }
        candidate_files.sort_unstable();
        candidate_files.dedup();

        let mod_prefix = local_trait.rsplit_once("::").map(|(m, _)| m).unwrap_or("");
        let initial_files: HashSet<&str> = out_refs.iter().map(|r| r.file).collect();
        for (r_idx, fid) in candidate_files {
            let file = self.readers[r_idx].get_file(fid);
            let file_had_initial_refs = initial_files.contains(file);
            let Some(content) = self.read_source_file(file).filter(|c| c.contains("macro_rules!") && c.contains(item_name)) else { continue };
            let file_mentions_trait = has_word(&content, trait_short);
            let blocks = extract_macro_rules_blocks(fid, file, &content);
            let mut any_macro_mentioned_trait = false;

            for blk in &blocks {
                let macro_lines: Vec<&str> = blk.body.lines().collect();
                let macro_has_impl = blk.has_impl_kw;
                let macro_mentions_trait = !trait_short.is_empty() && blk.idents.contains(trait_short);
                let macro_defines_assoc_ty = def.kind == DefKind::AssocTy && blk.body.contains(&format!("type {item_name}"));
                let macro_defines_fn = def.kind == DefKind::AssocFn && macro_lines.iter().any(|l| l.contains("fn ") && has_word(l, item_name));
                any_macro_mentioned_trait |= macro_mentions_trait;
                let declares_other_mod_trait = !mod_prefix.is_empty()
                    && macro_lines.iter().any(|l| l.contains(&format!("trait {trait_short}")) || (!macro_mentions_trait && l.contains("trait $")))
                    && !file.contains(&format!("/{mod_prefix}/"))
                    && !file.ends_with(&format!("/{mod_prefix}.rs"));
                if declares_other_mod_trait || (def.kind == DefKind::AssocTy && !macro_mentions_trait && (macro_has_impl || !any_macro_mentioned_trait)) {
                    continue;
                }
                for (off, &line) in macro_lines.iter().enumerate() {
                    let (code, ln_1) = (line.split("//").next().unwrap_or(line), blk.start_line_1based + off as u32);
                    for (b_pos, _) in code.match_indices(item_name) {
                        let end_b = b_pos + item_name.len();
                        if (b_pos > 0 && is_ident_byte(code.as_bytes()[b_pos - 1])) || (end_b < code.len() && is_ident_byte(code.as_bytes()[end_b])) {
                            continue;
                        }
                        let (before, after) = (code[..b_pos].trim_end(), code[end_b..].trim_start());
                        let matched = match def.kind {
                            DefKind::AssocTy => {
                                (before.ends_with("type") && (after.starts_with('=') || after.starts_with(':')))
                                    || (before.ends_with(">::") && before.contains(" as ") && !trait_short.is_empty() && has_word(before, trait_short))
                                    || (!before.ends_with(">::")
                                        && before.strip_suffix("::").is_some_and(|p| {
                                            p.rsplit(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '$').next().is_some_and(|t| {
                                                (t.starts_with('$') && (macro_mentions_trait || file_mentions_trait))
                                                    || (t == "Self"
                                                        && (macro_defines_assoc_ty || (Some(r_idx) == def.reader_idx && content.contains(&format!("type {item_name}")))))
                                            })
                                        }))
                            }
                            DefKind::AssocFn => {
                                ((!macro_has_impl || macro_lines.iter().any(|l| l.contains("impl") && has_word(l, trait_short)))
                                    && before.ends_with("fn")
                                    && (after.starts_with('(') || after.starts_with('<')))
                                    || (def_has_self
                                        && before.ends_with('.')
                                        && (!before.ends_with("self.") || (!macro_has_impl && macro_defines_fn))
                                        && after.starts_with('(')
                                        && !trait_short.is_empty()
                                        && (macro_mentions_trait
                                            || (!macro_has_impl && macro_defines_fn)
                                            || ((!file_had_initial_refs || Some(r_idx) != def.reader_idx) && file_mentions_trait)))
                                    || ((!trait_short.is_empty() && before.ends_with(&format!("{trait_short}::"))
                                        || (macro_mentions_trait && before.ends_with(">::"))
                                        || (!trait_short.is_empty()
                                            && (macro_mentions_trait || ((!file_had_initial_refs || before.ends_with(":)+")) && file_mentions_trait))
                                            && ((before.ends_with("::") && before.contains('$')) || before.ends_with(":)+"))))
                                        && (after.starts_with('(') || after.starts_with("::<")))
                                    || (before.ends_with('{') && after.starts_with('<'))
                            }
                            _ => false,
                        };
                        if matched {
                            let col_1 = line[..b_pos].chars().count() as u32 + 1;
                            push_ref(r_idx, fid, crate::format::Span::new(fid, ln_1, col_1, ln_1, col_1 + name_chars), false, seen_locs, out_refs);
                            if before.ends_with(":)+") {
                                let r = &self.readers[r_idx];
                                for d in r.definitions().iter().filter(|d| DefKind::from(d.kind) == def.kind && r.get_str(d.name_offset()) == item_name) {
                                    let dp = r.get_str(d.path_offset());
                                    if !dp.starts_with('<')
                                        && !dp
                                            .rsplit_once("::")
                                            .is_some_and(|(ow, _)| r.definitions().iter().any(|td| DefKind::from(td.kind) == DefKind::Trait && r.get_str(td.path_offset()) == ow))
                                    {
                                        continue;
                                    }
                                    push_ref(r_idx, d.span().file_id(), d.span(), d.is_macro_expansion(), seen_locs, out_refs);
                                    if let Some((sib_p, _)) = dp.rsplit_once("::") {
                                        let sib_short = sib_p.rsplit("::").next().unwrap_or(sib_p);
                                        if sib_short != trait_short && !sib_p.starts_with('<') {
                                            let sib_local = sib_p.strip_prefix(&format!("{}::", r.crate_name())).unwrap_or(sib_p);
                                            self.expand_trait_qualified_refs(def, item_name, sib_local, sib_local, sib_p, &[sib_short.to_string()], seen_locs, out_refs);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    pub(super) fn expand_rename_type_prefixes_and_cfg<'a>(
        &'a self,
        def: &ResolvedDefinition<'a>,
        fn_bounds: Option<(u32, u32)>,
        is_fn_local_dup: bool,
        seen_locs: &mut HashSet<(String, u32, u32)>,
        out_refs: &mut Vec<WorkspaceReference<'a>>,
    ) {
        let make_ref =
            |crate_name: &'a str, file: &'a str, span: crate::format::Span, kind: DefKind, is_macro_expansion: bool, source_root: Option<&'a Path>, unit_key: Option<&'a str>| {
                WorkspaceReference { crate_name, file, span, kind, is_write: false, is_macro_expansion, source_root, unit_key }
            };
        let target_crate = def.reader_idx.map(|ri| self.readers[ri].crate_name()).unwrap_or_else(|| super::qualified_path::extract_root_crate(def.path));
        let local_path = def.path.strip_prefix(&format!("{target_crate}::")).unwrap_or(def.path);
        let global_path = format!("{target_crate}::{local_path}");
        let (local_prefix, global_prefix, name_colon, name_chars) =
            (format!("{local_path}::"), format!("{global_path}::"), format!("{}::", def.name), def.name.chars().count() as u32);

        let mut child_tails: HashSet<String> = HashSet::new();
        let def_readers = self.readers_for_crate_fast(target_crate);
        let single_reader_only = def.reader_idx.filter(|&ri| {
            fn_bounds.is_some() || self.target_kind_at(ri).is_some_and(|tk| matches!(tk, TargetKind::Bin | TargetKind::Example | TargetKind::Test | TargetKind::Bench))
        });
        let owner_scan_readers: Vec<usize> = match single_reader_only {
            Some(ri) => vec![ri],
            None => def_readers.iter().copied().chain(def.reader_idx).collect(),
        };
        for &ri in &owner_scan_readers {
            let Some(r) = self.readers.get(ri) else { continue };
            for d in r.find_definitions_by_owner(local_path).into_iter().chain(r.find_definitions_by_owner(&global_path)) {
                if DefKind::from(d.kind) != DefKind::Field {
                    let name = r.get_str(d.name_offset());
                    if !name.is_empty() && !name.contains("::") {
                        child_tails.insert(name.to_string());
                    }
                }
            }
            if child_tails.is_empty() && def.kind == DefKind::Mod {
                for d in r.definitions() {
                    let dp = r.get_str(d.path_offset());
                    if let Some(tail) = dp.strip_prefix(&local_prefix).or_else(|| dp.strip_prefix(&global_prefix)).filter(|t| !t.is_empty() && !t.contains("::")) {
                        child_tails.insert(tail.to_string());
                    }
                }
            }
        }
        if def.kind == DefKind::Enum {
            if let Some(def_src) = self.read_source_file(def.file) {
                collect_enum_variant_idents_from_source(&def_src, def.name, def.start_line, &mut child_tails);
            }
        }

        if !child_tails.is_empty() {
            let cand_pairs: Vec<(String, String)> = child_tails.iter().map(|t| (format!("{global_path}::{t}"), format!("{local_path}::{t}"))).collect();
            let source_readers: Vec<usize> = def.reader_idx.map_or_else(|| def_readers.into_owned(), |ri| vec![ri]);
            let reader_indices: Vec<usize> = match single_reader_only {
                Some(ri) => vec![ri],
                None => (0..self.readers.len()).filter(|&i| !self.is_reader_external(i)).collect(),
            };
            for i in reader_indices {
                let reader = &self.readers[i];
                let same_crate = reader.crate_name() == target_crate;
                if !same_crate && !source_readers.is_empty() && !self.is_reader_in_dep_graph(&source_readers, i) {
                    continue;
                }
                let source_root = self.source_roots.get(i).and_then(|r| r.as_deref()).or(self.workspace_root.as_deref());
                let unit_key = self.unit_keys.get(i).and_then(|u| u.as_deref());
                for (g_cand, l_cand) in &cand_pairs {
                    for r in reader.find_references(g_cand).into_iter().chain(same_crate.then(|| reader.find_references(l_cand)).into_iter().flatten()) {
                        if r.is_macro_expansion() {
                            continue;
                        }
                        let (sp, file) = (r.span(), reader.get_file(r.span().file_id()));
                        if fn_bounds.is_some_and(|(fs, fe)| file != def.file || sp.start_line() < fs || sp.start_line() > fe) {
                            continue;
                        }
                        let Some(content) = self.read_source_file(file) else { continue };
                        let Some(line) = content.lines().nth(sp.start_line().saturating_sub(1) as usize) else { continue };
                        let (chars, sc_idx, prefix_len) = (line.chars().collect::<Vec<char>>(), sp.start_col().saturating_sub(1) as usize, name_chars as usize + 2);
                        let prefix_col = if sc_idx + prefix_len <= chars.len() && chars[sc_idx..sc_idx + prefix_len].iter().collect::<String>() == name_colon {
                            Some(sp.start_col())
                        } else if sc_idx >= prefix_len && chars[sc_idx - prefix_len..sc_idx].iter().collect::<String>() == name_colon {
                            Some(sp.start_col() - prefix_len as u32)
                        } else {
                            None
                        };
                        if let Some(p_col) = prefix_col {
                            if seen_locs.insert((file.to_string(), sp.start_line(), p_col)) {
                                let span = crate::format::Span::new(sp.file_id(), sp.start_line(), p_col, sp.start_line(), p_col + name_chars);
                                out_refs.push(make_ref(reader.crate_name(), file, span, def.kind, false, source_root, unit_key));
                            }
                        }
                    }
                }
            }
        }

        if !is_fn_local_dup && matches!(def.kind, DefKind::Struct | DefKind::Enum | DefKind::Union | DefKind::Trait) {
            let ref_files: Vec<_> = out_refs.iter().map(|r| (r.file, r.span.file_id(), r.crate_name, r.source_root, r.unit_key)).collect();
            let mut visited_cfg_files = HashSet::new();
            for (file, fid, cname, sr, uk) in ref_files {
                if !visited_cfg_files.insert(file) {
                    continue;
                }
                let Some(content) = self.read_source_file(file) else { continue };
                let has_local_type_alias = file != def.file && content.contains(&format!("type {} =", def.name));
                let has_cfg = !has_local_type_alias && (content.contains("#[cfg(") || content.contains("cfg_")) && (content.contains("use ") || file == def.file);
                let multi_def_same_file = file == def.file && content.matches(&format!("struct {}", def.name)).count() > 1;
                let (qual_macro_pat, bound_macro_pat, turbo_macro_pat, owner_colon_pat) =
                    (format!("::{}<", def.name), format!(": {}>", def.name), format!("::<{}>", def.name), format!("{}::", def.name));
                let file_defines_other_enum = file != def.file
                    && self.readers_for_crate_fast(cname).iter().any(|&ri| {
                        let r = &self.readers[ri];
                        r.definitions().iter().any(|d| {
                            d.span().file_id() == fid
                                && DefKind::from(d.kind) == DefKind::Enum
                                && r.get_str(d.name_offset()) == def.name
                                && r.get_str(d.path_offset()).strip_prefix(&format!("{cname}::")).unwrap_or_else(|| r.get_str(d.path_offset())) != local_path
                        })
                    });
                let has_owner_colon = has_cfg
                    && def.kind == DefKind::Enum
                    && !file_defines_other_enum
                    && content.matches(&format!("enum {}", def.name)).count() <= 2
                    && content.contains(&owner_colon_pat);
                let has_macro_qual = content.contains("macro_rules!")
                    && (content.contains(&qual_macro_pat) || (file == def.file && (content.contains(&bound_macro_pat) || content.contains(&turbo_macro_pat))));
                if !has_cfg && !has_macro_qual && !has_owner_colon {
                    continue;
                }
                let lines: Vec<&str> = content.lines().collect();
                for (l_idx, &ln) in lines.iter().enumerate() {
                    let strict_near_cfg = has_cfg
                        && ((l_idx.saturating_sub(6)..=l_idx).any(|k| lines[k].contains("#[cfg(") || lines[k].contains("cfg_"))
                            || (multi_def_same_file && (ln.contains("impl ") || ln.contains("-> ") || ln.contains(&format!("{} {{", def.name)))));
                    let near_cfg = strict_near_cfg || (has_owner_colon && ln.contains(&owner_colon_pat));
                    let on_macro_qual = (has_macro_qual
                        && (ln.contains(&qual_macro_pat) || (file == def.file && (ln.contains(&bound_macro_pat) || ln.contains(&turbo_macro_pat)))))
                        || (has_owner_colon && ln.contains(&format!("super::{owner_colon_pat}")));
                    if !near_cfg && !on_macro_qual {
                        continue;
                    }
                    for (b_pos, _) in ln.match_indices(def.name) {
                        let end_b = b_pos + def.name.len();
                        let bnd_ok = (b_pos == 0 || !is_ident_byte(ln.as_bytes()[b_pos - 1])) && (end_b >= ln.len() || !is_ident_byte(ln.as_bytes()[end_b]));
                        if !bnd_ok || is_in_string_or_comment(&lines, l_idx, b_pos) {
                            continue;
                        }
                        let (before, after) = (ln[..b_pos].trim_end(), ln[end_b..].trim_start());
                        if before.ends_with('.')
                            || (before.ends_with("::") && !on_macro_qual)
                            || (!strict_near_cfg && !on_macro_qual && !after.starts_with("::"))
                            || after.starts_with('!')
                            || (after.starts_with(':') && !after.starts_with("::"))
                            || ["struct", "enum", "trait", "type", "fn", "let", "mod", "for"].iter().any(|kw| before.ends_with(kw))
                        {
                            continue;
                        }
                        let (ln_1, col_1) = ((l_idx + 1) as u32, ln[..b_pos].chars().count() as u32 + 1);
                        if seen_locs.insert((file.to_string(), ln_1, col_1)) {
                            out_refs.push(make_ref(cname, file, crate::format::Span::new(fid, ln_1, col_1, ln_1, col_1 + name_chars), def.kind, false, sr, uk));
                        }
                    }
                }
            }
        }
    }
}
