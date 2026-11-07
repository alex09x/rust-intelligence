/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! CLI tool to audit `.ridx` item schemas and empirical statistics across workspaces.

use std::env;
use std::path::Path;
use std::process;

use ridx_format::audit::{audit_workspace_items, def_kind_from_slot, AuditScope, ItemAuditReport};
use ridx_format::WorkspaceIndex;

fn parse_scope(s: &str) -> AuditScope {
    match s {
        "non-sysroot" => AuditScope::NonSysroot,
        "workspace" => AuditScope::WorkspaceOnly,
        "sysroot" => AuditScope::SysrootOnly,
        "all" => AuditScope::All,
        other => {
            eprintln!("Unknown --scope '{other}' (expected non-sysroot|workspace|sysroot|all)");
            process::exit(2);
        }
    }
}

fn merge_reports(dst: &mut ItemAuditReport, src: &ItemAuditReport) {
    dst.included_shards += src.included_shards;
    dst.workspace_shards += src.workspace_shards;
    dst.dep_shards += src.dep_shards;
    dst.sysroot_shards += src.sysroot_shards;
    dst.total_defs += src.total_defs;
    for (d, s) in dst.by_kind.iter_mut().zip(src.by_kind.iter()) {
        d.total += s.total;
        d.with_sig += s.with_sig;
        d.non_public_vis += s.non_public_vis;
        d.macro_exp += s.macro_exp;
        d.reexport_flag += s.reexport_flag;
        d.owned_defs += s.owned_defs;
    }
    dst.fn_sigs.fn_total += src.fn_sigs.fn_total;
    dst.fn_sigs.fn_with_sig += src.fn_sigs.fn_with_sig;
    dst.fn_sigs.assoc_fn_total += src.fn_sigs.assoc_fn_total;
    dst.fn_sigs.assoc_fn_with_sig += src.fn_sigs.assoc_fn_with_sig;
    dst.fn_sigs.assoc_fn_with_self += src.fn_sigs.assoc_fn_with_self;
    dst.fn_sigs.sigs_with_where += src.fn_sigs.sigs_with_where;
    dst.fn_sigs.sigs_with_async_or_const += src.fn_sigs.sigs_with_async_or_const;
    dst.fn_sigs.sigs_with_unsafe_or_extern += src.fn_sigs.sigs_with_unsafe_or_extern;
    dst.fn_sigs.sigs_with_lifetime += src.fn_sigs.sigs_with_lifetime;
    dst.fn_sigs.sigs_with_impl_or_dyn += src.fn_sigs.sigs_with_impl_or_dyn;
    dst.fn_sigs.total_sig_bytes += src.fn_sigs.total_sig_bytes;

    dst.fields.named_fields += src.fields.named_fields;
    dst.fields.tuple_fields += src.fields.tuple_fields;
    dst.fields.fields_with_sig += src.fields.fields_with_sig;
    dst.fields.owner_has_struct += src.fields.owner_has_struct;
    dst.fields.owner_has_enum += src.fields.owner_has_enum;
    dst.fields.owner_has_union += src.fields.owner_has_union;
    dst.fields.owner_has_variant += src.fields.owner_has_variant;
    dst.fields.owner_unresolved += src.fields.owner_unresolved;
    dst.fields.struct_ctors += src.fields.struct_ctors;
    dst.fields.variant_ctors += src.fields.variant_ctors;
    dst.fields.ctors_with_sig += src.fields.ctors_with_sig;

    dst.impls_traits.impl_defs += src.impls_traits.impl_defs;
    dst.impls_traits.impl_with_sig += src.impls_traits.impl_with_sig;
    dst.impls_traits.impl_used_as_owner += src.impls_traits.impl_used_as_owner;
    dst.impls_traits.traits += src.impls_traits.traits;
    dst.impls_traits.trait_aliases += src.impls_traits.trait_aliases;
    dst.impls_traits.assoc_fn_owned_by_trait += src.impls_traits.assoc_fn_owned_by_trait;
    dst.impls_traits.assoc_fn_owned_by_type += src.impls_traits.assoc_fn_owned_by_type;
    dst.impls_traits.assoc_ty_total += src.impls_traits.assoc_ty_total;
    dst.impls_traits.assoc_ty_owned_by_trait += src.impls_traits.assoc_ty_owned_by_trait;
    dst.impls_traits.assoc_ty_owned_by_trait_with_sig += src.impls_traits.assoc_ty_owned_by_trait_with_sig;
    dst.impls_traits.assoc_ty_owned_by_type += src.impls_traits.assoc_ty_owned_by_type;
    dst.impls_traits.assoc_ty_owned_by_type_with_sig += src.impls_traits.assoc_ty_owned_by_type_with_sig;
    dst.impls_traits.assoc_const_total += src.impls_traits.assoc_const_total;
    dst.impls_traits.assoc_const_with_sig += src.impls_traits.assoc_const_with_sig;
    dst.impls_traits.ty_alias_total += src.impls_traits.ty_alias_total;
    dst.impls_traits.ty_alias_with_sig += src.impls_traits.ty_alias_with_sig;
    dst.impls_traits.const_or_static_total += src.impls_traits.const_or_static_total;
    dst.impls_traits.const_or_static_with_sig += src.impls_traits.const_or_static_with_sig;

    dst.generics.ty_params += src.generics.ty_params;
    dst.generics.ty_params_with_bounds += src.generics.ty_params_with_bounds;
    dst.generics.ty_params_scope_owned += src.generics.ty_params_scope_owned;
    dst.generics.lifetime_params += src.generics.lifetime_params;
    dst.generics.lifetime_params_with_sig += src.generics.lifetime_params_with_sig;
    dst.generics.const_params += src.generics.const_params;
    dst.generics.const_params_with_sig += src.generics.const_params_with_sig;

    dst.derefs.total_entries += src.derefs.total_entries;
    dst.derefs.actual_deref_to_type += src.derefs.actual_deref_to_type;
    dst.derefs.trait_impl_edges += src.derefs.trait_impl_edges;
    dst.derefs.supertrait_edges += src.derefs.supertrait_edges;
    dst.derefs.unresolved_target_edges += src.derefs.unresolved_target_edges;

    dst.reexports.use_defs += src.reexports.use_defs;
    dst.reexports.use_with_sig += src.reexports.use_with_sig;
    dst.reexports.use_glob += src.reexports.use_glob;
    dst.reexports.flag_reexport_defs += src.reexports.flag_reexport_defs;
    dst.reexports.non_public_vis_defs += src.reexports.non_public_vis_defs;
}

