/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Targeted audit of sysroot index shards (`core`, `alloc`, `std`).

use std::fs;

use crate::format::{fnv1a_64, DefKind};
use crate::reader::IndexReader;
use crate::workspace::WorkspaceIndex;

use super::{audit_workspace_items, def_kind_slot, is_primitive_owner, is_sysroot_shard, AuditScope, GlobalKindSets, ItemAuditReport};

/// Per-shard statistics for a sysroot `.ridx` shard (`core`, `alloc`, `std`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SysrootShardSummary {
    pub crate_name: String,
    pub file_size: u64,
    pub string_pool_bytes: u64,
    pub defs_count: u32,
    pub refs_count: u32,
    pub types_count: u32,
    pub notes_count: u32,
    pub derefs_count: u32,
    pub owners_count: u32,
    pub fields_count: u64,
    pub variants_count: u64,
    pub ty_params_count: u64,
    pub impls_count: u64,
    pub reexports_count: u64,
}

/// Detailed probe of a canonical owner/type in the sysroot index.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SysrootOwnerProbe {
    pub target: String,
    pub def_present: bool,
    pub def_kind: Option<DefKind>,
    pub def_has_sig: bool,
    pub reexport_aliases: u32,
    pub owned_assoc_fns: u32,
    pub assoc_fns_with_self: u32,
    pub assoc_fns_unqualified_ret: u32,
    pub owned_fields: u32,
    pub owned_variants: u32,
    pub variants_with_sig: u32,
    pub owned_assoc_tys: u32,
    pub assoc_tys_with_sig: u32,
    pub owned_ty_params: u32,
    pub actual_deref_targets: Vec<String>,
    pub trait_impl_targets: u32,
    pub supertrait_targets: u32,
    pub sample_sigs: Vec<(String, String)>,
}

/// Summary across `core::iter::adapters::*` structs in sysroot.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IteratorAdaptersAudit {
    pub adapter_structs: u32,
    pub adapters_with_iterator_edge: u32,
    pub adapters_with_item_assoc_ty: u32,
    pub adapters_with_ty_params: u32,
    pub total_inherent_assoc_fns: u32,
    pub adapter_names: Vec<String>,
}

/// Full sysroot audit report.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SysrootAuditReport {
    pub shards: Vec<SysrootShardSummary>,
    pub items: ItemAuditReport,
    pub probes: Vec<SysrootOwnerProbe>,
    pub iter_adapters: IteratorAdaptersAudit,
}

/// Default canonical targets audited for sysroot (`0.2`).
pub const DEFAULT_SYSROOT_TARGETS: &[&str] = &[
    "alloc::vec::Vec",
    "core::option::Option",
    "core::result::Result",
    "std::io::error::Result",
    "alloc::string::String",
    "core::iter::traits::iterator::Iterator",
    "slice",
    "core::slice::iter::Iter",
    "str",
    "core::ops::range::Range",
];

fn has_unqualified_adt_return(sig: &str) -> bool {
    let Some((_, ret)) = sig.split_once("->") else {
        return false;
    };
    let ret = ret.trim().trim_start_matches('&').trim_start_matches("mut ").trim();
    let head: String = ret.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == ':').collect();
    if head.is_empty() || head.contains("::") || head == "Self" || head.len() == 1 || is_primitive_owner(&head) {
        return false;
    }
    head.as_bytes()[0].is_ascii_uppercase()
}

/// Audits attached sysroot shards in `ws` (or all shards if `ws` was opened directly on a sysroot directory).
pub fn audit_sysroot(ws: &WorkspaceIndex, targets: &[&str]) -> SysrootAuditReport {
    let any_marked_sys = (0..ws.readers().len()).any(|i| is_sysroot_shard(ws, i));
    let scope = if any_marked_sys { AuditScope::SysrootOnly } else { AuditScope::All };
    let items = audit_workspace_items(ws, scope);

    let mut sets = GlobalKindSets::default();
    let mut sys_indices = Vec::new();
    for (idx, reader) in ws.readers().iter().enumerate() {
        sets.record_reader(reader);
        if !any_marked_sys || is_sysroot_shard(ws, idx) {
            sys_indices.push(idx);
        }
    }

    let mut shards = Vec::with_capacity(sys_indices.len());
    for &idx in &sys_indices {
        let reader = &ws.readers()[idx];
        let h = reader.header();
        let file_size = ws.paths().get(idx).and_then(|p| fs::metadata(p).ok()).map_or(0, |m| m.len());
        let single = super::audit_reader_items(reader);
        shards.push(SysrootShardSummary {
            crate_name: reader.crate_name().to_string(),
            file_size,
            string_pool_bytes: h.string_pool_len,
            defs_count: h.defs_count,
            refs_count: h.refs_count,
            types_count: h.types_count,
            notes_count: h.notes_count,
            derefs_count: h.derefs_count,
            owners_count: h.defs_by_owner_count,
            fields_count: single.by_kind[def_kind_slot(DefKind::Field)].total,
            variants_count: single.by_kind[def_kind_slot(DefKind::Variant)].total,
            ty_params_count: single.by_kind[def_kind_slot(DefKind::TyParam)].total,
            impls_count: single.by_kind[def_kind_slot(DefKind::Impl)].total,
            reexports_count: single.reexports.flag_reexport_defs,
        });
    }

    let sys_readers: Vec<&IndexReader<_>> = sys_indices.iter().map(|&i| &ws.readers()[i]).collect();
    let probes = targets.iter().map(|&t| probe_owner(&sys_readers, &sets, t)).collect();
    let iter_adapters = audit_iterator_adapters(&sys_readers, &sets);

    SysrootAuditReport { shards, items, probes, iter_adapters }
}

