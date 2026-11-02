/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Per-workspace precomputed rename index: Cargo target tables, associated-item reference lookup,
//! `macro_rules!` expansion body index, `proc-macro` source Coupling cache, and unindexed targets.

use super::cargo_targets::CargoWorkspaceTargets;
use super::rename_parse::{
    extract_macro_rules_blocks, has_self_receiver, parse_unindexed_rust_file, scan_proc_macro_dir, ParsedMacroBlock, ParsedUnindexedTarget, ProcMacroSourceFile,
};
use super::WorkspaceIndex;
use crate::format::DefKind;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

/// Cached workspace-wide structures shared by `prepareRename` and `rename`.
pub struct RenameWorkspaceCache {
    pub cargo_targets: CargoWorkspaceTargets,
    /// Short associated item name -> `(reader_idx, ref_idx)` across workspace shards.
    pub assoc_refs_by_name: HashMap<String, Vec<(u32, u32)>>,
    /// Uppercase-starting short path segment -> deduplicated `(reader_idx, file_id)` pairs referencing it.
    pub trait_ref_files_by_short: HashMap<String, Vec<(u32, u32)>>,
    /// Short identifier names that participate in at least one dual-bound macro call-site token (`M2`).
    pub dual_bound_short_names: HashSet<String>,
    /// Short names of all `Trait` definitions across workspace shards.
    pub ws_trait_short_names: HashSet<String>,
    /// Type/trait short name -> workspace crate names defining a non-reexport type/trait of that name.
    pub type_def_crates_by_name: HashMap<String, HashSet<String>>,
    /// Inherent method names coupled to string literals + `.#ident` synthesis in workspace `proc-macro` crates.
    pub proc_macro_coupled_methods: HashMap<String, String>,
    /// Pre-loaded workspace `proc-macro` source files.
    pub proc_macro_files: Vec<ProcMacroSourceFile>,
    /// Crate name -> `(reader_idx, ParsedMacroBlock)` for indexed `macro_rules!` files.
    pub macro_blocks_by_crate: HashMap<String, Vec<(usize, ParsedMacroBlock)>>,
    /// `(file, method_name)` -> count of distinct root owners with `fn method(...self...)` in `file`.
    pub file_self_method_owners: HashMap<(String, String), usize>,
    /// `(module_path_suffix, method_name)` -> count of distinct root owners in that module.
    pub mod_self_method_owners: HashMap<(String, String), usize>,
    /// Method name -> `(crate_name, owner_short, is_trait)` for workspace `AssocFn` items with a `self` receiver.
    pub self_methods_by_name: HashMap<String, Vec<(String, String, bool)>>,
    /// Pre-parsed unindexed Cargo target files (`bin`/`example`/`test`/`bench`/`custom-build`).
    pub unindexed_targets: Vec<ParsedUnindexedTarget>,
    /// Identifier token -> indices into `unindexed_targets`.
    pub unindexed_ident_map: HashMap<String, Vec<usize>>,
}

type CacheKey = (u64, u64, usize);
static RENAME_WS_CACHE: OnceLock<Mutex<HashMap<CacheKey, Arc<RenameWorkspaceCache>>>> = OnceLock::new();

fn extract_root_owner_key(reader_crate: &str, def_path: &str, enclosing_impl_trait: Option<&str>, trait_paths: &HashSet<&str>) -> Option<String> {
    if let Some(tr) = super::navigation::qualified_path::extract_impl_trait(def_path).or(enclosing_impl_trait) {
        let base = tr.split('<').next().unwrap_or(tr).trim();
        let short = base.rsplit("::").next().unwrap_or(base);
        return (!short.is_empty()).then(|| format!("trait:{short}"));
    }
    let (raw_parent, _) = def_path.rsplit_once("::")?;
    let parent = raw_parent.trim();
    let unwrapped = parent.strip_prefix('<').and_then(|s| s.strip_suffix('>')).unwrap_or(parent);
    let base_parent = unwrapped.split('<').next().unwrap_or(unwrapped).trim();
    if trait_paths.contains(base_parent) {
        let short = base_parent.rsplit("::").next().unwrap_or(base_parent);
        return Some(format!("trait:{short}"));
    }
    let local_p = base_parent.strip_prefix(&format!("{reader_crate}::")).unwrap_or(base_parent);
    Some(format!("inherent:{reader_crate}::{local_p}"))
}