fn print_items_report(label: &str, r: &ItemAuditReport) {
    println!("=== {label} ===");
    println!(
        "shards: total={} (ws={}, deps={}, sysroot={}) | total_defs={} | non_pub_vis={}",
        r.included_shards, r.workspace_shards, r.dep_shards, r.sysroot_shards, r.total_defs, r.reexports.non_public_vis_defs
    );
    println!(
        "fn_sigs: fn={}/{} assoc_fn={}/{} (self={}) | where={} async_or_const={} unsafe_or_extern={} lifetime={} impl_or_dyn={} sig_bytes={}",
        r.fn_sigs.fn_with_sig,
        r.fn_sigs.fn_total,
        r.fn_sigs.assoc_fn_with_sig,
        r.fn_sigs.assoc_fn_total,
        r.fn_sigs.assoc_fn_with_self,
        r.fn_sigs.sigs_with_where,
        r.fn_sigs.sigs_with_async_or_const,
        r.fn_sigs.sigs_with_unsafe_or_extern,
        r.fn_sigs.sigs_with_lifetime,
        r.fn_sigs.sigs_with_impl_or_dyn,
        r.fn_sigs.total_sig_bytes
    );
    println!(
        "fields: named={} tuple={} with_sig={} | own_struct={} own_enum={} own_union={} own_variant={} own_unres={} | struct_ctor={} variant_ctor={} ctor_sig={}",
        r.fields.named_fields,
        r.fields.tuple_fields,
        r.fields.fields_with_sig,
        r.fields.owner_has_struct,
        r.fields.owner_has_enum,
        r.fields.owner_has_union,
        r.fields.owner_has_variant,
        r.fields.owner_unresolved,
        r.fields.struct_ctors,
        r.fields.variant_ctors,
        r.fields.ctors_with_sig
    );
    println!(
        "impls_traits: impl={}(sig={},as_owner={}) trait={} trait_alias={} | assoc_fn(trait={},type={}) | assoc_ty={}(trait={}/{},type={}/{}) | assoc_const={}/{} ty_alias={}/{} const_static={}/{}",
        r.impls_traits.impl_defs, r.impls_traits.impl_with_sig, r.impls_traits.impl_used_as_owner,
        r.impls_traits.traits, r.impls_traits.trait_aliases,
        r.impls_traits.assoc_fn_owned_by_trait, r.impls_traits.assoc_fn_owned_by_type,
        r.impls_traits.assoc_ty_total,
        r.impls_traits.assoc_ty_owned_by_trait_with_sig, r.impls_traits.assoc_ty_owned_by_trait,
        r.impls_traits.assoc_ty_owned_by_type_with_sig, r.impls_traits.assoc_ty_owned_by_type,
        r.impls_traits.assoc_const_with_sig, r.impls_traits.assoc_const_total,
        r.impls_traits.ty_alias_with_sig, r.impls_traits.ty_alias_total,
        r.impls_traits.const_or_static_with_sig, r.impls_traits.const_or_static_total
    );
    println!(
        "generics: ty_param={}(bounds={},scope_own={}) lifetime={}(sig={}) const_param={}(sig={})",
        r.generics.ty_params,
        r.generics.ty_params_with_bounds,
        r.generics.ty_params_scope_owned,
        r.generics.lifetime_params,
        r.generics.lifetime_params_with_sig,
        r.generics.const_params,
        r.generics.const_params_with_sig
    );
    println!(
        "derefs: total={} | actual_deref={} trait_impl={} supertrait={} unresolved={}",
        r.derefs.total_entries, r.derefs.actual_deref_to_type, r.derefs.trait_impl_edges, r.derefs.supertrait_edges, r.derefs.unresolved_target_edges
    );
    println!("reexports: use={}(sig={},glob={}) flag_reexport={}", r.reexports.use_defs, r.reexports.use_with_sig, r.reexports.use_glob, r.reexports.flag_reexport_defs);
    println!("by_kind:");
    for (slot, row) in r.by_kind.iter().enumerate() {
        if row.total == 0 {
            continue;
        }
        let kind = def_kind_from_slot(slot);
        println!("  {:14?} total={:<8} with_sig={:<8} owned={:<8} macro={:<7} reexp={}", kind, row.total, row.with_sig, row.owned_defs, row.macro_exp, row.reexport_flag);
    }
}

