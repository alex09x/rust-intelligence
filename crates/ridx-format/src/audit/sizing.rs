/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Per-corpus section size breakdown and empirical Phase 1 gap-table sizing.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::format::{fnv1a_64, join_resolving_overlap, DefEntry, DefKind, DerefEntry, FileEntry, HashIndexEntry, Header, NoteEntry, OwnerIndexEntry, RefEntry, TypeEntry};
use crate::reader::IndexReader;
use crate::workspace::WorkspaceIndex;

use super::sizing_types::*;
use super::{is_sysroot_shard, GlobalKindSets};

fn count_fn_params(sig: &str) -> u64 {
    let Some(open) = sig.find('(') else { return 0 };
    let rest = &sig.as_bytes()[open + 1..];
    let mut depth = 0i32;
    let mut commas = 0u64;
    let mut non_ws = false;
    for &b in rest {
        match b {
            b'(' | b'<' | b'[' => depth += 1,
            b')' if depth == 0 => break,
            b')' | b'>' | b']' => depth -= 1,
            b',' if depth == 0 => commas += 1,
            b' ' | b'\t' => {}
            _ => non_ws = true,
        }
    }
    if non_ws {
        commas + 1
    } else {
        0
    }
}

fn record_ty_expr(ty_str: &str, seen: &mut HashSet<u64>, arg_slots: &mut u64) {
    let t = ty_str.trim();
    if t.is_empty() {
        return;
    }
    if seen.insert(fnv1a_64(t.as_bytes())) {
        let constructors = t.bytes().filter(|&b| matches!(b, b'<' | b',' | b'&' | b'*' | b'[')).count() as u64;
        *arg_slots += constructors.max(1);
    }
    if let (Some(lt), Some(gt)) = (t.find('<'), t.rfind('>')) {
        if lt + 1 < gt {
            for part in t[lt + 1..gt].split(',') {
                let sub = part.trim();
                if !sub.is_empty() && seen.insert(fnv1a_64(sub.as_bytes())) {
                    *arg_slots += 1;
                }
            }
        }
    }
}

/// Computes current section sizes and empirical Phase 1 gap-table sizes across `ws`.
pub fn audit_workspace_sizing(ws: &WorkspaceIndex) -> SizingAuditReport {
    let mut sets = GlobalKindSets::default();
    for reader in ws.readers() {
        sets.record_reader(reader);
    }

    let mut seen_paths: HashSet<PathBuf> = HashSet::new();
    let mut seen_ws_files: HashSet<PathBuf> = HashSet::new();
    let mut rep = SizingAuditReport::default();

    for (idx, reader) in ws.readers().iter().enumerate() {
        if let Some(p) = ws.paths().get(idx) {
            if !seen_paths.insert(p.clone()) {
                continue;
            }
        }
        let is_sys = is_sysroot_shard(ws, idx);
        let is_ext = ws.is_reader_external(idx);
        let disk_bytes = ws.paths().get(idx).and_then(|p| fs::metadata(p).ok()).map_or(0, |m| m.len());
        let ws_src_root = if !is_sys && !is_ext { ws.source_root_at(idx) } else { None };

        let (sec, gap, lang) = audit_shard_sizing(reader, &sets, disk_bytes, ws_src_root, &mut seen_ws_files);
        rep.lang_items.add_assign(&lang);
        rep.total_sections.add_assign(&sec);
        rep.total_gaps.add_assign(&gap);
        if is_sys {
            rep.sysroot_sections.add_assign(&sec);
            rep.sysroot_gaps.add_assign(&gap);
        } else if is_ext {
            rep.dep_sections.add_assign(&sec);
            rep.dep_gaps.add_assign(&gap);
        } else {
            rep.ws_sections.add_assign(&sec);
            rep.ws_gaps.add_assign(&gap);
        }
    }
    rep
}

