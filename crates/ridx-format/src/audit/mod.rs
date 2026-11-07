/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Schema and empirical content audit for `.ridx` shards and workspaces.

pub mod sizing;
pub mod sizing_types;
pub mod sysroot;
pub mod types;

pub use self::sizing::*;
pub use self::sizing_types::*;
pub use self::sysroot::*;
pub use self::types::*;

use std::collections::HashSet;

use crate::format::{fnv1a_64, DefKind};
use crate::reader::IndexReader;
use crate::workspace::WorkspaceIndex;

const OWN_ANY: u16 = 1 << 0;
const OWN_TRAIT: u16 = 1 << 1;
const OWN_STRUCT: u16 = 1 << 2;
const OWN_ENUM: u16 = 1 << 3;
const OWN_UNION: u16 = 1 << 4;
const OWN_VARIANT: u16 = 1 << 5;
const OWN_IMPL: u16 = 1 << 6;
const OWN_TYPE_OTHER: u16 = 1 << 7;
const OWN_SCOPE: u16 = 1 << 8;

#[derive(Default)]
pub(crate) struct GlobalKindSets {
    pub traits: HashSet<u64>,
    pub structs: HashSet<u64>,
    pub enums: HashSet<u64>,
    pub unions: HashSet<u64>,
    pub variants: HashSet<u64>,
    pub impls: HashSet<u64>,
    pub other_types: HashSet<u64>,
}

impl GlobalKindSets {
    pub fn record_reader<B: AsRef<[u8]>>(&mut self, reader: &IndexReader<B>) {
        for raw in reader.definitions() {
            let d = raw.from_le();
            let path = reader.get_str(d.path_offset);
            if path.is_empty() {
                continue;
            }
            let h = fnv1a_64(path.as_bytes());
            match DefKind::from(d.kind) {
                DefKind::Trait | DefKind::TraitAlias => {
                    self.traits.insert(h);
                }
                DefKind::Struct => {
                    self.structs.insert(h);
                }
                DefKind::Enum => {
                    self.enums.insert(h);
                }
                DefKind::Union => {
                    self.unions.insert(h);
                }
                DefKind::Variant => {
                    self.variants.insert(h);
                }
                DefKind::Impl => {
                    self.impls.insert(h);
                }
                DefKind::TyAlias | DefKind::ForeignTy | DefKind::OpaqueTy => {
                    self.other_types.insert(h);
                }
                _ => {}
            }
        }
    }

    #[inline]
    pub fn is_type_path(&self, path: &str, hash: u64) -> bool {
        is_primitive_owner(path) || self.structs.contains(&hash) || self.enums.contains(&hash) || self.unions.contains(&hash) || self.other_types.contains(&hash)
    }

    #[inline]
    fn classify_owner(&self, owner: &str) -> u16 {
        if owner.is_empty() {
            return 0;
        }
        let mut mask = OWN_ANY;
        if owner.starts_with("scope:") {
            mask |= OWN_SCOPE;
        }
        let h = fnv1a_64(owner.as_bytes());
        if self.traits.contains(&h) {
            mask |= OWN_TRAIT;
        }
        if self.structs.contains(&h) {
            mask |= OWN_STRUCT;
        }
        if self.enums.contains(&h) {
            mask |= OWN_ENUM;
        }
        if self.unions.contains(&h) {
            mask |= OWN_UNION;
        }
        if self.variants.contains(&h) {
            mask |= OWN_VARIANT;
        }
        if self.impls.contains(&h) {
            mask |= OWN_IMPL;
        }
        if is_primitive_owner(owner) || self.other_types.contains(&h) {
            mask |= OWN_TYPE_OTHER;
        }
        mask
    }
}

/// Audits a single `IndexReader` shard.
pub fn audit_reader_items<B: AsRef<[u8]>>(reader: &IndexReader<B>) -> ItemAuditReport {
    let mut sets = GlobalKindSets::default();
    sets.record_reader(reader);
    let mut report = ItemAuditReport { included_shards: 1, workspace_shards: 1, ..Default::default() };
    accumulate_reader(reader, &sets, &mut report);
    report
}

