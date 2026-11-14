/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Semantic rename validation and fast refusal checks shared by `prepareRename` and `rename`.

use super::qualified_path::{extract_impl_trait, extract_root_crate};
use super::trait_impl_refs::strip_generics;
use super::*;
use crate::workspace::rename_parse::{extract_macro_rules_blocks, has_self_receiver, ParsedMacroBlock};
use crate::workspace::RenameRefusal;
use std::path::Path;

pub(super) fn detect_enclosing_fn_bounds(ws: &WorkspaceIndex, def: &ResolvedDefinition<'_>) -> Option<(u32, u32)> {
    if !matches!(def.kind, DefKind::Struct | DefKind::Enum | DefKind::Union | DefKind::Fn | DefKind::Trait | DefKind::TyAlias) {
        return None;
    }
    let content = ws.read_source_file(def.file).filter(|c| !(c.contains("macro_rules!") && c.contains(&format!("::<{}>", def.name))))?;
    let li = def.start_line.saturating_sub(1) as usize;
    let dl_pre = content.lines().nth(li)?;
    let ind = dl_pre.len() - dl_pre.trim_start().len();
    if ind == 0 {
        return None;
    }
    let ls: Vec<&str> = content.lines().collect();
    let (fn_k, fn_ind) = (0..li)
        .rev()
        .filter_map(|k| {
            let t = ls[k].split("//").next().unwrap_or(ls[k]).trim();
            let k_ind = ls[k].len() - ls[k].trim_start().len();
            if t.is_empty() || t.starts_with('#') || k_ind >= ind {
                return None;
            }
            let is_fn = (t.starts_with("fn ") || t.contains(" fn ")) && !t.starts_with("macro_rules!");
            let is_other =
                t.starts_with("mod ") || t.contains(" mod ") || t.starts_with("impl") || t.starts_with("trait ") || t.contains(" trait ") || t.starts_with("macro_rules!");
            (is_fn || is_other).then_some((k, k_ind, is_fn))
        })
        .next()
        .and_then(|(k, k_ind, is_fn)| is_fn.then_some((k, k_ind)))?;
    let fn_end_idx = ((li + 1)..ls.len())
        .find(|&m| {
            let mt = ls[m].split("//").next().unwrap_or(ls[m]).trim();
            !mt.is_empty() && (ls[m].len() - ls[m].trim_start().len()) <= fn_ind && mt.starts_with('}')
        })
        .unwrap_or(ls.len().saturating_sub(1));
    let next_dup = def
        .reader_idx
        .and_then(|ri| {
            let r = &ws.readers[ri];
            let fid = r.find_file_id(def.file)?;
            let defs = r.definitions();
            ws.unit_match
                .defs_in_file_up_to(ri, fid, u32::MAX)
                .iter()
                .filter_map(|&(_, sl, di)| (sl > def.start_line).then(|| defs.get(di as usize)).flatten())
                .filter(|d| DefKind::from(d.kind) == def.kind && r.get_str(d.name_offset()) == def.name)
                .map(|d| d.span().start_line())
                .min()
        })
        .unwrap_or(u32::MAX);
    Some(((fn_k + 1) as u32, ((fn_end_idx + 1) as u32).min(next_dup)))
}

pub(crate) fn canonicalize_symbol_path(ws: &WorkspaceIndex, default_crate: &str, raw_path: &str) -> String {
    let trimmed = strip_generics(raw_path).trim();
    if trimmed.starts_with('<') {
        return trimmed.to_string();
    }
    let first = extract_root_crate(trimmed);
    if trimmed.contains("::") && (first == default_crate || matches!(first, "core" | "alloc" | "std" | "proc_macro") || ws.has_crate(first)) {
        trimmed.to_string()
    } else if !default_crate.is_empty() {
        format!("{default_crate}::{trimmed}")
    } else {
        trimmed.to_string()
    }
}

pub(crate) fn same_trait_assoc_family(def_path: &str, other_path: &str) -> bool {
    let tr_a = extract_impl_trait(def_path).map(strip_generics).or_else(|| def_path.rsplit_once("::").map(|(p, _)| strip_generics(p)));
    let tr_b = extract_impl_trait(other_path).map(strip_generics).or_else(|| other_path.rsplit_once("::").map(|(p, _)| strip_generics(p)));
    let (Some(a), Some(b)) = (tr_a, tr_b) else { return false };
    let (sa, sb) = (a.rsplit("::").next().unwrap_or(a), b.rsplit("::").next().unwrap_or(b));
    !sa.is_empty() && sa == sb && (def_path.starts_with('<') || other_path.starts_with('<'))
}