fn audit_shard_sizing<B: AsRef<[u8]>>(
    reader: &IndexReader<B>,
    sets: &GlobalKindSets,
    disk_bytes: u64,
    ws_src_root: Option<&Path>,
    seen_ws_files: &mut HashSet<PathBuf>,
) -> (TierSectionBytes, GapTableBytes, LangItemCounts) {
    let h = reader.header();
    let item_tables_bytes = std::mem::size_of::<Header>() as u64
        + u64::from(h.files_count) * std::mem::size_of::<FileEntry>() as u64
        + u64::from(h.defs_count) * std::mem::size_of::<DefEntry>() as u64
        + u64::from(h.defs_by_hash_count) * std::mem::size_of::<HashIndexEntry>() as u64
        + u64::from(h.defs_by_owner_count) * std::mem::size_of::<OwnerIndexEntry>() as u64
        + u64::from(h.derefs_count) * std::mem::size_of::<DerefEntry>() as u64;
    let body_refs_bytes = u64::from(h.refs_count) * std::mem::size_of::<RefEntry>() as u64 + u64::from(h.refs_by_target_count) * std::mem::size_of::<HashIndexEntry>() as u64;
    let body_types_notes_bytes = u64::from(h.types_count) * std::mem::size_of::<TypeEntry>() as u64 + u64::from(h.notes_count) * std::mem::size_of::<NoteEntry>() as u64;

    let mut seen_sig_offs: HashSet<u32> = HashSet::new();
    let mut unique_sig_string_bytes = 0u64;
    let mut seen_tys: HashSet<u64> = HashSet::new();
    let mut ty_arg_slots = 0u64;

    let mut fn_count = 0u64;
    let mut fn_params = 0u64;
    let mut named_fields = 0u64;
    let mut adts_and_variants = 0u64;
    let mut untyped_vals_and_aliases = 0u64;
    let mut generic_params = 0u64;
    let mut where_preds = 0u64;
    let mut impl_blocks = 0u64;
    let mut assoc_items = 0u64;
    let mut impl_assoc_tys = 0u64;
    let mut uses_and_reexports = 0u64;
    let mut macro_defs = 0u64;
    let mut macro_def_body_bytes = 0u64;
    let mut ws_macro_defs = 0u64;

    for raw in reader.definitions() {
        let d = raw.from_le();
        let kind = DefKind::from(d.kind);
        if raw.is_reexport() {
            uses_and_reexports += 1;
            continue;
        }
        if ws_src_root.is_some() && raw.is_macro_expansion() {
            ws_macro_defs += 1;
        }
        if d.sig_offset != 0 && matches!(kind, DefKind::Fn | DefKind::AssocFn | DefKind::Field | DefKind::TyParam | DefKind::AssocTy) && seen_sig_offs.insert(d.sig_offset) {
            unique_sig_string_bytes += reader.get_str(d.sig_offset).len() as u64 + 1;
        }
        match kind {
            DefKind::Fn | DefKind::AssocFn => {
                fn_count += 1;
                assoc_items += u64::from(kind == DefKind::AssocFn);
                if d.sig_offset != 0 {
                    let sig = reader.get_str(d.sig_offset);
                    fn_params += count_fn_params(sig);
                    generic_params += sig.matches('\'').count().min(3) as u64;
                    record_ty_expr(sig, &mut seen_tys, &mut ty_arg_slots);
                    if let Some((_, ret)) = sig.split_once("->") {
                        record_ty_expr(ret, &mut seen_tys, &mut ty_arg_slots);
                    }
                }
            }
            DefKind::Field => {
                named_fields += 1;
                if d.sig_offset != 0 {
                    record_ty_expr(reader.get_str(d.sig_offset), &mut seen_tys, &mut ty_arg_slots);
                }
            }
            DefKind::Struct | DefKind::Variant => adts_and_variants += 1,
            DefKind::TyAlias | DefKind::Const | DefKind::Static | DefKind::AssocConst => {
                untyped_vals_and_aliases += 1;
                assoc_items += u64::from(kind == DefKind::AssocConst);
            }
            DefKind::AssocTy => {
                assoc_items += 1;
                if d.sig_offset != 0 {
                    where_preds += reader.get_str(d.sig_offset).split('+').count() as u64;
                } else {
                    impl_assoc_tys += 1;
                }
            }
            DefKind::TyParam => {
                generic_params += 1;
                if d.sig_offset != 0 {
                    where_preds += reader.get_str(d.sig_offset).split('+').count() as u64;
                }
            }
            DefKind::LifetimeParam | DefKind::ConstParam => generic_params += 1,
            DefKind::Impl => impl_blocks += 1,
            DefKind::Use => uses_and_reexports += 1,
            DefKind::Macro => {
                macro_defs += 1;
                let lines = u64::from(d.span.end_line.saturating_sub(d.span.start_line));
                macro_def_body_bytes += (lines * 42 + 64).min(4096);
            }
            _ => {}
        }
    }

    let (lang, trait_impl_edges) = count_lang_items(reader, sets);
    where_preds += trait_impl_edges;
    let (ws_macro_callsites, ws_macro_refs) = count_ws_macros(reader, ws_src_root, seen_ws_files);

    let tuple_payload_est = adts_and_variants.div_ceil(2);
    let extra_ty_nodes = (untyped_vals_and_aliases + impl_assoc_tys + tuple_payload_est).div_ceil(3);
    let unique_ty_nodes = seen_tys.len() as u64 + extra_ty_nodes;
    let total_ty_args = ty_arg_slots + extra_ty_nodes;
    let typed_fields_and_vals = named_fields + tuple_payload_est + untyped_vals_and_aliases;

    let gap = GapTableBytes {
        g1_ty_arena_bytes: unique_ty_nodes * BYTES_TY_NODE + total_ty_args * BYTES_TY_ARG,
        g2_fn_sigs_bytes: fn_count * BYTES_FN_SIG + fn_params * BYTES_FN_PARAM,
        g3_fields_variants_vals_bytes: typed_fields_and_vals * BYTES_FIELD_OR_VAL_TY,
        g4_generics_bounds_bytes: generic_params * BYTES_GENERIC_PARAM + where_preds * BYTES_WHERE_PRED,
        g5_impls_assoc_bytes: impl_blocks * BYTES_IMPL_HEADER
            + assoc_items * BYTES_IMPL_ASSOC_REF
            + impl_assoc_tys * BYTES_ASSOC_TY_BINDING
            + trait_impl_edges * BYTES_TRAIT_IMPL_INDEX,
        g6_lang_items_bytes: lang.total() * BYTES_LANG_ITEM_ROW,
        g7_reexports_vis_cfg_bytes: uses_and_reexports * BYTES_USE_TARGET + BYTES_CRATE_CFG_META,
        g8_macros_bytes: macro_defs * BYTES_MACRO_DEF_HDR + macro_def_body_bytes + ws_macro_callsites * BYTES_MACRO_CALLSITE_HDR + ws_macro_defs * 40 + ws_macro_refs * 24,
        unique_ty_nodes,
        fn_count,
        fn_params,
        typed_fields_and_vals,
        generic_params,
        where_preds,
        impl_blocks,
        macro_defs,
        ws_macro_callsites,
    };

    let sec = TierSectionBytes { shards: 1, disk_bytes, string_pool_bytes: h.string_pool_len, unique_sig_string_bytes, item_tables_bytes, body_refs_bytes, body_types_notes_bytes };
    (sec, gap, lang)
}

