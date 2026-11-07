/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Report data structures and helpers for `.ridx` index auditing.

use crate::format::DefKind;
use crate::workspace::WorkspaceIndex;

/// Subset of workspace shards to include in an item audit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditScope {
    /// Local workspace crates + Cargo.lock dependencies (excludes sysroot shards).
    NonSysroot,
    /// Local workspace crates only.
    WorkspaceOnly,
    /// Attached `core`/`alloc`/`std` sysroot shards only.
    SysrootOnly,
    /// Every loaded shard in the workspace.
    All,
}

/// Counts for one `DefKind` slot (`0..32` plus `32` for `DefKind::Other`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DefKindAuditRow {
    pub total: u64,
    pub with_sig: u64,
    pub non_public_vis: u64,
    pub macro_exp: u64,
    pub reexport_flag: u64,
    pub owned_defs: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FnSigAudit {
    pub fn_total: u64,
    pub fn_with_sig: u64,
    pub assoc_fn_total: u64,
    pub assoc_fn_with_sig: u64,
    pub assoc_fn_with_self: u64,
    pub sigs_with_where: u64,
    pub sigs_with_async_or_const: u64,
    pub sigs_with_unsafe_or_extern: u64,
    pub sigs_with_lifetime: u64,
    pub sigs_with_impl_or_dyn: u64,
    pub total_sig_bytes: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FieldAudit {
    pub named_fields: u64,
    pub tuple_fields: u64,
    pub fields_with_sig: u64,
    pub owner_has_struct: u64,
    pub owner_has_enum: u64,
    pub owner_has_union: u64,
    pub owner_has_variant: u64,
    pub owner_unresolved: u64,
    pub struct_ctors: u64,
    pub variant_ctors: u64,
    pub ctors_with_sig: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ImplTraitAudit {
    pub impl_defs: u64,
    pub impl_with_sig: u64,
    pub impl_used_as_owner: u64,
    pub traits: u64,
    pub trait_aliases: u64,
    pub assoc_fn_owned_by_trait: u64,
    pub assoc_fn_owned_by_type: u64,
    pub assoc_ty_total: u64,
    pub assoc_ty_owned_by_trait: u64,
    pub assoc_ty_owned_by_trait_with_sig: u64,
    pub assoc_ty_owned_by_type: u64,
    pub assoc_ty_owned_by_type_with_sig: u64,
    pub assoc_const_total: u64,
    pub assoc_const_with_sig: u64,
    pub ty_alias_total: u64,
    pub ty_alias_with_sig: u64,
    pub const_or_static_total: u64,
    pub const_or_static_with_sig: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GenericAudit {
    pub ty_params: u64,
    pub ty_params_with_bounds: u64,
    pub ty_params_scope_owned: u64,
    pub lifetime_params: u64,
    pub lifetime_params_with_sig: u64,
    pub const_params: u64,
    pub const_params_with_sig: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DerefAudit {
    pub total_entries: u64,
    pub actual_deref_to_type: u64,
    pub trait_impl_edges: u64,
    pub supertrait_edges: u64,
    pub unresolved_target_edges: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReexportAudit {
    pub use_defs: u64,
    pub use_with_sig: u64,
    pub use_glob: u64,
    pub flag_reexport_defs: u64,
    pub non_public_vis_defs: u64,
}

/// Aggregated item schema audit across one or more `.ridx` shards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemAuditReport {
    pub included_shards: usize,
    pub workspace_shards: usize,
    pub dep_shards: usize,
    pub sysroot_shards: usize,
    pub total_defs: u64,
    pub by_kind: [DefKindAuditRow; 33],
    pub fn_sigs: FnSigAudit,
    pub fields: FieldAudit,
    pub impls_traits: ImplTraitAudit,
    pub generics: GenericAudit,
    pub derefs: DerefAudit,
    pub reexports: ReexportAudit,
}

impl Default for ItemAuditReport {
    fn default() -> Self {
        Self {
            included_shards: 0,
            workspace_shards: 0,
            dep_shards: 0,
            sysroot_shards: 0,
            total_defs: 0,
            by_kind: [DefKindAuditRow::default(); 33],
            fn_sigs: FnSigAudit::default(),
            fields: FieldAudit::default(),
            impls_traits: ImplTraitAudit::default(),
            generics: GenericAudit::default(),
            derefs: DerefAudit::default(),
            reexports: ReexportAudit::default(),
        }
    }
}

/// Returns true if `idx` in `ws` is a cached sysroot shard (`core`, `alloc`, `std`).
pub fn is_sysroot_shard(ws: &WorkspaceIndex, idx: usize) -> bool {
    let Some(reader) = ws.readers().get(idx) else {
        return false;
    };
    if !matches!(reader.crate_name(), "core" | "alloc" | "std") {
        return false;
    }
    reader.unit_key().starts_with("sysroot:") || ws.source_root_at(idx).is_none() || ws.paths().get(idx).is_some_and(|p| p.to_string_lossy().contains("/sysroot/"))
}

/// Returns true if `s` names a built-in primitive owner string in `.ridx`.
pub fn is_primitive_owner(s: &str) -> bool {
    let bare = s.strip_prefix("core::primitive::").or_else(|| s.strip_prefix("std::primitive::")).unwrap_or(s);
    matches!(
        bare,
        "bool"
            | "char"
            | "str"
            | "slice"
            | "array"
            | "tuple"
            | "unit"
            | "pointer"
            | "reference"
            | "fn"
            | "never"
            | "i8"
            | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "isize"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "usize"
            | "f16"
            | "f32"
            | "f64"
            | "f128"
    )
}

#[inline]
pub fn def_kind_slot(kind: DefKind) -> usize {
    let raw = kind as u8 as usize;
    if raw < 32 {
        raw
    } else {
        32
    }
}

#[inline]
pub fn def_kind_from_slot(slot: usize) -> DefKind {
    if slot < 32 {
        DefKind::from(slot as u8)
    } else {
        DefKind::Other
    }
}
