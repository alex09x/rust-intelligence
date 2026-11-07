/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;
use std::time::Instant;

use ridx_format::{IndexWriter, RawDefinition};
use rustc_hir::def::DefKind;
use rustc_middle::ty::TyCtxt;

use crate::map_def_kind;
use crate::sanitize::sanitize_type_name;

pub mod audit;
mod audit_impls;
pub mod audit_types;
mod cas;
mod items;
mod lang;
mod primitives;
mod traits;
pub mod util;

pub use cas::{compute_sysroot_cas_key, compute_sysroot_cas_key_str, ensure_sysroot_indexed};
pub use primitives::extract_primitive_inherent_impls;
pub use traits::{collect_sysroot_traits, extract_trait_and_deref_links};
pub use util::{discover_rust_src_path, format_assoc_fn_sig, is_publicly_visible, is_stable_api, remap_sysroot_path};

/// Statistics collected during sysroot crate metadata extraction.
#[derive(Debug, Clone, Default)]
pub struct SysrootExtractStats {
    pub crate_name: String,
    pub total_definitions: usize,
    pub total_derefs: usize,
    pub index_bytes: usize,
    pub elapsed_ms: f64,
}

/// Per-crate accumulator for sysroot definitions and structured `TyCtxt` tables.
#[derive(Default)]
pub struct SysrootCollector {
    pub total_definitions: usize,
    pub local_def_indices: HashMap<rustc_span::def_id::DefId, u32>,
    pub callables: Vec<(rustc_span::def_id::DefId, u32)>,
    pub impl_dids: Vec<rustc_span::def_id::DefId>,
    pub seen_impls: HashSet<rustc_span::def_id::DefId>,
    pub adts: Vec<(rustc_span::def_id::DefId, u32)>,
    pub def_tys: Vec<(rustc_span::def_id::DefId, u32)>,
    pub generic_items: Vec<(rustc_span::def_id::DefId, u32)>,
    pub mod_items: Vec<(rustc_span::def_id::DefId, u32)>,
}

impl SysrootCollector {
    #[inline]
    pub fn register_primary(&mut self, did: rustc_span::def_id::DefId, kind: DefKind, def_idx: u32) {
        self.total_definitions += 1;
        self.local_def_indices.insert(did, def_idx);
        match kind {
            DefKind::Mod => self.mod_items.push((did, def_idx)),
            DefKind::Fn | DefKind::AssocFn => self.callables.push((did, def_idx)),
            DefKind::Struct | DefKind::Union | DefKind::Enum => self.adts.push((did, def_idx)),
            DefKind::TyAlias | DefKind::Const { .. } | DefKind::Static { .. } | DefKind::AssocConst { .. } | DefKind::Variant => {
                self.def_tys.push((did, def_idx));
            }
            _ => {}
        }
        if matches!(
            kind,
            DefKind::Fn | DefKind::AssocFn | DefKind::Struct | DefKind::Union | DefKind::Enum | DefKind::Trait | DefKind::TraitAlias | DefKind::TyAlias | DefKind::AssocTy
        ) {
            self.generic_items.push((did, def_idx));
        }
    }

    #[inline]
    pub fn register_reexport(&mut self, did: rustc_span::def_id::DefId, kind: DefKind, def_idx: u32) {
        self.total_definitions += 1;
        match kind {
            DefKind::Fn | DefKind::AssocFn => self.callables.push((did, def_idx)),
            DefKind::TyAlias
            | DefKind::Const { .. }
            | DefKind::Static { .. }
            | DefKind::AssocConst { .. }
            | DefKind::Struct
            | DefKind::Union
            | DefKind::Enum
            | DefKind::Variant => {
                self.def_tys.push((did, def_idx));
            }
            _ => {}
        }
        if matches!(
            kind,
            DefKind::Fn | DefKind::AssocFn | DefKind::Struct | DefKind::Union | DefKind::Enum | DefKind::Trait | DefKind::TraitAlias | DefKind::TyAlias | DefKind::AssocTy
        ) {
            self.generic_items.push((did, def_idx));
        }
    }

    #[inline]
    pub fn register_impl(&mut self, impl_did: rustc_span::def_id::DefId) {
        if self.seen_impls.insert(impl_did) {
            self.impl_dids.push(impl_did);
        }
    }
}

