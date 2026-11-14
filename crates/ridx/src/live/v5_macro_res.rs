/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Module-qualified, `use`-re-exported, and cross-crate `macro_rules!` resolver for [`RidxV5ItemDb`].

use ridx_format::{DefKind, UseOwner, USE_FLAG_GLOB};
use ridx_syntax::name_res::{CrateId, GlobalDefId, ItemDb};

use super::v5_db::{RidxV5ItemDb, CRATE_ROOT_DEF_IDX, PROC_MACRO_SYNTH_BIT};

#[inline]
fn norm_ident(s: &str) -> &str {
    s.strip_prefix("r#").unwrap_or(s)
}

pub fn resolve_v5_macro_rules<'w>(db: &'w RidxV5ItemDb<'w>, krate: CrateId, module_path: &str, call_path: &str) -> Option<(&'w str, &'w str)> {
    let r_idx = krate.0 as usize;
    let reader = db.ws.readers().get(r_idx)?;
    let trimmed = call_path.trim();
    let rooted = trimmed.starts_with("::");
    let clean = trimmed.trim_start_matches("::");
    let segs: Vec<&str> = clean.split("::").map(|s| norm_ident(s.trim())).filter(|s| !s.is_empty()).collect();
    if segs.is_empty() {
        return None;
    }

    let mut visited = Vec::with_capacity(16);
    if segs.len() == 1 {
        let mac_name = segs[0];
        if !rooted {
            let mut cur_path = if module_path.is_empty() { "crate" } else { module_path };
            loop {
                let mgid = mod_gid_from_crate_path(db, r_idx, cur_path);
                if mgid.is_some() {
                    if let Some(hit) = find_macro_in_mod(db, r_idx, mgid, mac_name, 0, &mut visited) {
                        return Some(hit);
                    }
                }
                match cur_path.rsplit_once("::") {
                    Some((parent, _)) => cur_path = parent,
                    None => break,
                }
            }
            if let Some(pre) = db.prelude_module(krate) {
                if let Some(hit) = find_macro_in_mod(db, r_idx, pre, mac_name, 0, &mut visited) {
                    return Some(hit);
                }
            }
        }
        return db.shards.get(r_idx).and_then(|s| s.lookup_macro_rules(reader, mac_name)).map(|b| (b, "crate"));
    }

    let prefix = &segs[..segs.len() - 1];
    let mac_name = segs[segs.len() - 1];
    let (mut cur_mod, start_idx) = resolve_prefix_head(db, krate, r_idx, module_path, rooted, prefix)?;
    for &seg in &prefix[start_idx..] {
        visited.clear();
        cur_mod = step_mod_in_mod(db, cur_mod, seg, 0, &mut visited)?;
    }
    visited.clear();
    find_macro_in_mod(db, r_idx, cur_mod, mac_name, 0, &mut visited)
}

fn resolve_prefix_head(db: &RidxV5ItemDb<'_>, krate: CrateId, r_idx: usize, module_path: &str, rooted: bool, prefix: &[&str]) -> Option<(GlobalDefId, usize)> {
    let first = prefix[0];
    if rooted {
        let ext = db.extern_crate(krate, first)?;
        return Some((db.crate_root_gid(ext.0 as usize), 1));
    }
    match first {
        "crate" => Some((db.crate_root_gid(r_idx), 1)),
        "self" => Some((mod_gid_from_crate_path(db, r_idx, module_path), 1)),
        "super" => {
            let mut n_super = 0usize;
            while n_super < prefix.len() && prefix[n_super] == "super" {
                n_super += 1;
            }
            let mut cur_path = if module_path.is_empty() { "crate" } else { module_path };
            for _ in 0..n_super {
                cur_path = cur_path.rsplit_once("::").map_or("crate", |(p, _)| p);
            }
            Some((mod_gid_from_crate_path(db, r_idx, cur_path), n_super))
        }
        other => {
            let mut visited = Vec::with_capacity(16);
            let cur_gid = mod_gid_from_crate_path(db, r_idx, module_path);
            if cur_gid.is_some() {
                if let Some(m) = step_mod_in_mod(db, cur_gid, other, 0, &mut visited) {
                    return Some((m, 1));
                }
            }
            let root_gid = db.crate_root_gid(r_idx);
            if root_gid != cur_gid {
                if let Some(m) = step_mod_in_mod(db, root_gid, other, 0, &mut visited) {
                    return Some((m, 1));
                }
            }
            if let Some(ext) = db.extern_crate(krate, other) {
                return Some((db.crate_root_gid(ext.0 as usize), 1));
            }
            if let Some(pre) = db.prelude_module(krate) {
                if let Some(m) = step_mod_in_mod(db, pre, other, 0, &mut visited) {
                    return Some((m, 1));
                }
            }
            None
        }
    }
}