fn collect_declared_submodules(root_rs: &Path, out: &mut Vec<std::path::PathBuf>) {
    let Some(base_dir) = root_rs.parent() else { return };
    let Ok(src) = std::fs::read_to_string(root_rs) else { return };
    let mod_search_dir = if matches!(root_rs.file_stem().and_then(|s| s.to_str()), Some("main" | "lib" | "mod")) {
        base_dir.to_path_buf()
    } else if let Some(stem) = root_rs.file_stem() {
        base_dir.join(stem)
    } else {
        base_dir.to_path_buf()
    };
    let mut pending_path_attr: Option<String> = None;
    for raw_ln in src.lines() {
        let ln = crate::workspace::rename_parse::strip_line_comment(raw_ln).trim();
        if let Some(rest) = ln.strip_prefix("#[path").map(str::trim_start).and_then(|r| r.strip_prefix('=')) {
            if let Some(q1) = rest.find('"') {
                if let Some(q2) = rest[q1 + 1..].find('"') {
                    pending_path_attr = Some(rest[q1 + 1..q1 + 1 + q2].to_string());
                }
            }
            continue;
        }
        let mod_rest = ln.strip_prefix("mod ").or_else(|| ln.strip_prefix("pub mod ")).or_else(|| ln.strip_prefix("pub(crate) mod "));
        if let Some(mr) = mod_rest {
            if let Some(semi) = mr.strip_suffix(';') {
                let mod_name = semi.trim();
                if let Some(rel) = pending_path_attr.take() {
                    let p = crate::paths::normalize_path(&base_dir.join(rel));
                    if p.is_file() {
                        out.push(p);
                    }
                } else if !mod_name.is_empty() {
                    for cand in [mod_search_dir.join(format!("{mod_name}.rs")), mod_search_dir.join(mod_name).join("mod.rs"), base_dir.join(format!("{mod_name}.rs"))] {
                        if cand.is_file() {
                            out.push(cand);
                            break;
                        }
                    }
                }
            } else {
                pending_path_attr = None;
            }
        } else if !ln.starts_with("#[") && !ln.is_empty() {
            pending_path_attr = None;
        }
    }
}

