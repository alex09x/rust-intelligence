/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Trait-item declaration lookup for `textDocument/declaration` and macro-based trait rename expansion.

use super::impl_parse::parse_impl_def_path;
use super::qualified_path::{extract_impl_trait, extract_root_crate};
use super::trait_impl_refs::{path_matches_trait, strip_generics};
use super::*;
use std::collections::HashSet;

impl WorkspaceIndex {
    /// Resolves the trait declaration for an associated item defined inside a trait `impl`.
    pub fn find_trait_item_declaration<'a>(&'a self, def: &ResolvedDefinition<'a>) -> Option<ResolvedDefinition<'a>> {
        if !matches!(def.kind, DefKind::AssocFn | DefKind::AssocConst | DefKind::AssocTy) || def.name.is_empty() {
            return None;
        }
        let trait_str = extract_impl_trait(def.path).or_else(|| self.find_enclosing_impl_trait_path(def))?;
        let raw_trait = strip_generics(trait_str);
        if raw_trait.is_empty() {
            return None;
        }

        let mut cands = std_map::std_path_candidates(raw_trait);
        if let Some(rest) = raw_trait.strip_prefix("core::") {
            cands.extend([format!("std::{rest}"), format!("alloc::{rest}")]);
        } else if let Some(rest) = raw_trait.strip_prefix("alloc::") {
            cands.extend([format!("std::{rest}"), format!("core::{rest}")]);
        }
        if let Some(r_idx) = def.reader_idx {
            let def_crate = self.readers[r_idx].crate_name();
            if !def_crate.is_empty() && !raw_trait.starts_with(&format!("{def_crate}::")) {
                cands.push(format!("{def_crate}::{raw_trait}"));
            }
        }

        let mut checked = HashSet::new();
        for idx in def.reader_idx.into_iter().chain(cands.iter().flat_map(|tp| self.readers_for_crate_fast(extract_root_crate(tp)).into_owned())).chain(0..self.readers.len()) {
            if checked.insert(idx) {
                if let Some(res) = self.search_trait_item_decl_in_reader(idx, def, raw_trait, &cands) {
                    return Some(res);
                }
            }
        }
        None
    }

    pub(super) fn find_enclosing_impl_trait_path<'a>(&'a self, def: &ResolvedDefinition<'a>) -> Option<&'a str> {
        let r = self.readers.get(def.reader_idx?)?;
        find_enclosing_impl_trait_at(r, r.find_file_id(def.file)?, def.start_line)
    }

    fn search_trait_item_decl_in_reader<'a>(&'a self, reader_idx: usize, def: &ResolvedDefinition<'a>, raw_trait: &str, cands: &[String]) -> Option<ResolvedDefinition<'a>> {
        let (r, cname) = (&self.readers[reader_idx], self.readers[reader_idx].crate_name());
        let is_valid_item = |d: &DefEntry| -> bool {
            DefKind::from(d.kind) == def.kind
                && !d.is_reexport()
                && r.get_str(d.name_offset()) == def.name
                && r.get_file(d.span().file_id()) != "external"
                && extract_impl_trait(r.get_str(d.path_offset())).is_none()
        };
        let mut matches: Vec<&DefEntry> = Vec::new();
        let push_unique = |matches: &mut Vec<&'a DefEntry>, d: &'a DefEntry| {
            if is_valid_item(d) && !matches.iter().any(|existing| std::ptr::eq(*existing, d)) {
                matches.push(d);
            }
        };
        for tp in cands {
            let local_tp = tp.strip_prefix(&format!("{cname}::")).unwrap_or(tp);
            let global_tp = format!("{cname}::{local_tp}");
            for owner_q in [local_tp, global_tp.as_str(), tp.as_str()] {
                r.find_definitions_by_owner(owner_q).into_iter().for_each(|d| push_unique(&mut matches, d));
            }
            for path_q in [format!("{local_tp}::{}", def.name), format!("{global_tp}::{}", def.name)] {
                if let Some(d) = r.find_definition_by_path(&path_q) {
                    push_unique(&mut matches, d);
                }
            }
        }
        if matches.is_empty() && (Some(reader_idx) == def.reader_idx || cands.iter().any(|tp| extract_root_crate(tp) == cname)) {
            let (local_raw, global_raw) =
                (raw_trait.strip_prefix(&format!("{cname}::")).unwrap_or(raw_trait), format!("{cname}::{}", raw_trait.strip_prefix(&format!("{cname}::")).unwrap_or(raw_trait)));
            for td in r.definitions().iter().filter(|td| DefKind::from(td.kind) == DefKind::Trait) {
                let dp = r.get_str(td.path_offset());
                if path_matches_trait(dp, raw_trait, local_raw, &global_raw, cands) {
                    for owner_q in [dp, dp.strip_prefix(&format!("{cname}::")).unwrap_or(dp)] {
                        r.find_definitions_by_owner(owner_q).into_iter().for_each(|d| push_unique(&mut matches, d));
                    }
                }
            }
        }
        let best = matches
            .iter()
            .copied()
            .filter(|d| Some(reader_idx) == def.reader_idx && r.get_file(d.span().file_id()) == def.file && d.span().start_line() <= def.start_line)
            .max_by_key(|d| d.span().start_line())
            .or_else(|| matches.first().copied())?;
        Some(self.make_resolved_definition(reader_idx, best))
    }