fn mod_gid_from_crate_path(db: &RidxV5ItemDb<'_>, r_idx: usize, module_path: &str) -> GlobalDefId {
    let rel = module_path.strip_prefix("crate::").unwrap_or(if module_path == "crate" { "" } else { module_path });
    if rel.is_empty() {
        return db.crate_root_gid(r_idx);
    }
    let found = db.find_def_in_reader_by_path(r_idx, rel);
    if found.is_some() {
        found
    } else {
        db.crate_root_gid(r_idx)
    }
}

fn step_mod_in_mod(db: &RidxV5ItemDb<'_>, mod_gid: GlobalDefId, seg: &str, depth: u8, visited: &mut Vec<GlobalDefId>) -> Option<GlobalDefId> {
    if !mod_gid.is_some() || depth > 5 || visited.contains(&mod_gid) {
        return None;
    }
    visited.push(mod_gid);
    let r_idx = mod_gid.krate.0 as usize;
    let reader = db.ws.readers().get(r_idx)?;
    let shard = db.shards.get(r_idx)?;
    let is_root = mod_gid.index == CRATE_ROOT_DEF_IDX || Some(mod_gid.index) == shard.root_mod_def_idx;
    let key = if is_root { shard.root_mod_def_idx.unwrap_or(CRATE_ROOT_DEF_IDX) } else { mod_gid.index };
    let defs = reader.definitions();
    if let Some(children) = shard.mod_children.get(&key) {
        for &didx in children {
            if didx < PROC_MACRO_SYNTH_BIT {
                if let Some(d) = defs.get(didx as usize) {
                    if DefKind::from(d.kind) == DefKind::Mod && norm_ident(reader.get_str(d.name_offset())) == seg {
                        return Some(db.canonical_def(GlobalDefId::new(mod_gid.krate, didx)));
                    }
                }
            }
        }
    }
    let owner = if is_root { UseOwner::CrateRoot } else { UseOwner::Def(mod_gid.index) };
    let arena = reader.ty_arena();
    for u in reader.uses_for_owner(owner) {
        if (u.flags & USE_FLAG_GLOB) == 0 && norm_ident(u.name) == seg {
            if let Some(td) = u.type_def {
                let g = db.resolve_ty_def(r_idx, arena, td);
                if g.is_some() {
                    return Some(db.canonical_def(g));
                }
            }
        }
    }
    for u in reader.uses_for_owner(owner) {
        if (u.flags & USE_FLAG_GLOB) != 0 {
            if let Some(td) = u.type_def {
                let g = db.resolve_ty_def(r_idx, arena, td);
                if g.is_some() {
                    let tgt = db.canonical_def(g);
                    if tgt != mod_gid {
                        if let Some(hit) = step_mod_in_mod(db, tgt, seg, depth + 1, visited) {
                            return Some(hit);
                        }
                    }
                }
            }
        }
    }
    None
}