/// Audits a `WorkspaceIndex` using all loaded shards for symbol-kind disambiguation.
pub fn audit_workspace_items(ws: &WorkspaceIndex, scope: AuditScope) -> ItemAuditReport {
    let mut sets = GlobalKindSets::default();
    for reader in ws.readers() {
        sets.record_reader(reader);
    }

    let mut report = ItemAuditReport::default();
    for (idx, reader) in ws.readers().iter().enumerate() {
        let is_sys = is_sysroot_shard(ws, idx);
        let is_ext = ws.is_reader_external(idx);
        let include = match scope {
            AuditScope::NonSysroot => !is_sys,
            AuditScope::WorkspaceOnly => !is_sys && !is_ext,
            AuditScope::SysrootOnly => is_sys,
            AuditScope::All => true,
        };
        if !include {
            continue;
        }
        report.included_shards += 1;
        if is_sys {
            report.sysroot_shards += 1;
        } else if is_ext {
            report.dep_shards += 1;
        } else {
            report.workspace_shards += 1;
        }
        accumulate_reader(reader, &sets, &mut report);
    }
    report
}

fn accumulate_reader<B: AsRef<[u8]>>(reader: &IndexReader<B>, sets: &GlobalKindSets, out: &mut ItemAuditReport) {
    let defs = reader.definitions();
    let mut owner_masks = vec![0u16; defs.len()];
    for raw_owner in reader.defs_by_owner() {
        let oe = raw_owner.from_le();
        let idx = oe.def_index as usize;
        if idx < owner_masks.len() && oe.owner_path_offset != 0 {
            owner_masks[idx] |= sets.classify_owner(reader.get_str(oe.owner_path_offset));
        }
    }

    for (idx, raw) in defs.iter().enumerate() {
        let d = raw.from_le();
        let kind = DefKind::from(d.kind);
        let has_sig = d.sig_offset != 0;
        let mask = owner_masks[idx];

        out.total_defs += 1;
        let row = &mut out.by_kind[def_kind_slot(kind)];
        row.total += 1;
        row.with_sig += u64::from(has_sig);
        if d.visibility != 0 {
            row.non_public_vis += 1;
            out.reexports.non_public_vis_defs += 1;
        }
        row.macro_exp += u64::from(raw.is_macro_expansion());
        if raw.is_reexport() {
            row.reexport_flag += 1;
            out.reexports.flag_reexport_defs += 1;
        }
        row.owned_defs += u64::from((mask & OWN_ANY) != 0);
        out.impls_traits.impl_used_as_owner += u64::from((mask & OWN_IMPL) != 0);

        audit_single_def(reader, &d, kind, has_sig, mask, out);
    }

    for raw_deref in reader.deref_entries() {
        let de = raw_deref.from_le();
        let src = reader.get_str(de.source_type_offset);
        let tgt = reader.get_str(de.target_type_offset);
        let tgt_h = fnv1a_64(tgt.as_bytes());
        out.derefs.total_entries += 1;
        let src_is_trait = sets.traits.contains(&de.source_type_hash);
        let tgt_is_trait = sets.traits.contains(&tgt_h);
        if src_is_trait && tgt_is_trait {
            out.derefs.supertrait_edges += 1;
        } else if !src_is_trait && tgt_is_trait {
            out.derefs.trait_impl_edges += 1;
        } else if !src_is_trait && sets.is_type_path(tgt, tgt_h) && !src.is_empty() {
            out.derefs.actual_deref_to_type += 1;
        } else {
            out.derefs.unresolved_target_edges += 1;
        }
    }
}

