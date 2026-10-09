/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use std::collections::{HashSet, VecDeque};
use std::path::Path;
use std::time::Instant;

use ridx_format::{IndexWriter, RawDefinition};
use rustc_hir::def::DefKind;
use rustc_middle::ty::TyCtxt;

use crate::map_def_kind;
use crate::sanitize::sanitize_type_name;

mod cas;
mod items;
mod primitives;
mod traits;
pub mod util;

pub use cas::{compute_sysroot_cas_key, compute_sysroot_cas_key_str, ensure_sysroot_indexed};
pub use primitives::extract_primitive_inherent_impls;
pub use traits::{collect_sysroot_traits, extract_trait_and_deref_links};
pub use util::{
    discover_rust_src_path, format_assoc_fn_sig, is_publicly_visible, is_stable_api,
    remap_sysroot_path,
};

/// Statistics collected during sysroot crate metadata extraction.
#[derive(Debug, Clone, Default)]
pub struct SysrootExtractStats {
    pub crate_name: String,
    pub total_definitions: usize,
    pub total_derefs: usize,
    pub index_bytes: usize,
    pub elapsed_ms: f64,
}

/// Extracts definitions, owners, signatures, and deref relationships for a sysroot crate
/// (e.g. "core", "alloc", "std") directly from pre-loaded compiler `.rmeta` metadata.
pub fn extract_sysroot_crate<'tcx>(
    tcx: TyCtxt<'tcx>,
    target_crate: &str,
    sysroot_src: Option<&Path>,
) -> Option<(IndexWriter, SysrootExtractStats)> {
    let start = Instant::now();

    let cnum = tcx
        .crates(())
        .iter()
        .copied()
        .find(|&c| tcx.crate_name(c).as_str() == target_crate)?;

    let root_did = cnum.as_def_id();
    let toolchain_version = rustc_interface::util::rustc_version_str()
        .map(|s| s.to_string())
        .unwrap_or_else(|| tcx.sess.cfg_version.to_string());
    let mut writer = IndexWriter::new(target_crate, &toolchain_version);
    let mut visited = HashSet::new();
    let mut queue = VecDeque::new();
    let mut queue_visited = HashSet::new();

    queue.push_back(root_did);
    queue_visited.insert(root_did);
    visited.insert(root_did);

    let mut total_definitions = 0usize;
    let mut total_derefs = 0usize;

    while let Some(mod_did) = queue.pop_front() {
        let mod_path = rustc_middle::ty::print::with_no_visible_paths!(
            rustc_middle::ty::print::with_no_trimmed_paths!(tcx.def_path_str(mod_did))
        );
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
            if (target_crate == "core" || target_crate == "std")
                && (name_str == "arch" || name_str == "core_arch" || name_str == "intrinsics")
            {
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

                if !child.vis.is_public()
                    || !is_publicly_visible(tcx, did)
                    || !is_stable_api(tcx, did)
                {
                    continue;
                }

                items::record_reexport(
                    tcx,
                    did,
                    name_str,
                    &mod_path,
                    sysroot_src,
                    &mut writer,
                    &mut total_definitions,
                );
                continue;
            }

            // Intra-crate item
            let canonical_path = rustc_middle::ty::print::with_no_visible_paths!(
                rustc_middle::ty::print::with_no_trimmed_paths!(tcx.def_path_str(did))
            );
            let alias_path = format!("{mod_path}::{name_str}");

            let kind = tcx.def_kind(did);
            if kind == DefKind::Mod && is_publicly_visible(tcx, did) && queue_visited.insert(did) {
                queue.push_back(did);
            }

            if !is_publicly_visible(tcx, did) || !is_stable_api(tcx, did) {
                continue;
            }

            if !visited.insert(did) {
                // If already visited, record as re-export alias if exposed under different path/name
                if alias_path != canonical_path && child.vis.is_public() {
                    items::record_reexport(
                        tcx,
                        did,
                        name_str,
                        &mod_path,
                        sysroot_src,
                        &mut writer,
                        &mut total_definitions,
                    );
                }
                continue;
            }

            // First time seeing this definition in this crate
            let (file, s_line, s_col, e_line, e_col) =
                items::extract_def_span(tcx, did, sysroot_src);

            let signature = match kind {
                DefKind::Fn => Some(sanitize_type_name(
                    &tcx.fn_sig(did).skip_binder().to_string(),
                )),
                DefKind::AssocFn => Some(util::format_assoc_fn_sig(tcx, did)),
                _ => None,
            };

            let raw_def = RawDefinition {
                def_id: total_definitions as u32,
                kind: map_def_kind(kind),
                visibility: 0,
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
                let parent_path = rustc_middle::ty::print::with_no_visible_paths!(
                    rustc_middle::ty::print::with_no_trimmed_paths!(
                        tcx.def_path_str(tcx.parent(did))
                    )
                );
                if parent_path != mod_path {
                    owners.push(parent_path);
                }
            }

            writer.add_definition_with_owners(raw_def, owners);
            total_definitions += 1;

            if alias_path != canonical_path && child.vis.is_public() {
                items::record_reexport(
                    tcx,
                    did,
                    name_str,
                    &mod_path,
                    sysroot_src,
                    &mut writer,
                    &mut total_definitions,
                );
            }

            if kind == DefKind::Trait {
                items::extract_trait_items(
                    tcx,
                    did,
                    &canonical_path,
                    sysroot_src,
                    &mut writer,
                    &mut visited,
                    &mut total_definitions,
                    &mut total_derefs,
                );
            }

            match kind {
                DefKind::Struct | DefKind::Enum | DefKind::Union | DefKind::TyAlias => {
                    items::extract_inherent_impl_items(
                        tcx,
                        did,
                        &canonical_path,
                        sysroot_src,
                        &mut writer,
                        &mut visited,
                        &mut total_definitions,
                    );
                }
                _ => {}
            }
        }
    }

    // Extract primitive inherent impls (impl str, impl [T], impl u32, etc.)
    extract_primitive_inherent_impls(
        tcx,
        cnum,
        &mut writer,
        &mut visited,
        &mut total_definitions,
        sysroot_src,
    );

    // Extract Deref relationships and trait implementation links for types
    let sysroot_traits = traits::collect_sysroot_traits(tcx);
    traits::extract_trait_and_deref_links(
        tcx,
        cnum,
        &sysroot_traits,
        &mut writer,
        &mut total_derefs,
    );

    let elapsed = start.elapsed();
    let stats = SysrootExtractStats {
        crate_name: target_crate.to_string(),
        total_definitions,
        total_derefs,
        index_bytes: 0,
        elapsed_ms: elapsed.as_secs_f64() * 1000.0,
    };

    Some((writer, stats))
}