/// Extracts definitions, owners, signatures, and deref relationships for a sysroot crate
/// (e.g. "core", "alloc", "std") directly from pre-loaded compiler `.rmeta` metadata.
pub fn extract_sysroot_crate<'tcx>(tcx: TyCtxt<'tcx>, target_crate: &str, sysroot_src: Option<&Path>) -> Option<(IndexWriter, SysrootExtractStats)> {
    let start = Instant::now();

    let cnum = tcx.crates(()).iter().copied().find(|&c| tcx.crate_name(c).as_str() == target_crate)?;

    let root_did = cnum.as_def_id();
    let toolchain_version = rustc_interface::util::rustc_version_str().map(|s| s.to_string()).unwrap_or_else(|| tcx.sess.cfg_version.to_string());
    let mut writer = IndexWriter::new(target_crate, &toolchain_version);
    let mut visited = HashSet::new();
    let mut assoc_visited = HashMap::new();
    let mut queue = VecDeque::new();
    let mut queue_visited = HashSet::new();

    queue.push_back(root_did);
    queue_visited.insert(root_did);
    visited.insert(root_did);

    let mut collector = SysrootCollector::default();
    let mut total_derefs = 0usize;

    // Record the crate root definition (e.g. `core`, `alloc`, `std`)
    let (root_file, root_sl, root_sc, root_el, root_ec) = items::extract_def_span(tcx, root_did, sysroot_src);
    let root_idx = writer.add_definition_with_owners(
        RawDefinition {
            def_id: collector.total_definitions as u32,
            kind: map_def_kind(DefKind::Mod),
            visibility: 0,
            is_macro_expansion: false,
            name: target_crate.to_string(),
            path: target_crate.to_string(),
            signature: None,
            file: root_file,
            start_line: root_sl,
            start_col: root_sc,
            end_line: root_el,
            end_col: root_ec,
        },
        vec![target_crate.to_string()],
    );
    collector.register_primary(root_did, DefKind::Mod, root_idx);

    while let Some(mod_did) = queue.pop_front() {
        let mod_path = rustc_middle::ty::print::with_no_visible_paths!(rustc_middle::ty::print::with_no_trimmed_paths!(tcx.def_path_str(mod_did)));
        let mut mod_seen = HashSet::new();

        for child in tcx.module_children(mod_did) {
            let Some(did) = child.res.opt_def_id() else {
                continue;
            };

            let name_str = child.ident.as_str();
            if name_str.starts_with("__rust_") || name_str.starts_with("__") {
                continue;
            }

            // Exclude arch SIMD intrinsics and compiler intrinsics from completion index
            if (target_crate == "core" || target_crate == "std") && (name_str == "arch" || name_str == "core_arch" || name_str == "intrinsics") {
                continue;
            }

            if !mod_seen.insert((did, name_str.to_string())) {
                continue;
            }

            // Cross-crate re-export check
            if did.krate != cnum {
                let dep_crate = tcx.crate_name(did.krate);
                let dep_crate_str = dep_crate.as_str();
                let allowed = match target_crate {
                    "std" => dep_crate_str == "core" || dep_crate_str == "alloc",
                    "alloc" => dep_crate_str == "core",
                    _ => false,
                };
                if !allowed {
                    continue;
                }

                if !child.vis.is_public() || !is_publicly_visible(tcx, did) || !is_stable_api(tcx, did) {
                    continue;
                }

                items::record_reexport(tcx, did, name_str, &mod_path, sysroot_src, &mut writer, &mut collector);
                continue;
            }

            // Intra-crate item
            let canonical_path = rustc_middle::ty::print::with_no_visible_paths!(rustc_middle::ty::print::with_no_trimmed_paths!(tcx.def_path_str(did)));
            let alias_path = format!("{mod_path}::{name_str}");

            let kind = tcx.def_kind(did);
            let is_prelude_mod =
                kind == DefKind::Mod && mod_path.ends_with("::prelude") && matches!(name_str, "v1" | "rust_2015" | "rust_2018" | "rust_2018_preview" | "rust_2021" | "rust_2024");
            let vis_ok = is_publicly_visible(tcx, did) || (is_prelude_mod && tcx.visibility(did).is_public());
            if kind == DefKind::Mod && vis_ok && queue_visited.insert(did) {
                queue.push_back(did);
            }

            if !vis_ok || !is_stable_api(tcx, did) {
                continue;
            }

            if !visited.insert(did) {
                // If already visited, record as re-export alias if exposed under different path/name
                if alias_path != canonical_path && child.vis.is_public() {
                    items::record_reexport(tcx, did, name_str, &mod_path, sysroot_src, &mut writer, &mut collector);
                }
                continue;
            }

            // First time seeing this definition in this crate
            let (file, s_line, s_col, e_line, e_col) = items::extract_def_span(tcx, did, sysroot_src);

            let signature = match kind {
                DefKind::Fn => Some(sanitize_type_name(&tcx.fn_sig(did).skip_binder().to_string())),
                DefKind::AssocFn => Some(util::format_assoc_fn_sig(tcx, did)),
                _ => None,
            };

            let raw_def = RawDefinition {
                def_id: collector.total_definitions as u32,
                kind: map_def_kind(kind),
                visibility: crate::ty_lower::fields::def_visibility(tcx, did, kind) as u8,
                is_macro_expansion: false,
                name: name_str.to_string(),
                path: canonical_path.clone(),
                signature,
                file,
                start_line: s_line,
                start_col: s_col,
                end_line: e_line,
                end_col: e_col,
            };

            let mut owners = vec![mod_path.clone()];
            if !did.is_crate_root() && tcx.def_kind(tcx.parent(did)) == DefKind::Mod {
                let parent_path = rustc_middle::ty::print::with_no_visible_paths!(rustc_middle::ty::print::with_no_trimmed_paths!(tcx.def_path_str(tcx.parent(did))));
                if parent_path != mod_path {
                    owners.push(parent_path);
                }
            }

            let def_idx = writer.add_definition_with_owners(raw_def, owners);
            collector.register_primary(did, kind, def_idx);

            if alias_path != canonical_path && child.vis.is_public() {
                items::record_reexport(tcx, did, name_str, &mod_path, sysroot_src, &mut writer, &mut collector);
            }

            if kind == DefKind::Trait {
                items::extract_trait_items(tcx, did, &canonical_path, sysroot_src, &mut writer, &mut visited, &mut collector, &mut total_derefs);
            }

            if kind == DefKind::Enum {
                items::extract_enum_variants(tcx, did, &canonical_path, sysroot_src, &mut writer, &mut visited, &mut collector);
            }

            match kind {
                DefKind::Struct | DefKind::Enum | DefKind::Union | DefKind::TyAlias | DefKind::Trait => {
                    items::extract_inherent_impl_items(tcx, did, &canonical_path, sysroot_src, &mut writer, &mut assoc_visited, &mut collector);
                }
                _ => {}
            }
        }
    }

    // Extract primitive and trait-object inherent impls (impl str, impl [T], impl dyn Any, etc.)
    extract_primitive_inherent_impls(tcx, cnum, &mut writer, &mut visited, &mut collector, sysroot_src);

    // Collect 100% of trait impls in `cnum` directly from `.rmeta` (zero filtering, no `peel_refs`)
    for &impl_did in tcx.trait_impls_in_crate(cnum) {
        collector.register_impl(impl_did);
    }

    // Extract Deref relationships and trait implementation links for types
    let sysroot_traits = traits::collect_sysroot_traits(tcx);
    traits::extract_trait_and_deref_links(tcx, cnum, &sysroot_traits, &mut writer, &mut total_derefs);

    // Collect `#[lang]` items in `cnum` and materialize unstable/private lang items & assoc items
    let crate_lang_items = crate::ty_lower::lang_items::collect_crate_lang_items(tcx, cnum);
    lang::materialize_lang_items(tcx, &crate_lang_items, sysroot_src, &mut writer, &mut visited, &mut collector);

    if !collector.callables.is_empty()
        || !collector.impl_dids.is_empty()
        || !collector.adts.is_empty()
        || !collector.def_tys.is_empty()
        || !collector.generic_items.is_empty()
        || !crate_lang_items.is_empty()
    {
        let sysroot_units: HashMap<rustc_span::def_id::CrateNum, String> = tcx
            .crates(())
            .iter()
            .copied()
            .filter_map(|c| {
                let name = tcx.crate_name(c).to_string();
                matches!(name.as_str(), "core" | "alloc" | "std" | "proc_macro").then_some((c, name))
            })
            .collect();
        let mut lowerer = crate::ty_lower::TcxTypeLowerer::new(tcx, cnum, &sysroot_units, &collector.local_def_indices);
        lowerer.record_fn_sigs(&collector.callables, &mut writer);
        lowerer.record_impls(&collector.impl_dids, &mut writer);
        lowerer.record_adts(&collector.adts, &mut writer);
        lowerer.record_def_tys(&collector.def_tys, &mut writer);
        lowerer.record_item_generics_and_preds(&collector.generic_items, &mut writer);
        lowerer.record_lang_items(&crate_lang_items, &mut writer);
        lowerer.record_sysroot_uses(cnum, target_crate, &collector.mod_items, &mut writer);
    }

    let elapsed = start.elapsed();
    let stats = SysrootExtractStats {
        crate_name: target_crate.to_string(),
        total_definitions: collector.total_definitions,
        total_derefs,
        index_bytes: 0,
        elapsed_ms: elapsed.as_secs_f64() * 1000.0,
    };

    Some((writer, stats))
}