fn audit_single_def<B: AsRef<[u8]>>(reader: &IndexReader<B>, d: &crate::format::DefEntry, kind: DefKind, has_sig: bool, mask: u16, out: &mut ItemAuditReport) {
    match kind {
        DefKind::Fn | DefKind::AssocFn => {
            let is_assoc = kind == DefKind::AssocFn;
            if is_assoc {
                out.fn_sigs.assoc_fn_total += 1;
                if (mask & OWN_TRAIT) != 0 {
                    out.impls_traits.assoc_fn_owned_by_trait += 1;
                } else if (mask & OWN_ANY) != 0 {
                    out.impls_traits.assoc_fn_owned_by_type += 1;
                }
            } else {
                out.fn_sigs.fn_total += 1;
            }
            if has_sig {
                let sig = reader.get_str(d.sig_offset);
                out.fn_sigs.total_sig_bytes += sig.len() as u64;
                if is_assoc {
                    out.fn_sigs.assoc_fn_with_sig += 1;
                    if sig.contains("(&self") || sig.contains("(&mut self") || sig.contains("(self") {
                        out.fn_sigs.assoc_fn_with_self += 1;
                    }
                } else {
                    out.fn_sigs.fn_with_sig += 1;
                }
                out.fn_sigs.sigs_with_where += u64::from(sig.contains(" where "));
                out.fn_sigs.sigs_with_async_or_const += u64::from(sig.starts_with("async ") || sig.starts_with("const "));
                out.fn_sigs.sigs_with_unsafe_or_extern += u64::from(sig.starts_with("unsafe ") || sig.starts_with("extern "));
                out.fn_sigs.sigs_with_lifetime += u64::from(sig.contains('\''));
                out.fn_sigs.sigs_with_impl_or_dyn += u64::from(sig.contains("impl ") || sig.contains("dyn "));
            }
        }
        DefKind::Field => {
            let name = reader.get_str(d.name_offset);
            if !name.is_empty() && name.bytes().all(|b| b.is_ascii_digit()) {
                out.fields.tuple_fields += 1;
            } else {
                out.fields.named_fields += 1;
            }
            out.fields.fields_with_sig += u64::from(has_sig);
            out.fields.owner_has_struct += u64::from((mask & OWN_STRUCT) != 0);
            out.fields.owner_has_enum += u64::from((mask & OWN_ENUM) != 0);
            out.fields.owner_has_union += u64::from((mask & OWN_UNION) != 0);
            out.fields.owner_has_variant += u64::from((mask & OWN_VARIANT) != 0);
            if (mask & (OWN_STRUCT | OWN_ENUM | OWN_UNION | OWN_VARIANT)) == 0 {
                out.fields.owner_unresolved += 1;
            }
        }
        DefKind::StructCtor => {
            out.fields.struct_ctors += 1;
            out.fields.ctors_with_sig += u64::from(has_sig);
        }
        DefKind::VariantCtor => {
            out.fields.variant_ctors += 1;
            out.fields.ctors_with_sig += u64::from(has_sig);
        }
        DefKind::Impl => {
            out.impls_traits.impl_defs += 1;
            out.impls_traits.impl_with_sig += u64::from(has_sig);
        }
        DefKind::Trait => out.impls_traits.traits += 1,
        DefKind::TraitAlias => out.impls_traits.trait_aliases += 1,
        DefKind::AssocTy => {
            out.impls_traits.assoc_ty_total += 1;
            if (mask & OWN_TRAIT) != 0 {
                out.impls_traits.assoc_ty_owned_by_trait += 1;
                out.impls_traits.assoc_ty_owned_by_trait_with_sig += u64::from(has_sig);
            } else {
                out.impls_traits.assoc_ty_owned_by_type += 1;
                out.impls_traits.assoc_ty_owned_by_type_with_sig += u64::from(has_sig);
            }
        }
        DefKind::AssocConst => {
            out.impls_traits.assoc_const_total += 1;
            out.impls_traits.assoc_const_with_sig += u64::from(has_sig);
        }
        DefKind::TyAlias => {
            out.impls_traits.ty_alias_total += 1;
            out.impls_traits.ty_alias_with_sig += u64::from(has_sig);
        }
        DefKind::Const | DefKind::Static => {
            out.impls_traits.const_or_static_total += 1;
            out.impls_traits.const_or_static_with_sig += u64::from(has_sig);
        }
        DefKind::TyParam => {
            out.generics.ty_params += 1;
            out.generics.ty_params_with_bounds += u64::from(has_sig);
            out.generics.ty_params_scope_owned += u64::from((mask & OWN_SCOPE) != 0);
        }
        DefKind::LifetimeParam => {
            out.generics.lifetime_params += 1;
            out.generics.lifetime_params_with_sig += u64::from(has_sig);
        }
        DefKind::ConstParam => {
            out.generics.const_params += 1;
            out.generics.const_params_with_sig += u64::from(has_sig);
        }
        DefKind::Use => {
            out.reexports.use_defs += 1;
            out.reexports.use_with_sig += u64::from(has_sig);
            out.reexports.use_glob += u64::from(reader.get_str(d.name_offset) == "*");
        }
        _ => {}
    }
}