fn find_macro_in_mod<'w>(
    db: &'w RidxV5ItemDb<'w>,
    caller_r_idx: usize,
    mod_gid: GlobalDefId,
    mac_name: &str,
    depth: u8,
    visited: &mut Vec<GlobalDefId>,
) -> Option<(&'w str, &'w str)> {
    if !mod_gid.is_some() || depth > 5 || visited.contains(&mod_gid) {
        return None;
    }
    visited.push(mod_gid);
    let tr = mod_gid.krate.0 as usize;
    let reader = db.ws.readers().get(tr)?;
    let shard = db.shards.get(tr)?;
    let is_root = mod_gid.index == CRATE_ROOT_DEF_IDX || Some(mod_gid.index) == shard.root_mod_def_idx;
    let key = if is_root { shard.root_mod_def_idx.unwrap_or(CRATE_ROOT_DEF_IDX) } else { mod_gid.index };
    let defs = reader.definitions();
    if let Some(children) = shard.mod_children.get(&key) {
        for &didx in children {
            if didx < PROC_MACRO_SYNTH_BIT {
                if let Some(d) = defs.get(didx as usize) {
                    if DefKind::from(d.kind) == DefKind::Macro && norm_ident(reader.get_str(d.name_offset())) == mac_name {
                        if let Some(hit) = body_of_macro_gid(db, caller_r_idx, GlobalDefId::new(mod_gid.krate, didx)) {
                            return Some(hit);
                        }
                    }
                }
            }
        }
    }
    let owner = if is_root { UseOwner::CrateRoot } else { UseOwner::Def(mod_gid.index) };
    let arena = reader.ty_arena();
    for u in reader.uses_for_owner(owner) {
        if (u.flags & USE_FLAG_GLOB) == 0 && norm_ident(u.name) == mac_name {
            if let Some(md) = u.macro_def {
                let g = db.resolve_ty_def(tr, arena, md);
                if g.is_some() {
                    if let Some(hit) = body_of_macro_gid(db, caller_r_idx, g) {
                        return Some(hit);
                    }
                }
            }
        }
    }
    for u in reader.uses_for_owner(owner) {
        if (u.flags & USE_FLAG_GLOB) != 0 {
            if let Some(td) = u.type_def {
                let g = db.resolve_ty_def(tr, arena, td);
                if g.is_some() {
                    let tgt = db.canonical_def(g);
                    if tgt != mod_gid {
                        if let Some(hit) = find_macro_in_mod(db, caller_r_idx, tgt, mac_name, depth + 1, visited) {
                            return Some(hit);
                        }
                    }
                }
            }
        }
    }
    if is_root {
        if let Some(body) = shard.lookup_macro_rules(reader, mac_name) {
            return Some((body, dollar_crate_for_target(db, caller_r_idx, tr)));
        }
    }
    None
}

pub fn body_of_macro_gid<'w>(db: &'w RidxV5ItemDb<'w>, caller_r_idx: usize, raw_gid: GlobalDefId) -> Option<(&'w str, &'w str)> {
    let mut g = raw_gid;
    for _ in 0..4 {
        let next = db.canonical_def(g);
        if next == g || !next.is_some() {
            break;
        }
        g = next;
    }
    if !g.is_some() {
        return None;
    }
    let tr = g.krate.0 as usize;
    let reader = db.ws.readers().get(tr)?;
    let mv = reader.macro_def_for_def(g.index)?;
    if !mv.is_macro_rules() {
        return None;
    }
    Some((mv.body, dollar_crate_for_target(db, caller_r_idx, tr)))
}

fn dollar_crate_for_target<'w>(db: &'w RidxV5ItemDb<'w>, caller_r_idx: usize, target_r_idx: usize) -> &'w str {
    if caller_r_idx == target_r_idx {
        return "crate";
    }
    if let Some(meta) = db.metas.get(caller_r_idx) {
        for (alias, uk) in &meta.extern_units {
            if db.resolve_unit_to_reader(caller_r_idx, uk) == Some(target_r_idx) {
                return alias.as_str();
            }
        }
    }
    if let Some(tmeta) = db.metas.get(target_r_idx) {
        if tmeta.is_sysroot {
            return tmeta.crate_name.as_str();
        }
    }
    "crate"
}
