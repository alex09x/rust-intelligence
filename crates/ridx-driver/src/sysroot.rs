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
use std::path::{Path, PathBuf};
use std::time::Instant;

use ridx_format::{IndexWriter, RawDefinition};
use rustc_hir::def::DefKind;
use rustc_middle::ty::TyCtxt;

use crate::map_def_kind;
use crate::sanitize::sanitize_type_name;

/// Remaps a compiler-internal span path (e.g. `/rustc/<commit>/library/...`)
/// to a local `rust-src` source path if available, or returns the relative library path.
pub fn remap_sysroot_path(raw_path: &str, sysroot_src: Option<&Path>) -> String {
    if let Some(idx) = raw_path.find("/library/") {
        let rel_part = &raw_path[idx + 1..];
        if let Some(src_root) = sysroot_src {
            let candidate = src_root.join(rel_part);
            if candidate.exists() {
                return candidate.to_string_lossy().into_owned();
            }
        }
        return rel_part.to_string();
    }
    raw_path.to_string()
}

/// Discovers the path to `library` inside `rust-src` for the current sysroot if installed.
pub fn discover_rust_src_path() -> Option<PathBuf> {
    if let Ok(output) = std::process::Command::new("rustc")
        .args(["--print", "sysroot"])
        .output()
    {
        if output.status.success() {
            let sysroot = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let candidate = PathBuf::from(sysroot).join("lib/rustlib/src/rust/library");
            if candidate.is_dir() {
                return Some(candidate);
            }
        }
    }
    None
}

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
    let mut writer = IndexWriter::new(target_crate, "1.0.0");
    let mut visited = HashSet::new();
    let mut queue = VecDeque::new();

    queue.push_back(root_did);
    visited.insert(root_did);

    let mut total_definitions = 0usize;
    let mut total_derefs = 0usize;

    while let Some(mod_did) = queue.pop_front() {
        for child in tcx.module_children(mod_did) {
            let Some(did) = child.res.opt_def_id() else {
                continue;
            };

            // Stay strictly within the target sysroot crate
            if did.krate != cnum {
                continue;
            }

            if !visited.insert(did) {
                continue;
            }

            let kind = tcx.def_kind(did);
            if kind == DefKind::Mod {
                queue.push_back(did);
            }

            let item_span = tcx.def_ident_span(did).unwrap_or_else(|| tcx.def_span(did));
            let (file, s_line, s_col, e_line, e_col) = if !item_span.is_dummy() {
                let sm = tcx.sess.source_map();
                let lo = sm.lookup_char_pos(item_span.lo());
                let hi = sm.lookup_char_pos(item_span.hi());
                let raw_file = sm.filename_for_diagnostics(&lo.file.name).to_string();
                let remapped = remap_sysroot_path(&raw_file, sysroot_src);
                (
                    remapped,
                    lo.line as u32,
                    (lo.col.0 + 1) as u32,
                    hi.line as u32,
                    (hi.col.0 + 1) as u32,
                )
            } else {
                (String::new(), 0, 0, 0, 0)
            };

            let path = rustc_middle::ty::print::with_no_visible_paths!(
                rustc_middle::ty::print::with_no_trimmed_paths!(tcx.def_path_str(did))
            );
            let name = child.ident.as_str().to_string();

            let signature = match kind {
                DefKind::Fn | DefKind::AssocFn => {
                    Some(sanitize_type_name(&tcx.fn_sig(did).skip_binder().to_string()))
                }
                _ => None,
            };

            let raw_def = RawDefinition {
                def_id: total_definitions as u32,
                kind: map_def_kind(kind),
                visibility: 0,
                is_macro_expansion: false,
                name: name.clone(),
                path: path.clone(),
                signature,
                file: file.clone(),
                start_line: s_line,
                start_col: s_col,
                end_line: e_line,
                end_col: e_col,
            };

            // Module ownership: items declared inside module belong to module for `mod::` completion
            let owners = if !did.is_crate_root() && tcx.def_kind(tcx.parent(did)) == DefKind::Mod {
                let parent_did = tcx.parent(did);
                let mod_path = rustc_middle::ty::print::with_no_visible_paths!(
                    rustc_middle::ty::print::with_no_trimmed_paths!(tcx.def_path_str(parent_did))
                );
                vec![mod_path]
            } else {
                Vec::new()
            };

            writer.add_definition_with_owners(raw_def, owners);
            total_definitions += 1;

            // Extract supertraits and associated items for traits
            if kind == DefKind::Trait {
                for (clause, _) in tcx.explicit_super_predicates_of(did).skip_binder() {
                    if let rustc_middle::ty::ClauseKind::Trait(trait_pred) =
                        clause.kind().skip_binder()
                    {
                        let super_did = trait_pred.trait_ref.def_id;
                        if super_did != did
                            && tcx.lang_items().sized_trait() != Some(super_did)
                            && !tcx.trait_is_auto(super_did)
                        {
                            let super_path = rustc_middle::ty::print::with_no_visible_paths!(
                                rustc_middle::ty::print::with_no_trimmed_paths!(tcx
                                    .def_path_str(super_did))
                            );
                            if !super_path.starts_with("core::marker::") {
                                writer.add_deref(path.clone(), super_path);
                                total_derefs += 1;
                            }
                        }
                    }
                }

                for assoc_did in tcx.associated_item_def_ids(did) {
                    let assoc_did = *assoc_did;
                    if !visited.insert(assoc_did) {
                        continue;
                    }
                    let a_kind = tcx.def_kind(assoc_did);
                    let a_path = rustc_middle::ty::print::with_no_visible_paths!(
                        rustc_middle::ty::print::with_no_trimmed_paths!(tcx
                            .def_path_str(assoc_did))
                    );
                    let Some(name_sym) = tcx.opt_item_name(assoc_did) else {
                        continue;
                    };
                    let a_name = name_sym.to_string();
                    let a_sig = match a_kind {
                        DefKind::Fn | DefKind::AssocFn => Some(sanitize_type_name(
                            &tcx.fn_sig(assoc_did).skip_binder().to_string(),
                        )),
                        _ => None,
                    };
                    let a_span = tcx
                        .def_ident_span(assoc_did)
                        .unwrap_or_else(|| tcx.def_span(assoc_did));
                    let (a_file, a_sl, a_sc, a_el, a_ec) = if !a_span.is_dummy() {
                        let sm = tcx.sess.source_map();
                        let lo = sm.lookup_char_pos(a_span.lo());
                        let hi = sm.lookup_char_pos(a_span.hi());
                        let raw_file = sm.filename_for_diagnostics(&lo.file.name).to_string();
                        let remapped = remap_sysroot_path(&raw_file, sysroot_src);
                        (
                            remapped,
                            lo.line as u32,
                            (lo.col.0 + 1) as u32,
                            hi.line as u32,
                            (hi.col.0 + 1) as u32,
                        )
                    } else {
                        (file.clone(), 0, 0, 0, 0)
                    };
                    writer.add_definition_with_owners(
                        RawDefinition {
                            def_id: total_definitions as u32,
                            kind: map_def_kind(a_kind),
                            visibility: 0,
                            is_macro_expansion: false,
                            name: a_name,
                            path: a_path,
                            signature: a_sig,
                            file: a_file,
                            start_line: a_sl,
                            start_col: a_sc,
                            end_line: a_el,
                            end_col: a_ec,
                        },
                        vec![path.clone()],
                    );
                    total_definitions += 1;
                }
            }

            // Extract inherent methods for types
            match kind {
                DefKind::Struct | DefKind::Enum | DefKind::Union | DefKind::TyAlias => {
                    for impl_did in tcx.inherent_impls(did) {
                        let impl_did = *impl_did;
                        for assoc_did in tcx.associated_item_def_ids(impl_did) {
                            let assoc_did = *assoc_did;
                            if !visited.insert(assoc_did) {
                                continue;
                            }
                            let a_kind = tcx.def_kind(assoc_did);
                            let a_path = rustc_middle::ty::print::with_no_visible_paths!(
                                rustc_middle::ty::print::with_no_trimmed_paths!(tcx
                                    .def_path_str(assoc_did))
                            );
                            let Some(name_sym) = tcx.opt_item_name(assoc_did) else {
                                continue;
                            };
                            let a_name = name_sym.to_string();
                            let a_sig = match a_kind {
                                DefKind::Fn | DefKind::AssocFn => Some(sanitize_type_name(
                                    &tcx.fn_sig(assoc_did).skip_binder().to_string(),
                                )),
                                _ => None,
                            };
                            let a_span = tcx
                                .def_ident_span(assoc_did)
                                .unwrap_or_else(|| tcx.def_span(assoc_did));
                            let (a_file, a_sl, a_sc, a_el, a_ec) = if !a_span.is_dummy() {
                                let sm = tcx.sess.source_map();
                                let lo = sm.lookup_char_pos(a_span.lo());
                                let hi = sm.lookup_char_pos(a_span.hi());
                                let raw_file =
                                    sm.filename_for_diagnostics(&lo.file.name).to_string();
                                let remapped = remap_sysroot_path(&raw_file, sysroot_src);
                                (
                                    remapped,
                                    lo.line as u32,
                                    (lo.col.0 + 1) as u32,
                                    hi.line as u32,
                                    (hi.col.0 + 1) as u32,
                                )
                            } else {
                                (file.clone(), 0, 0, 0, 0)
                            };
                            writer.add_definition_with_owners(
                                RawDefinition {
                                    def_id: total_definitions as u32,
                                    kind: map_def_kind(a_kind),
                                    visibility: 0,
                                    is_macro_expansion: false,
                                    name: a_name,
                                    path: a_path,
                                    signature: a_sig,
                                    file: a_file,
                                    start_line: a_sl,
                                    start_col: a_sc,
                                    end_line: a_el,
                                    end_col: a_ec,
                                },
                                vec![path.clone()],
                            );
                            total_definitions += 1;
                        }
                    }
                }
                _ => {}
            }
        }
    }

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