fn main() {
    let mut args = env::args().skip(1);
    let mut mode = String::from("items");
    let mut scope = AuditScope::NonSysroot;
    let mut paths = Vec::new();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--mode" => mode = args.next().unwrap_or_else(|| "items".to_string()),
            "--scope" => scope = parse_scope(&args.next().unwrap_or_default()),
            other => paths.push(other.to_string()),
        }
    }

    if paths.is_empty() {
        eprintln!("Usage: index_audit [--mode items] [--scope non-sysroot|workspace|sysroot|all] <workspace_or_ridx>...");
        process::exit(2);
    }

    if mode != "items" {
        eprintln!("Unsupported --mode '{mode}'");
        process::exit(2);
    }

    let mut combined = ItemAuditReport::default();
    for p in &paths {
        let path = Path::new(p);
        let label = path.file_name().and_then(|s| s.to_str()).unwrap_or(p);
        let ws = WorkspaceIndex::open(path).unwrap_or_else(|e| {
            eprintln!("Failed to open {p}: {e}");
            process::exit(1);
        });
        let report = audit_workspace_items(&ws, scope);
        print_items_report(label, &report);
        merge_reports(&mut combined, &report);
    }
    if paths.len() > 1 {
        print_items_report("COMBINED", &combined);
    }
}