fn count_lang_items<B: AsRef<[u8]>>(reader: &IndexReader<B>, sets: &GlobalKindSets) -> (LangItemCounts, u64) {
    let mut lang = LangItemCounts::default();
    let mut trait_impl_edges = 0u64;
    for raw_d in reader.deref_entries() {
        let de = raw_d.from_le();
        let tgt = reader.get_str(de.target_type_offset);
        let tgt_h = fnv1a_64(tgt.as_bytes());
        let src_is_trait = sets.traits.contains(&de.source_type_hash);
        let tgt_is_trait = sets.traits.contains(&tgt_h);
        if !src_is_trait && tgt_is_trait {
            trait_impl_edges += 1;
            match tgt {
                "core::ops::deref::DerefMut" | "std::ops::DerefMut" => lang.deref_mut += 1,
                "core::ops::index::Index" | "core::ops::index::IndexMut" | "std::ops::Index" | "std::ops::IndexMut" => lang.index_and_mut += 1,
                "core::iter::traits::collect::IntoIterator" | "core::iter::traits::iterator::Iterator" => lang.into_iterator_and_iter += 1,
                "core::ops::try_trait::Try" => lang.try_trait += 1,
                "core::future::future::Future" | "std::future::Future" => lang.future_trait += 1,
                _ => {}
            }
        } else if !src_is_trait && sets.is_type_path(tgt, tgt_h) && !tgt.is_empty() {
            lang.deref_targets += 1;
        }
    }
    (lang, trait_impl_edges)
}

fn count_ws_macros<B: AsRef<[u8]>>(reader: &IndexReader<B>, ws_src_root: Option<&Path>, seen_ws_files: &mut HashSet<PathBuf>) -> (u64, u64) {
    let Some(root) = ws_src_root else {
        return (0, 0);
    };
    let ws_macro_refs = reader.references().iter().filter(|r| r.is_macro_expansion()).count() as u64;
    let mut callsites = 0u64;
    for fid in 0..reader.files_count() {
        let rel = reader.get_file(fid);
        if rel.is_empty() {
            continue;
        }
        let full = join_resolving_overlap(root, Path::new(rel));
        if seen_ws_files.insert(full.clone()) {
            if let Ok(src) = fs::read_to_string(&full) {
                callsites += count_macro_invocations_fast(src.as_bytes());
            }
        }
    }
    (callsites, ws_macro_refs)
}

fn count_macro_invocations_fast(bytes: &[u8]) -> u64 {
    let mut count = 0u64;
    let mut i = 1usize;
    while i + 1 < bytes.len() {
        if bytes[i] == b'!' {
            let prev = bytes[i - 1];
            if prev.is_ascii_alphanumeric() || prev == b'_' {
                let mut j = i + 1;
                while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t') {
                    j += 1;
                }
                if j < bytes.len() && matches!(bytes[j], b'(' | b'[' | b'{') {
                    count += 1;
                }
            }
        }
        i += 1;
    }
    count
}
