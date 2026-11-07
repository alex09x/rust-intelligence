/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Completeness audit comparing `rustc` (`TyCtxt`) public sysroot items against `.ridx` sysroot shards.

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::io::Cursor;

use ridx_format::IndexReader;
use rustc_hir::def::DefKind;
use rustc_middle::ty::TyCtxt;
use rustc_span::def_id::{CrateNum, DefId};

use super::audit_impls::{audit_primitive_impls, audit_trait_impls, canon_path, count_subtree_defs, family_key, subfamily_key};
pub use super::audit_types::{print_sysroot_completeness, CrateCompleteness, FamilyCompleteness, SysrootCompletenessReport};
use super::extract_sysroot_crate;
use super::util::{is_publicly_visible, is_stable_api};

#[repr(C, align(8))]
struct AlignedBuf {
    _align: [u64; 0],
    bytes: Vec<u8>,
}

impl AsRef<[u8]> for AlignedBuf {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}

/// Audits `core`, `alloc`, `std` in `tcx` against `extract_sysroot_crate`.
pub fn audit_sysroot_completeness<'tcx>(tcx: TyCtxt<'tcx>) -> SysrootCompletenessReport {
    let mut ridx_canon: HashSet<String> = HashSet::new();
    let mut ridx_stats: BTreeMap<String, (usize, usize, usize, usize)> = BTreeMap::new();

    for cname in ["core", "alloc", "std"] {
        let Some((mut writer, st)) = extract_sysroot_crate(tcx, cname, None) else { continue };
        let mut cur = Cursor::new(Vec::new());
        writer.write_to(&mut cur).expect("serialize sysroot");
        let raw = cur.into_inner();
        let mut words = vec![0u64; raw.len().div_ceil(8)];
        unsafe {
            std::slice::from_raw_parts_mut(words.as_mut_ptr() as *mut u8, raw.len()).copy_from_slice(&raw);
            let vec_u8 = Vec::from_raw_parts(words.as_mut_ptr() as *mut u8, raw.len(), words.capacity() * 8);
            std::mem::forget(words);
            let reader = IndexReader::from_buffer(AlignedBuf { _align: [], bytes: vec_u8 }).expect("open sysroot");
            let (mut canon_n, mut reexp_n) = (0usize, 0usize);
            for d in reader.definitions() {
                if d.is_reexport() {
                    reexp_n += 1;
                } else {
                    canon_n += 1;
                    ridx_canon.insert(reader.get_str(d.path_offset()).to_string());
                }
            }
            ridx_stats.insert(cname.to_string(), (reader.definitions().len(), canon_n, reexp_n, st.total_derefs));
        }
    }

    let mut all_pub_traits: Vec<(DefId, String, bool)> = Vec::new();
    let mut seen_tr = HashSet::new();
    for &cnum in tcx.crates(()) {
        if !matches!(tcx.crate_name(cnum).as_str(), "core" | "alloc" | "std") {
            continue;
        }
        let mut q = VecDeque::from([cnum.as_def_id()]);
        let mut sm = HashSet::from([cnum.as_def_id()]);
        while let Some(m) = q.pop_front() {
            for ch in tcx.module_children(m) {
                if !ch.vis.is_public() {
                    continue;
                }
                let Some(did) = ch.res.opt_def_id() else { continue };
                if did.krate != cnum {
                    continue;
                }
                let k = tcx.def_kind(did);
                if k == DefKind::Mod && is_publicly_visible(tcx, did) && sm.insert(did) {
                    q.push_back(did);
                } else if k == DefKind::Trait && is_publicly_visible(tcx, did) && seen_tr.insert(did) {
                    let cp = canon_path(tcx, did);
                    let dropped = !is_stable_api(tcx, did)
                        || tcx.lang_items().sized_trait() == Some(did)
                        || tcx.trait_is_auto(did)
                        || tcx.associated_item_def_ids(did).is_empty()
                        || cp.starts_with("core::marker::");
                    all_pub_traits.push((did, cp, dropped));
                }
            }
        }
    }

    let mut families: BTreeMap<String, FamilyCompleteness> = BTreeMap::new();
    let mut subfamilies: BTreeMap<String, FamilyCompleteness> = BTreeMap::new();
    let mut crates_out = Vec::new();

    for cname in ["core", "alloc", "std"] {
        let Some(&cnum) = tcx.crates(()).iter().find(|&&c| tcx.crate_name(c).as_str() == cname) else { continue };
        let (tot_r, can_r, reexp_r, deref_r) = ridx_stats.get(cname).copied().unwrap_or((0, 0, 0, 0));
        let mut cc = CrateCompleteness {
            crate_name: cname.to_string(),
            ridx_total_defs: tot_r,
            ridx_canonical_defs: can_r,
            ridx_reexport_defs: reexp_r,
            ridx_deref_edges: deref_r,
            ..Default::default()
        };
        walk_crate(tcx, cname, cnum, &ridx_canon, &all_pub_traits, &mut cc, &mut families, &mut subfamilies);
        crates_out.push(cc);
    }

    SysrootCompletenessReport { crates: crates_out, families: families.into_values().collect(), subfamilies: subfamilies.into_values().collect() }
}