pub(crate) fn probe_owner<B: AsRef<[u8]>>(readers: &[&IndexReader<B>], sets: &GlobalKindSets, target: &str) -> SysrootOwnerProbe {
    let mut p = SysrootOwnerProbe { target: target.to_string(), ..Default::default() };
    let target_hash = fnv1a_64(target.as_bytes());
    let src_is_trait = sets.traits.contains(&target_hash);
    let mut methods: Vec<(String, String)> = Vec::new();

    for reader in readers {
        let defs = reader.definitions();
        for raw in defs {
            let d = raw.from_le();
            let path = reader.get_str(d.path_offset);
            if raw.is_reexport() {
                if d.sig_offset != 0 && reader.get_str(d.sig_offset) == target {
                    p.reexport_aliases += 1;
                }
                continue;
            }
            if path == target {
                p.def_present = true;
                p.def_kind = Some(DefKind::from(d.kind));
                p.def_has_sig = d.sig_offset != 0;
            }
        }

        let owners = reader.defs_by_owner();
        let from = owners.partition_point(|o| o.hash() < target_hash);
        for oe in owners[from..].iter().take_while(|o| o.hash() == target_hash) {
            if reader.get_str(oe.owner_path_offset()) != target {
                continue;
            }
            let Some(raw_def) = defs.get(oe.def_index() as usize) else {
                continue;
            };
            if raw_def.is_reexport() {
                continue;
            }
            let d = raw_def.from_le();
            match DefKind::from(d.kind) {
                DefKind::AssocFn => {
                    p.owned_assoc_fns += 1;
                    let sig = if d.sig_offset != 0 { reader.get_str(d.sig_offset) } else { "" };
                    if sig.contains("(&self") || sig.contains("(&mut self") || sig.contains("(self") {
                        p.assoc_fns_with_self += 1;
                    }
                    if has_unqualified_adt_return(sig) {
                        p.assoc_fns_unqualified_ret += 1;
                    }
                    methods.push((reader.get_str(d.name_offset).to_string(), sig.to_string()));
                }
                DefKind::Field => p.owned_fields += 1,
                DefKind::Variant => {
                    p.owned_variants += 1;
                    p.variants_with_sig += u32::from(d.sig_offset != 0);
                }
                DefKind::AssocTy => {
                    p.owned_assoc_tys += 1;
                    p.assoc_tys_with_sig += u32::from(d.sig_offset != 0);
                }
                DefKind::TyParam => p.owned_ty_params += 1,
                _ => {}
            }
        }

        let derefs = reader.deref_entries();
        let d_from = derefs.partition_point(|e| e.source_type_hash() < target_hash);
        for de in derefs[d_from..].iter().take_while(|e| e.source_type_hash() == target_hash) {
            if reader.get_str(de.source_type_offset()) != target {
                continue;
            }
            let tgt = reader.get_str(de.target_type_offset());
            let tgt_h = fnv1a_64(tgt.as_bytes());
            if sets.traits.contains(&tgt_h) {
                if src_is_trait {
                    p.supertrait_targets += 1;
                } else {
                    p.trait_impl_targets += 1;
                }
            } else if !p.actual_deref_targets.iter().any(|x| x == tgt) {
                p.actual_deref_targets.push(tgt.to_string());
            }
        }
    }

    methods.sort_by(|a, b| a.0.cmp(&b.0));
    methods.dedup_by(|a, b| a.0 == b.0);
    p.sample_sigs = methods.into_iter().take(4).collect();
    p
}

fn audit_iterator_adapters<B: AsRef<[u8]>>(readers: &[&IndexReader<B>], sets: &GlobalKindSets) -> IteratorAdaptersAudit {
    const ITER_TRAIT: &str = "core::iter::traits::iterator::Iterator";
    let mut adapter_paths: Vec<(String, String)> = Vec::new();
    for reader in readers {
        for raw in reader.definitions() {
            if raw.is_reexport() {
                continue;
            }
            let d = raw.from_le();
            if DefKind::from(d.kind) != DefKind::Struct {
                continue;
            }
            let path = reader.get_str(d.path_offset);
            if path.starts_with("core::iter::adapters::") {
                adapter_paths.push((reader.get_str(d.name_offset).to_string(), path.to_string()));
            }
        }
    }
    adapter_paths.sort_by(|a, b| a.1.cmp(&b.1));
    adapter_paths.dedup_by(|a, b| a.1 == b.1);

    let iter_hash = fnv1a_64(ITER_TRAIT.as_bytes());
    let mut out = IteratorAdaptersAudit { adapter_structs: adapter_paths.len() as u32, ..Default::default() };

    for (name, path) in adapter_paths {
        let probe = probe_owner(readers, sets, &path);
        out.total_inherent_assoc_fns += probe.owned_assoc_fns;
        out.adapters_with_item_assoc_ty += u32::from(probe.owned_assoc_tys > 0);
        out.adapters_with_ty_params += u32::from(probe.owned_ty_params > 0);

        let path_h = fnv1a_64(path.as_bytes());
        let mut impls_iter = false;
        for reader in readers {
            let derefs = reader.deref_entries();
            let from = derefs.partition_point(|e| e.source_type_hash() < path_h);
            for de in derefs[from..].iter().take_while(|e| e.source_type_hash() == path_h) {
                let tgt = reader.get_str(de.target_type_offset());
                if fnv1a_64(tgt.as_bytes()) == iter_hash && tgt == ITER_TRAIT {
                    impls_iter = true;
                    break;
                }
            }
        }
        out.adapters_with_iterator_edge += u32::from(impls_iter);
        out.adapter_names.push(name);
    }
    out
}