    pub fn external_trait_for_def(&self, def: &ResolvedDefinition<'_>) -> Option<String> {
        self.external_trait_for_target(def.kind, def.path, def.reader_idx, def.file, def.start_line, &def.resolved_path())
    }

    pub fn external_trait_for_impl_item(&self, loc_refs: &LocationReferences<'_>) -> Option<String> {
        self.external_trait_for_target(
            loc_refs.target_kind,
            loc_refs.target_path,
            loc_refs.target_reader_idx,
            loc_refs.target_file,
            loc_refs.target_line,
            &loc_refs.target_resolved_path(),
        )
    }

    fn external_trait_for_target(&self, kind: DefKind, path: &str, reader_idx: Option<usize>, file: &str, line: u32, resolved_p: &Path) -> Option<String> {
        if !matches!(kind, DefKind::AssocFn | DefKind::AssocTy | DefKind::AssocConst | DefKind::Field) {
            return None;
        }
        let check_trait_is_external = |tr: &str| -> Option<String> {
            let base_tr = strip_generics(tr);
            if base_tr.is_empty() {
                return None;
            }
            if matches!(extract_root_crate(base_tr), "core" | "alloc" | "std" | "proc_macro") {
                return Some(base_tr.to_string());
            }
            if self.resolve_workspace_trait_container(base_tr, reader_idx, true).is_some_and(|(t, _, _)| t.is_some_and(|i| !self.is_reader_external(i))) {
                return None;
            }
            let trait_short = base_tr.rsplit("::").next().unwrap_or(base_tr);
            (!self.rename_cache().ws_trait_short_names.contains(trait_short)).then(|| base_tr.to_string())
        };
        if kind != DefKind::Field {
            if let Some(tr) = extract_impl_trait(path) {
                return check_trait_is_external(tr);
            }
            if let Some((raw_parent, _)) = path.rsplit_once("::") {
                if self.resolve_workspace_trait_container(raw_parent, reader_idx, false).is_some_and(|(t, _, _)| t.is_some_and(|i| !self.is_reader_external(i))) {
                    return None;
                }
            }
            if let Some(ri) = reader_idx {
                if let Some(r) = self.readers.get(ri) {
                    if let Some(fid) = r.find_file_id(file) {
                        let defs = r.definitions();
                        let mut found_impl = false;
                        for &(_, _, d_idx) in self.unit_match.defs_in_file_up_to(ri, fid, line) {
                            if let Some(d) = defs.get(d_idx as usize) {
                                if DefKind::from(d.kind) == DefKind::Impl && d.span().start_line() <= line && line <= d.span().end_line() {
                                    found_impl = true;
                                    if let (Some(tr), _) = parse_impl_def_path(r.get_str(d.path_offset())) {
                                        let base_tr = strip_generics(tr);
                                        if !base_tr.contains("::") {
                                            for &(_, _, td_idx) in self.unit_match.defs_in_file_up_to(ri, fid, u32::MAX) {
                                                if let Some(td) =
                                                    defs.get(td_idx as usize).filter(|td| DefKind::from(td.kind) == DefKind::Trait && r.get_str(td.name_offset()) == base_tr)
                                                {
                                                    return check_trait_is_external(r.get_str(td.path_offset()));
                                                }
                                            }
                                        }
                                        return check_trait_is_external(tr);
                                    }
                                }
                            }
                        }
                        if found_impl {
                            return None;
                        }
                    }
                }
            }
        }
        let content = self.read_source_file(file).or_else(|| std::fs::read_to_string(resolved_p).ok())?;
        if !content.contains(" for ") {
            return None;
        }
        let (lines, idx) = (content.lines().collect::<Vec<&str>>(), line.saturating_sub(1) as usize);
        if kind == DefKind::Field && !lines.get(idx).is_some_and(|l| l.trim_start().starts_with("fn ")) {
            return None;
        }
        lines.get(idx).and_then(|_| find_enclosing_trait_in_impl_for(&lines, idx)).and_then(&check_trait_is_external)
    }
}