#[allow(clippy::too_many_arguments)]
fn walk_crate<'tcx>(
    tcx: TyCtxt<'tcx>,
    cname: &str,
    cnum: CrateNum,
    ridx_canon: &HashSet<String>,
    all_pub_traits: &[(DefId, String, bool)],
    cc: &mut CrateCompleteness,
    families: &mut BTreeMap<String, FamilyCompleteness>,
    subfamilies: &mut BTreeMap<String, FamilyCompleteness>,
) {
    let root_did = cnum.as_def_id();
    let mut q = VecDeque::from([root_did]);
    let mut q_seen = HashSet::from([root_did]);
    let mut seen_did = HashSet::from([root_did]);
    cc.tcx_pub_stable_defs += 1;
    cc.tcx_pub_stable_in_ridx += usize::from(ridx_canon.contains(cname));
    let mut foreign_set = BTreeMap::<String, usize>::new();

    while let Some(m_did) = q.pop_front() {
        let m_path = canon_path(tcx, m_did);
        for ch in tcx.module_children(m_did) {
            if !ch.vis.is_public() {
                continue;
            }
            let Some(did) = ch.res.opt_def_id() else { continue };
            let ident = ch.ident.as_str();
            if ident.starts_with("__") {
                continue;
            }
            if matches!(cname, "core" | "std") && matches!(ident, "arch" | "core_arch" | "intrinsics") {
                if did.krate == cnum && seen_did.insert(did) {
                    cc.skipped_arch_intrinsics_defs += count_subtree_defs(tcx, did);
                }
                continue;
            }
            if did.krate != cnum {
                let dep_c = tcx.crate_name(did.krate).to_string();
                let allowed = (cname == "std" && (dep_c == "core" || dep_c == "alloc")) || (cname == "alloc" && dep_c == "core");
                if !allowed && is_publicly_visible(tcx, did) && is_stable_api(tcx, did) {
                    cc.skipped_foreign_krate_reexports += 1;
                    *foreign_set.entry(dep_c.clone()).or_insert(0) += 1;
                    if cc.foreign_reexport_samples.len() < 12 {
                        cc.foreign_reexport_samples.push(format!("{m_path}::{ident}->{dep_c}::{}", canon_path(tcx, did)));
                    }
                    let fk = family_key(cname, &m_path);
                    families.entry(fk.clone()).or_insert_with(|| FamilyCompleteness { family: fk, ..Default::default() }).foreign_reexp += 1;
                    if let Some(sfk) = subfamily_key(cname, &m_path, &m_path) {
                        subfamilies.entry(sfk.clone()).or_insert_with(|| FamilyCompleteness { family: sfk, ..Default::default() }).foreign_reexp += 1;
                    }
                }
                continue;
            }

            let kind = tcx.def_kind(did);
            if kind == DefKind::Mod && is_publicly_visible(tcx, did) && q_seen.insert(did) {
                q.push_back(did);
            }
            if !seen_did.insert(did) {
                continue;
            }
            let cpath = canon_path(tcx, did);
            let fk = family_key(cname, &cpath);
            let sfk = subfamily_key(cname, &cpath, &m_path);
            let fam = families.entry(fk.clone()).or_insert_with(|| FamilyCompleteness { family: fk, ..Default::default() });
            let sfam = sfk.map(|k| subfamilies.entry(k.clone()).or_insert_with(|| FamilyCompleteness { family: k, ..Default::default() }));

            if !is_publicly_visible(tcx, did) {
                cc.skipped_doc_hidden_or_priv_defs += 1;
                fam.doc_hidden += 1;
                if let Some(sf) = sfam {
                    sf.doc_hidden += 1;
                }
                continue;
            }
            if !is_stable_api(tcx, did) {
                cc.skipped_unstable_pub_defs += 1;
                fam.pub_unstable += 1;
                if let Some(sf) = sfam {
                    sf.pub_unstable += 1;
                }
                if kind == DefKind::Trait && cc.unstable_traits_sample.len() < 14 {
                    cc.unstable_traits_sample.push(cpath);
                }
                continue;
            }
            audit_item(tcx, did, kind, &cpath, ridx_canon, &mut seen_did, cc, fam, sfam);
        }
    }
    audit_primitive_impls(tcx, cname, cnum, ridx_canon, &mut seen_did, cc, families);
    cc.foreign_krates = foreign_set.into_iter().map(|(k, n)| format!("{k}:{n}")).collect();
    audit_trait_impls(tcx, cnum, all_pub_traits, cc, families, subfamilies);
}