impl WorkspaceIndex {
    /// Returns `(target_kind, target_name)` if `file_path` is a Cargo target root whose entrypoint is `fn main`.
    pub fn cargo_main_entrypoint(&self, file_path: &Path) -> Option<(String, String)> {
        self.rename_cache().cargo_targets.main_entrypoint_for(file_path).cloned()
    }

    /// Evaluates all workspace-level semantic rename refusal rules for `def`.
    pub fn check_rename_refusal_for_def(&self, def: &ResolvedDefinition<'_>, precomputed_refs: Option<&[WorkspaceReference<'_>]>) -> Result<(), RenameRefusal> {
        let target_name = def.name.rsplit("::").next().unwrap_or(def.name);
        if target_name.is_empty() {
            return Err(RenameRefusal::EmptySymbol);
        }
        if def.reader_idx.is_some_and(|i| self.is_reader_external(i)) || self.is_external_target(def.file, &def.resolved_path(), def.unit_key) {
            return Err(RenameRefusal::ExternalSymbol { name: target_name.to_string() });
        }
        if def.kind == DefKind::Fn && target_name == "main" && !def.path.strip_suffix("::main").unwrap_or("").contains("::") {
            if let Some((target_kind, target_title)) = self.cargo_main_entrypoint(&def.resolved_path()) {
                return Err(RenameRefusal::CargoMainEntrypoint { target_kind, target_name: target_title });
            }
        }
        if let Some(trait_path) = self.external_trait_for_def(def) {
            return Err(RenameRefusal::ExternalTraitItem { name: target_name.to_string(), trait_path });
        }

        if !matches!(def.kind, DefKind::AssocFn | DefKind::AssocConst | DefKind::AssocTy | DefKind::Field | DefKind::Variant | DefKind::VariantCtor | DefKind::Local) {
            let aliases = self.collect_reexport_aliases(def.path);
            let needs_unmerged_check = aliases.iter().any(|alias| {
                self.readers.iter().enumerate().any(|(i, r)| {
                    !self.is_reader_external(i)
                        && r.find_references(alias).iter().any(|aref| {
                            DefKind::from(aref.kind) == def.kind
                                && (def.start_line == 0 || aref.target_line() == 0 || aref.target_line() == def.start_line)
                                && (precomputed_refs.is_some() || aref.target_line() == 0 || def.reader_idx.is_some_and(|ti| !self.is_reader_in_dep_graph(&[ti], i)))
                        })
                })
            });
            if needs_unmerged_check {
                let owned_refs;
                let refs_slice = match precomputed_refs {
                    Some(s) => s,
                    None => {
                        owned_refs = self.find_references_for_unit_with_target(def.path, def.unit_key, Some((def.file, def.start_line, def.kind, def.name)));
                        &owned_refs
                    }
                };
                if self.has_unmerged_reexport_references(def.path, refs_slice) {
                    return Err(RenameRefusal::UnmergedReexport { path: def.path.to_string() });
                }
            }
        }

        let cache = self.rename_cache();
        let def_crate = def.reader_idx.and_then(|ri| self.readers.get(ri)).map(|r| r.crate_name()).unwrap_or_else(|| extract_root_crate(def.path));

        if def.kind == DefKind::AssocFn && def.signature.is_some_and(has_self_receiver) {
            if let Some(macro_name) = self.check_macro_body_multi_target_coupled(def, target_name, def_crate, &cache) {
                return Err(RenameRefusal::MacroBodyMultiTargetCoupled { name: target_name.to_string(), macro_name });
            }
        }

        let check_m2 = !matches!(def.kind, DefKind::Local | DefKind::TyParam | DefKind::LifetimeParam | DefKind::ConstParam | DefKind::Mod | DefKind::Field)
            && cache.dual_bound_short_names.contains(target_name);
        let pm_coupled_crate = (def.kind == DefKind::AssocFn && !def.path.starts_with('<'))
            .then(|| cache.proc_macro_coupled_methods.get(target_name))
            .flatten()
            .filter(|pm| pm.as_str() != def_crate && self.is_pub_inherent_struct_method(def, def_crate, target_name));

        if check_m2 || pm_coupled_crate.is_some() {
            let (has_macro_exp_ref, dual_bound) = self.scan_macro_expansion_ref_hazards(def, target_name, def_crate, check_m2, pm_coupled_crate.is_some());
            if dual_bound {
                return Err(RenameRefusal::MacroCallDualBoundToken { name: target_name.to_string() });
            }
            if let Some(pm_crate) = pm_coupled_crate {
                let any_mac = has_macro_exp_ref || precomputed_refs.is_some_and(|rs| rs.iter().any(|r| r.is_macro_expansion));
                if any_mac {
                    return Err(RenameRefusal::ProcMacroStringCoupled { name: target_name.to_string(), crate_name: pm_crate.clone() });
                }
            }
        }

        Ok(())
    }

    fn is_pub_inherent_struct_method(&self, def: &ResolvedDefinition<'_>, def_crate: &str, short_name: &str) -> bool {
        let Some((parent, _)) = def.path.rsplit_once("::") else { return false };
        if let Some(ri) = def.reader_idx.and_then(|i| self.readers.get(i)) {
            if ri.definitions().iter().any(|d| d.span().start_line() == def.start_line && ri.get_str(d.name_offset()) == short_name && d.visibility != 0) {
                return false;
            }
        }
        if !self.is_decl_externally_pub(def.reader_idx, def.file, def.start_line, short_name) {
            return false;
        }
        let clean_parent = strip_generics(parent).trim().trim_end_matches("::");
        let local_parent = clean_parent.strip_prefix(&format!("{def_crate}::")).unwrap_or(clean_parent);
        let global_parent = format!("{def_crate}::{local_parent}");
        for &ri in self.readers_for_crate_fast(def_crate).iter().chain(def.reader_idx.as_slice()) {
            let Some(r) = self.readers.get(ri) else { continue };
            if [clean_parent, local_parent, global_parent.as_str()].iter().any(|&p| super::decl_lookup::reader_has_def_kind_at_path(r, p, DefKind::Struct)) {
                return true;
            }
        }
        false
    }

    fn check_macro_body_multi_target_coupled(
        &self,
        def: &ResolvedDefinition<'_>,
        target_name: &str,
        def_crate: &str,
        cache: &crate::workspace::rename_cache::RenameWorkspaceCache,
    ) -> Option<String> {
        let def_local_mod = def
            .path
            .rsplit_once("::")
            .map(|(parent, _)| strip_generics(parent).trim())
            .and_then(|p| p.rsplit_once("::").map(|(m, _)| m.strip_prefix(&format!("{def_crate}::")).unwrap_or(m)))
            .unwrap_or("");
        let file_multi = cache.file_self_method_owners.get(&(def.file.to_string(), target_name.to_string())).copied().unwrap_or(0) >= 2;
        let mod_multi = !def_local_mod.is_empty() && cache.mod_self_method_owners.get(&(def_local_mod.to_string(), target_name.to_string())).copied().unwrap_or(0) >= 2;
        if !file_multi && !mod_multi {
            return None;
        }

        let check_block = |blk: &ParsedMacroBlock| -> bool {
            if blk.has_impl_kw || !blk.dot_calls.iter().any(|dc| dc.method == target_name) {
                return false;
            }
            if file_multi && blk.file_path == def.file {
                return true;
            }
            mod_multi && blk.glob_modules.iter().any(|gm| gm == def_local_mod || def_local_mod.ends_with(&format!("::{gm}")))
        };

        if let Some(blocks) = cache.macro_blocks_by_crate.get(def_crate) {
            for (_, blk) in blocks {
                if check_block(blk) {
                    return Some(blk.macro_name.clone());
                }
            }
        }
        if def.reader_idx.is_none() {
            if let Some(content) = self.read_source_file(def.file).filter(|c| c.contains("macro_rules!")) {
                for blk in extract_macro_rules_blocks(0, def.file, &content) {
                    if check_block(&blk) {
                        return Some(blk.macro_name);
                    }
                }
            }
        }
        None
    }

    fn scan_macro_expansion_ref_hazards(&self, def: &ResolvedDefinition<'_>, target_name: &str, def_crate: &str, check_m2: bool, check_pm_attr: bool) -> (bool, bool) {
        let local_path = def.path.strip_prefix(&format!("{def_crate}::")).unwrap_or(def.path);
        let global_path = if local_path.starts_with('<') { local_path.to_string() } else { format!("{def_crate}::{local_path}") };
        let def_canon = canonicalize_symbol_path(self, def_crate, def.path);
        let name_chars = target_name.chars().count() as u32;
        let source_readers: Vec<usize> = match def.reader_idx {
            Some(ri) => vec![ri],
            None => self.readers_for_crate_fast(def_crate).into_owned(),
        };

        let mut all_queries = vec![def.path.to_string(), local_path.to_string(), global_path];
        for sp in super::qualified_path::consumer_spellings(local_path, def_crate).into_iter().chain(super::qualified_path::impl_consumer_spellings(local_path, def_crate)) {
            if !all_queries.contains(&sp) {
                all_queries.push(sp);
            }
        }
        let mut any_macro_exp = false;
        for (i, reader) in self.readers.iter().enumerate() {
            if self.is_reader_external(i)
                || (!source_readers.is_empty() && reader.crate_name() != def_crate && !source_readers.iter().any(|&sr| self.is_reader_in_dep_graph(&[i], sr)))
            {
                continue;
            }
            let r_crate = reader.crate_name();
            for q in &all_queries {
                if q.is_empty() || (r_crate != def_crate && q == local_path) {
                    continue;
                }
                // A proc macro's generated call is no name use (`find_references` skips it): ask for macro-written refs.
                any_macro_exp = any_macro_exp || (check_pm_attr && reader.has_macro_expansion_reference(q, def.start_line));
                for r in reader.find_references(q) {
                    if def.start_line > 0 && r.target_line() > 0 && r.target_line() != def.start_line {
                        continue;
                    }
                    let sp = r.span();
                    if !any_macro_exp
                        && ((r.is_macro_expansion() && !check_pm_attr)
                            || (check_pm_attr
                                && DefKind::from(r.kind) == DefKind::AssocFn
                                && decl_lookup::is_synthesized_assoc_fn_ref(self, r_crate, reader.get_file(sp.file_id()), sp, def, target_name)))
                    {
                        any_macro_exp = true;
                    }
                    if !check_m2 || matches!(DefKind::from(r.kind), DefKind::Macro | DefKind::Use) {
                        continue;
                    }
                    if sp.start_line() == 0 || sp.start_line() != sp.end_line() || sp.end_col().saturating_sub(sp.start_col()) != name_chars {
                        continue;
                    }
                    for r2 in reader.references_at_start(sp.file_id(), sp.start_line(), sp.start_col()) {
                        let sp2 = r2.span();
                        if sp2.end_line() != sp.end_line()
                            || sp2.end_col() != sp.end_col()
                            || (!r.is_macro_expansion() && !r2.is_macro_expansion())
                            || matches!(DefKind::from(r2.kind), DefKind::Macro | DefKind::Use)
                            || !reader.is_name_reference(r2)
                        {
                            continue;
                        }
                        let tp2 = reader.get_str(r2.target_path_offset());
                        if tp2.rsplit("::").next() == Some(target_name) && canonicalize_symbol_path(self, r_crate, tp2) != def_canon && !same_trait_assoc_family(def.path, tp2) {
                            return (true, true);
                        }
                    }
                }
            }
        }
        (any_macro_exp, false)
    }

    /// Lightweight validation for `textDocument/prepareRename` without expanding full workspace edits.
    pub fn check_prepare_rename_at<'a>(&'a self, file: &str, line: u32, col: u32) -> Option<Result<ResolvedDefinition<'a>, RenameRefusal>> {
        let def = self.find_definition_at(file, line, col)?;
        Some(self.check_rename_refusal_for_def(&def, None).map(|()| def))
    }

    /// Returns `(def_crate, owner_short, is_trait_owner, has_self, is_externally_visible)` for semantic resolution in unindexed Cargo targets.
    pub fn unindexed_symbol_owner_info(&self, reader_idx: Option<usize>, kind: DefKind, path: &str, file: &str, start_line: u32) -> (String, String, bool, bool, bool) {
        let mut def_crate = reader_idx.and_then(|ri| self.readers.get(ri)).map(|r| r.crate_name()).unwrap_or_else(|| extract_root_crate(path)).to_string();
        let short_item = path.rsplit("::").next().unwrap_or(path);
        let entry_pair = reader_idx.and_then(|ri| {
            let r = self.readers.get(ri)?;
            let sub = strip_crate_prefix(path, r.crate_name()).unwrap_or(path);
            let e = r
                .find_definition_by_path(path)
                .or_else(|| r.find_definition_by_path(sub))
                .or_else(|| r.definitions().iter().find(|d| d.span().start_line() == start_line && r.get_str(d.name_offset()) == short_item))?;
            Some((r, e))
        });
        let entry_is_pub = entry_pair.map(|(_, e)| e.visibility == 0).unwrap_or_else(|| {
            self.find_definition_by_path(path)
                .and_then(|d| d.reader_idx.and_then(|ri| self.readers.get(ri)).and_then(|r| r.find_definition_by_path(d.path)))
                .is_none_or(|e| e.visibility == 0)
        });
        let has_self = kind == DefKind::AssocFn
            && entry_pair
                .and_then(|(r, e)| (e.sig_offset() != 0).then(|| r.get_str(e.sig_offset())))
                .or_else(|| self.find_definition_by_path(path).and_then(|d| d.signature))
                .is_some_and(has_self_receiver);
        if let Some(tr) = extract_impl_trait(path).or_else(|| entry_pair.and_then(|(r, e)| decl_lookup::find_enclosing_impl_trait_for_entry(r, e))) {
            let tr_base = strip_generics(tr).trim();
            let short = tr_base.rsplit("::").next().unwrap_or("").to_string();
            let tr_first = extract_root_crate(tr_base);
            let trait_crate = if !tr_first.is_empty() && tr_first != short && self.has_crate(tr_first) {
                tr_first.to_string()
            } else if let Some(sym) = self.find_first_symbol(&short, |s| s.name == short && s.kind == DefKind::Trait) {
                sym.crate_name.to_string()
            } else {
                def_crate
            };
            return (trait_crate, short, true, has_self, true);
        }
        let raw_parent = path.rsplit_once("::").map(|(p, _)| p.trim()).unwrap_or("");
        let unwrapped_parent = raw_parent.strip_prefix('<').and_then(|s| s.strip_suffix('>')).unwrap_or(raw_parent);
        let parent = strip_generics(unwrapped_parent).trim().trim_end_matches("::");
        let owner_short = parent.rsplit("::").next().unwrap_or("").to_string();
        let is_trait_owner = !parent.is_empty()
            && (self.find_definition_by_path(parent).is_some_and(|d| d.kind == DefKind::Trait)
                || self.find_definition_by_path(&format!("{def_crate}::{parent}")).is_some_and(|d| d.kind == DefKind::Trait));
        if is_trait_owner {
            let p_first = extract_root_crate(parent);
            if !p_first.is_empty() && p_first != owner_short && self.has_crate(p_first) {
                def_crate = p_first.to_string();
            }
        }
        let is_ext_vis = is_trait_owner || kind == DefKind::Variant || (entry_is_pub && self.is_decl_externally_pub(reader_idx, file, start_line, short_item));
        (def_crate, owner_short, is_trait_owner, has_self, is_ext_vis)
    }

    /// Resolves and expands all workspace rename references at `(file, line, col)`, returning an explicit
    /// `RenameRefusal` if renaming the target symbol is semantically unsafe or disallowed.
    pub fn try_find_rename_references_at<'a>(&'a self, file: &str, line: u32, col: u32) -> Option<Result<LocationReferences<'a>, RenameRefusal>> {
        let (def, mut refs) = self.resolve_direct_references_at(file, line, col)?;
        if let Err(refusal) = self.check_rename_refusal_for_def(&def, Some(&refs)) {
            return Some(Err(refusal));
        }
        self.expand_trait_impl_for_rename(&def, &mut refs);
        self.expand_rename_type_and_sibling_refs(&def, &mut refs);
        Some(Ok(LocationReferences {
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
        }))
    }
}