pub(super) fn pm_declares_derive_for_trait(content: &str, trait_short: &str) -> bool {
    use crate::workspace::cursor_path::is_ident_byte;
    !trait_short.is_empty()
        && content.contains("proc_macro_derive")
        && content.match_indices(&format!("proc_macro_derive({trait_short}")).any(|(pos, m)| pos + m.len() >= content.len() || !is_ident_byte(content.as_bytes()[pos + m.len()]))
}

pub(super) fn collect_enum_variant_idents_from_source(content: &str, enum_name: &str, start_line_1based: u32, out: &mut HashSet<String>) {
    use crate::workspace::cursor_path::is_ident_byte;
    let (mut depth, mut entered) = (0i32, false);
    for raw_ln in content.lines().skip(start_line_1based.saturating_sub(1) as usize) {
        let code = crate::workspace::rename_parse::strip_line_comment(raw_ln);
        for (i, ch) in code.char_indices() {
            match ch {
                '{' => {
                    depth += 1;
                    entered = true;
                }
                '}' => {
                    depth -= 1;
                    if entered && depth <= 0 {
                        return;
                    }
                }
                _ if entered && depth == 1 && (ch.is_ascii_alphabetic() || ch == '_') && (i == 0 || !is_ident_byte(code.as_bytes()[i - 1])) => {
                    let prev = code[..i].trim_end();
                    if prev.is_empty() || prev.ends_with('{') || prev.ends_with(',') || prev.ends_with(']') {
                        let len = code[i..].bytes().take_while(|&b| is_ident_byte(b)).count();
                        let ident = &code[i..i + len];
                        if ident != enum_name && !matches!(ident, "pub" | "crate" | "super") {
                            out.insert(ident.to_string());
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

pub(crate) fn reader_has_def_kind_at_path(r: &IndexReader<memmap2::Mmap>, path: &str, kind: DefKind) -> bool {
    let hash = crate::format::fnv1a_64(path.as_bytes());
    let defs_hash = r.defs_by_hash();
    let Ok(idx) = defs_hash.binary_search_by_key(&hash, |e| e.hash()) else { return false };
    let defs = r.definitions();
    let mut i = idx;
    while i > 0 && defs_hash[i - 1].hash() == hash {
        i -= 1;
    }
    while i < defs_hash.len() && defs_hash[i].hash() == hash {
        let d = &defs[defs_hash[i].index() as usize];
        if DefKind::from(d.kind) == kind && r.get_str(d.path_offset()) == path {
            return true;
        }
        i += 1;
    }
    false
}

pub(crate) fn reader_has_non_trait_type_at_path(r: &IndexReader<memmap2::Mmap>, path: &str) -> bool {
    let hash = crate::format::fnv1a_64(path.as_bytes());
    let defs_hash = r.defs_by_hash();
    let Ok(idx) = defs_hash.binary_search_by_key(&hash, |e| e.hash()) else { return false };
    let defs = r.definitions();
    let mut i = idx;
    while i > 0 && defs_hash[i - 1].hash() == hash {
        i -= 1;
    }
    while i < defs_hash.len() && defs_hash[i].hash() == hash {
        let d = &defs[defs_hash[i].index() as usize];
        if matches!(DefKind::from(d.kind), DefKind::Struct | DefKind::Enum | DefKind::Union) && r.get_str(d.path_offset()) == path {
            return true;
        }
        i += 1;
    }
    false
}

fn find_enclosing_trait_in_impl_for<'a>(lines: &[&'a str], line_idx: usize) -> Option<&'a str> {
    let mut brace_depth = 0i32;
    for l in (line_idx.saturating_sub(120)..=line_idx).rev() {
        let code = lines[l].split("//").next().unwrap_or(lines[l]);
        if l < line_idx {
            brace_depth += code.bytes().rev().map(|b| i32::from(b == b'}') - i32::from(b == b'{')).sum::<i32>();
        }
        if brace_depth < 0 {
            let rest = code.trim_start().strip_prefix("unsafe ").map(str::trim_start).unwrap_or_else(|| code.trim_start());
            if let Some(after_impl) = rest.strip_prefix("impl").filter(|s| s.starts_with(|c: char| c.is_whitespace() || c == '<')) {
                let mut header = String::from(after_impl);
                for extra in lines.iter().take((l + 6).min(lines.len())).skip(l + 1) {
                    if !header.contains('{') && !header.contains(" for ") {
                        header.push(' ');
                        header.push_str(extra.split("//").next().unwrap_or(extra).trim());
                    }
                }
                let (before_for, _) = header.split('{').next().unwrap_or(&header).split_once(" for ")?;
                let tr = crate::workspace::cursor_path::strip_generics_and_refs(before_for.trim());
                let idx_in_line = code.find(tr)?;
                return Some(&code[idx_in_line..idx_in_line + tr.len()]);
            }
            if ["trait ", "pub trait ", "pub(crate) trait "].iter().any(|p| rest.starts_with(p)) {
                return None;
            }
        }
    }
    None
}

pub(crate) fn find_enclosing_impl_trait_for_entry<'a>(r: &'a IndexReader<memmap2::Mmap>, impl_def: &DefEntry) -> Option<&'a str> {
    find_enclosing_impl_trait_at(r, impl_def.span().file_id(), impl_def.span().start_line())
}

fn find_enclosing_impl_trait_at(r: &IndexReader<memmap2::Mmap>, file_id: u32, line: u32) -> Option<&str> {
    for d in
        r.definitions().iter().filter(|d| DefKind::from(d.kind) == DefKind::Impl && d.span().file_id() == file_id && d.span().start_line() <= line && line <= d.span().end_line())
    {
        if let (Some(tr), _) = parse_impl_def_path(r.get_str(d.path_offset())) {
            let base_tr = strip_generics(tr);
            if !base_tr.contains("::") {
                if let Some(td) =
                    r.definitions().iter().find(|td| DefKind::from(td.kind) == DefKind::Trait && td.span().file_id() == file_id && r.get_str(td.name_offset()) == base_tr)
                {
                    return Some(r.get_str(td.path_offset()));
                }
            }
            return Some(tr);
        }
    }
    None
}

fn read_crate_source(ws: &WorkspaceIndex, r_crate: &str, r_file: &str) -> Option<String> {
    ws.read_source_file(r_file).or_else(|| ws.resolve_path_for_crate(r_crate, r_file).to_str().and_then(|s| ws.read_source_file(s)))
}

pub(crate) fn is_synthesized_assoc_fn_ref(ws: &WorkspaceIndex, r_crate: &str, r_file: &str, sp: crate::format::Span, def: &ResolvedDefinition<'_>, target_name: &str) -> bool {
    if sp.start_line() == def.start_line && (r_file == def.file || r_file.ends_with(def.file) || def.file.ends_with(r_file)) {
        return false;
    }
    if sp.start_line() == 0 || sp.start_col() == 0 {
        return true;
    }
    let Some(c) = read_crate_source(ws, r_crate, r_file) else { return false };
    let (l_idx, lines): (usize, Vec<&str>) = (sp.start_line() as usize - 1, c.lines().collect());
    let Some(&ln) = lines.get(l_idx) else { return false };
    if ln.trim_start().starts_with("//") {
        return false;
    }
    let chars: Vec<char> = ln.chars().collect();
    let (sc, ec) = ((sp.start_col() as usize - 1).min(chars.len()), if sp.end_line() == sp.start_line() { (sp.end_col() as usize - 1).min(chars.len()) } else { 0 });
    let span_text: String = if sc <= ec { chars[sc..ec].iter().collect() } else { String::new() };
    if span_text != target_name && span_text.strip_prefix("r#") != Some(target_name) {
        return true;
    }
    let pfx: String = chars[..sc].iter().collect();
    if !pfx.trim_end().is_empty() {
        return !pfx.trim_end().ends_with('.') && !pfx.trim_end().ends_with("::");
    }
    lines[..l_idx].iter().rev().map(|s| s.split("//").next().unwrap_or(s).trim_end()).find(|s| !s.is_empty()).is_none_or(|p| !p.ends_with('.') && !p.ends_with("::"))
}

impl WorkspaceIndex {
    /// Checks the source declaration header to distinguish unrestricted `pub` from private or `pub(...)` items.
    pub fn is_decl_externally_pub(&self, reader_idx: Option<usize>, file: &str, line_1b: u32, short_name: &str) -> bool {
        if file.is_empty() || file == "external" || line_1b == 0 || short_name.is_empty() {
            return true;
        }
        let cname = reader_idx.and_then(|ri| self.readers.get(ri)).map(|r| r.crate_name()).unwrap_or("");
        let Some(c) = read_crate_source(self, cname, file) else { return true };
        let (l_idx, lines): (usize, Vec<&str>) = (line_1b as usize - 1, c.lines().collect());
        let Some(&decl_raw) = lines.get(l_idx) else { return true };
        let (decl_code, is_id) = (decl_raw.split("//").next().unwrap_or(decl_raw), |b: u8| b.is_ascii_alphanumeric() || b == b'_');
        let db = decl_code.as_bytes();
        let Some((npos, _)) =
            decl_code.match_indices(short_name).find(|&(p, _)| (p == 0 || !is_id(db[p - 1])) && (p + short_name.len() >= db.len() || !is_id(db[p + short_name.len()])))
        else {
            return true;
        };
        let mut raw: String = lines[l_idx.saturating_sub(3)..l_idx].iter().map(|l| l.split("//").next().unwrap_or(l)).collect::<Vec<_>>().join(" ");
        raw.push(' ');
        raw.push_str(&decl_code[..npos + short_name.len()]);
        let (mut clean, mut depth) = (String::with_capacity(raw.len()), 0i32);
        for ch in raw.chars() {
            match ch {
                '[' => depth += 1,
                ']' => depth = (depth - 1).max(0),
                '#' if depth == 0 => clean.push(' '),
                _ if depth == 0 => clean.push(ch),
                _ => {}
            }
        }
        let seg = clean.rsplit([';', '{', '}']).next().unwrap_or(&clean);
        let sb = seg.as_bytes();
        let has_kw = ["fn", "const", "static", "struct", "enum", "union", "type", "trait", "mod"]
            .iter()
            .any(|kw| seg.match_indices(kw).any(|(p, _)| (p == 0 || !is_id(sb[p - 1])) && (p + kw.len() < sb.len() && !is_id(sb[p + kw.len()]))));
        !has_kw
            || seg
                .match_indices("pub")
                .any(|(p, _)| (p == 0 || !is_id(sb[p - 1])) && (p + 3 >= sb.len() || !is_id(sb[p + 3])) && seg[p + 3..].trim_start().as_bytes().first() != Some(&b'('))
    }
}