#[allow(clippy::too_many_arguments)]
fn audit_item(
    tcx: TyCtxt<'_>,
    did: DefId,
    kind: DefKind,
    cpath: &str,
    ridx_canon: &HashSet<String>,
    seen_did: &mut HashSet<DefId>,
    cc: &mut CrateCompleteness,
    fam: &mut FamilyCompleteness,
    mut sfam: Option<&mut FamilyCompleteness>,
) {
    let rec = |p: &str, k: DefKind, cc: &mut CrateCompleteness, fam: &mut FamilyCompleteness, sf: &mut Option<&mut FamilyCompleteness>| {
        cc.tcx_pub_stable_defs += 1;
        fam.pub_stable_total += 1;
        if let Some(s) = sf.as_deref_mut() {
            s.pub_stable_total += 1;
        }
        if ridx_canon.contains(p) {
            cc.tcx_pub_stable_in_ridx += 1;
            fam.pub_stable_in_ridx += 1;
            if let Some(s) = sf.as_deref_mut() {
                s.pub_stable_in_ridx += 1;
            }
        } else if cc.tcx_pub_stable_missing.len() < 12 {
            cc.tcx_pub_stable_missing.push(format!("{k:?}:{p}"));
        }
    };
    rec(cpath, kind, cc, fam, &mut sfam);

    if matches!(kind, DefKind::Struct | DefKind::Enum | DefKind::Union | DefKind::Trait | DefKind::Fn | DefKind::TyAlias) {
        cc.pub_generic_params_missing += tcx.generics_of(did).own_params.iter().filter(|p| p.name != rustc_span::symbol::kw::SelfUpper).count();
    }
    if matches!(kind, DefKind::Struct | DefKind::Enum | DefKind::Union) {
        for v in tcx.adt_def(did).variants() {
            if kind == DefKind::Enum && seen_did.insert(v.def_id) {
                rec(&canon_path(tcx, v.def_id), DefKind::Variant, cc, fam, &mut sfam);
            }
            for f in &v.fields {
                if f.vis.is_public() && is_stable_api(tcx, f.did) && !tcx.is_doc_hidden(f.did) {
                    if f.name.as_str().bytes().all(|b| b.is_ascii_digit()) {
                        cc.pub_tuple_fields_missing += 1;
                    } else {
                        cc.pub_named_fields_missing += 1;
                    }
                    fam.pub_fields_missing += 1;
                    if let Some(s) = sfam.as_deref_mut() {
                        s.pub_fields_missing += 1;
                    }
                }
            }
        }
        for &imp in tcx.inherent_impls(did) {
            for &a_did in tcx.associated_item_def_ids(imp) {
                if is_publicly_visible(tcx, a_did) && is_stable_api(tcx, a_did) && tcx.opt_item_name(a_did).is_some_and(|s| !s.as_str().starts_with("__")) && seen_did.insert(a_did)
                {
                    rec(&canon_path(tcx, a_did), tcx.def_kind(a_did), cc, fam, &mut sfam);
                }
            }
        }
    }
    if kind == DefKind::Trait {
        let assoc = tcx.associated_item_def_ids(did);
        if cpath.starts_with("core::marker::") || tcx.trait_is_auto(did) || assoc.is_empty() {
            cc.marker_or_empty_traits.push(cpath.to_string());
            fam.marker_or_empty_traits += 1;
            if let Some(s) = sfam.as_deref_mut() {
                s.marker_or_empty_traits += 1;
            }
        }
        for &a_did in assoc {
            if !is_publicly_visible(tcx, a_did) || tcx.opt_item_name(a_did).is_none_or(|s| s.as_str().starts_with("__")) {
                continue;
            }
            let ap = canon_path(tcx, a_did);
            if !is_stable_api(tcx, a_did) {
                cc.skipped_unstable_pub_defs += 1;
                fam.pub_unstable += 1;
                if let Some(s) = sfam.as_deref_mut() {
                    s.pub_unstable += 1;
                }
                if ap.starts_with("core::ops::") && cc.unstable_ops_assoc_sample.len() < 14 {
                    cc.unstable_ops_assoc_sample.push(ap);
                }
            } else if seen_did.insert(a_did) {
                rec(&ap, tcx.def_kind(a_did), cc, fam, &mut sfam);
            }
        }
    }
}