impl RenameWorkspaceCache {
    pub fn get(ws: &WorkspaceIndex) -> Arc<Self> {
        let key = (ws.instance_id, ws.generation, ws.readers.len());
        let mutex = RENAME_WS_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
        if let Some(existing) = mutex.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
            return Arc::clone(existing);
        }
        let built = Arc::new(Self::build(ws));
        let mut guard = mutex.lock().unwrap_or_else(|e| e.into_inner());
        guard.retain(|&(inst, gen, _), _| inst != ws.instance_id || gen == ws.generation);
        guard.insert(key, Arc::clone(&built));
        built
    }

    fn build(ws: &WorkspaceIndex) -> Self {
        let cargo_targets = CargoWorkspaceTargets::discover(ws);
        let mut assoc_refs_by_name: HashMap<String, Vec<(u32, u32)>> = HashMap::new();
        let mut trait_ref_files_by_short: HashMap<String, Vec<(u32, u32)>> = HashMap::new();
        let mut dual_bound_short_names: HashSet<String> = HashSet::new();
        let mut ws_trait_short_names: HashSet<String> = HashSet::new();
        let mut type_def_crates_by_name: HashMap<String, HashSet<String>> = HashMap::new();
        let mut macro_blocks_by_crate: HashMap<String, Vec<(usize, ParsedMacroBlock)>> = HashMap::new();
        let mut scanned_macro_files: HashSet<(String, String)> = HashSet::new();
        let mut file_owner_sets: HashMap<(String, String), HashSet<String>> = HashMap::new();
        let mut mod_owner_sets: HashMap<(String, String), HashSet<String>> = HashMap::new();
        let mut self_methods_by_name: HashMap<String, Vec<(String, String, bool)>> = HashMap::new();

        for (i, reader) in ws.readers.iter().enumerate() {
            if ws.is_reader_external(i) {
                continue;
            }
            let cname = reader.crate_name();
            let refs = reader.references();
            let mut span_name_refs: Vec<(u32, u32, u32, u32, u32)> = Vec::new();
            for (r_idx, r) in refs.iter().enumerate() {
                let r_kind = DefKind::from(r.kind);
                let tp = reader.get_str(r.target_path_offset());
                let short = tp.rsplit("::").next().unwrap_or(tp);
                if !short.is_empty() && (r_kind == DefKind::Trait || short.as_bytes()[0].is_ascii_uppercase()) {
                    let pair = (i as u32, r.span().file_id());
                    let list = trait_ref_files_by_short.entry(short.to_string()).or_default();
                    if list.last() != Some(&pair) {
                        list.push(pair);
                    }
                }
                if matches!(r_kind, DefKind::AssocFn | DefKind::AssocTy | DefKind::AssocConst) && reader.is_name_reference(r) && !short.is_empty() {
                    assoc_refs_by_name.entry(short.to_string()).or_default().push((i as u32, r_idx as u32));
                }
                let sp = r.span();
                if !short.is_empty()
                    && !matches!(r_kind, DefKind::Macro | DefKind::Use)
                    && reader.is_name_reference(r)
                    && sp.start_line() > 0
                    && sp.start_line() == sp.end_line()
                    && sp.end_col().saturating_sub(sp.start_col()) == short.chars().count() as u32
                {
                    span_name_refs.push((sp.file_id(), sp.start_line(), sp.start_col(), sp.end_col(), r_idx as u32));
                }
            }
            span_name_refs.sort_unstable();
            for win in span_name_refs.windows(2) {
                if (win[0].0, win[0].1, win[0].2, win[0].3) != (win[1].0, win[1].1, win[1].2, win[1].3) {
                    continue;
                }
                let (r1, r2) = (&refs[win[0].4 as usize], &refs[win[1].4 as usize]);
                if !r1.is_macro_expansion() && !r2.is_macro_expansion() {
                    continue;
                }
                let (tp1, tp2) = (reader.get_str(r1.target_path_offset()), reader.get_str(r2.target_path_offset()));
                let (Some(s1), Some(s2)) = (tp1.rsplit("::").next(), tp2.rsplit("::").next()) else { continue };
                if s1 == s2 && !s1.is_empty() {
                    let c1 = super::navigation::rename_check::canonicalize_symbol_path(ws, cname, tp1);
                    let c2 = super::navigation::rename_check::canonicalize_symbol_path(ws, cname, tp2);
                    if c1 != c2 && !super::navigation::rename_check::same_trait_assoc_family(tp1, tp2) {
                        dual_bound_short_names.insert(s1.to_string());
                    }
                }
            }
            let defs = reader.definitions();
            let mut trait_paths: HashSet<&str> = HashSet::new();
            let mut impls_by_file: HashMap<u32, Vec<(u32, u32, &str)>> = HashMap::new();
            for d in defs {
                let dk = DefKind::from(d.kind);
                if !d.is_reexport() && matches!(dk, DefKind::Struct | DefKind::Enum | DefKind::Union | DefKind::Trait | DefKind::TyAlias) {
                    let short = reader.get_str(d.name_offset());
                    if !short.is_empty() {
                        type_def_crates_by_name.entry(short.to_string()).or_default().insert(cname.to_string());
                    }
                }
                if dk == DefKind::Trait {
                    let short = reader.get_str(d.name_offset());
                    let dp = reader.get_str(d.path_offset());
                    trait_paths.insert(dp);
                    if !short.is_empty() {
                        ws_trait_short_names.insert(short.to_string());
                        let list = trait_ref_files_by_short.entry(short.to_string()).or_default();
                        for r in reader.find_references(dp) {
                            let pair = (i as u32, r.span().file_id());
                            if !list.contains(&pair) {
                                list.push(pair);
                            }
                        }
                    }
                } else if dk == DefKind::Impl {
                    if let (Some(tr), _) = super::navigation::impl_parse::parse_impl_def_path(reader.get_str(d.path_offset())) {
                        let sp = d.span();
                        impls_by_file.entry(sp.file_id()).or_default().push((sp.start_line(), sp.end_line(), tr));
                    }
                }
            }

            let mut macro_fids = Vec::new();
            for d in defs {
                let kind = DefKind::from(d.kind);
                if kind == DefKind::Macro && !d.is_reexport() {
                    macro_fids.push(d.span().file_id());
                } else if kind == DefKind::AssocFn && d.sig_offset() != 0 && has_self_receiver(reader.get_str(d.sig_offset())) {
                    let name = reader.get_str(d.name_offset());
                    let dp = reader.get_str(d.path_offset());
                    let sp = d.span();
                    let file = reader.get_file(sp.file_id());
                    let enc_tr = super::navigation::qualified_path::extract_impl_trait(dp).or_else(|| {
                        impls_by_file.get(&sp.file_id()).and_then(|v| v.iter().find_map(|&(sl, el, tr)| (sl <= sp.start_line() && sp.start_line() <= el).then_some(tr)))
                    });
                    if let Some(root_key) = extract_root_owner_key(cname, dp, enc_tr, &trait_paths) {
                        let (owner_short, is_tr) = if let Some(tr_s) = root_key.strip_prefix("trait:") {
                            (tr_s.to_string(), true)
                        } else {
                            let s = root_key.rsplit("::").next().unwrap_or(&root_key).to_string();
                            (s, false)
                        };
                        if !owner_short.is_empty() && (is_tr || d.visibility == 0) {
                            let list = self_methods_by_name.entry(name.to_string()).or_default();
                            if !list.iter().any(|(c, o, t)| *t == is_tr && o == &owner_short && (is_tr || c == cname)) {
                                list.push((cname.to_string(), owner_short, is_tr));
                            }
                        }
                        file_owner_sets.entry((file.to_string(), name.to_string())).or_default().insert(root_key.clone());
                        if let Some((parent, _)) = dp.rsplit_once("::") {
                            let clean_p = parent.split('<').next().unwrap_or(parent).trim();
                            if let Some((mod_path, _)) = clean_p.rsplit_once("::") {
                                let local_mod = mod_path.strip_prefix(&format!("{cname}::")).unwrap_or(mod_path);
                                if !local_mod.is_empty() {
                                    mod_owner_sets.entry((local_mod.to_string(), name.to_string())).or_default().insert(root_key.clone());
                                    if let Some((_, last_seg)) = local_mod.rsplit_once("::") {
                                        mod_owner_sets.entry((last_seg.to_string(), name.to_string())).or_default().insert(root_key);
                                    }
                                }
                            }
                        }
                    }
                }
            }

            macro_fids.sort_unstable();
            macro_fids.dedup();
            for fid in macro_fids {
                let f = reader.get_file(fid);
                if !scanned_macro_files.insert((cname.to_string(), f.to_string())) {
                    continue;
                }
                if let Some(content) = ws.read_source_file(f) {
                    for blk in extract_macro_rules_blocks(fid, f, &content) {
                        macro_blocks_by_crate.entry(cname.to_string()).or_default().push((i, blk));
                    }
                }
            }
        }

        let file_self_method_owners = file_owner_sets.into_iter().map(|(k, v)| (k, v.len())).collect();
        let mod_self_method_owners = mod_owner_sets.into_iter().map(|(k, v)| (k, v.len())).collect();

        let mut proc_macro_coupled_methods = HashMap::new();
        let mut proc_macro_files = Vec::new();
        for (pm_crate, src_dir) in &cargo_targets.proc_macro_crates {
            scan_proc_macro_dir(pm_crate, src_dir, &mut proc_macro_coupled_methods, &mut proc_macro_files);
        }
        for pm_file in &mut proc_macro_files {
            for (i, reader) in ws.readers.iter().enumerate() {
                if !ws.is_reader_external(i) && reader.crate_name() == pm_file.crate_name {
                    if let Some(target_rel) = ws.target_file_for_reader(i, &pm_file.file_path) {
                        if let Some(fid) = reader.find_file_id(target_rel.as_ref()) {
                            pm_file.reader_idx = Some(i);
                            pm_file.file_id = Some(fid);
                            pm_file.file_path = reader.get_file(fid).to_string();
                            break;
                        }
                    }
                }
            }
        }

        let mut unindexed_targets = Vec::new();
        let mut seen_unindexed = HashSet::new();
        for tgt in &cargo_targets.member_targets {
            if tgt.src_path.to_str().is_some_and(|s| !ws.resolved_reader_file_ids(s).is_empty()) {
                continue;
            }
            let mut candidate_paths = vec![tgt.src_path.clone()];
            if tgt.kinds.iter().any(|k| matches!(k.as_str(), "example" | "test" | "bench")) {
                collect_declared_submodules(&tgt.src_path, &mut candidate_paths);
                if tgt.src_path.file_name().and_then(|s| s.to_str()) == Some("main.rs") {
                    if let Some(dir) = tgt.src_path.parent() {
                        if let Ok(entries) = std::fs::read_dir(dir) {
                            for entry in entries.flatten() {
                                let ep = entry.path();
                                if ep.extension().and_then(|s| s.to_str()) == Some("rs") {
                                    candidate_paths.push(ep);
                                }
                            }
                        }
                    }
                }
            }
            for cand_path in candidate_paths {
                let Some(path_str) = cand_path.to_str() else { continue };
                if !ws.resolved_reader_file_ids(path_str).is_empty() || ws.is_external_path(&cand_path) {
                    continue;
                }
                if seen_unindexed.insert(cand_path.clone()) {
                    if let Some(parsed) = parse_unindexed_rust_file(cand_path, tgt.pkg_crate.clone(), tgt.dep_crates.clone()) {
                        unindexed_targets.push(parsed);
                    }
                }
            }
        }
        let mut unindexed_ident_map: HashMap<String, Vec<usize>> = HashMap::new();
        for (u_idx, ut) in unindexed_targets.iter().enumerate() {
            for ident in &ut.idents {
                unindexed_ident_map.entry(ident.clone()).or_default().push(u_idx);
            }
        }

        Self {
            cargo_targets,
            assoc_refs_by_name,
            trait_ref_files_by_short,
            dual_bound_short_names,
            ws_trait_short_names,
            type_def_crates_by_name,
            proc_macro_coupled_methods,
            proc_macro_files,
            macro_blocks_by_crate,
            file_self_method_owners,
            mod_self_method_owners,
            self_methods_by_name,
            unindexed_targets,
            unindexed_ident_map,
        }
    }
}

impl WorkspaceIndex {
    /// Returns the shared workspace rename cache for the current generation.
    pub fn rename_cache(&self) -> Arc<RenameWorkspaceCache> {
        RenameWorkspaceCache::get(self)
    }
}
