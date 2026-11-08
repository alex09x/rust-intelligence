/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Regression tests for [`CrateSnapshot::with_dirty_files`]:
//! - Zero false-positive `ModuleTree` rebuilds on files containing `cfg_*!` macro calls,
//!   `#[macro_use]`, or `macro_rules!` when only ordinary items/`use`s change;
//! - Preservation of inherited `#[macro_use]` textual macros across dirty/clean sibling modules;
//! - Exact scope and trait-table equivalence with a fresh [`CrateDefMap::build_cached`] across Tier 1
//!   (private local delta) and Tier 2 (`pub` item/`use` + glob re-export) incremental solves.

use ridx_syntax::name_res::{
    CrateDefMap, CrateId, CrateSnapshot, Edition, GlobalDefId, ItemKind, MemItemDb, MemSourceDb, ModuleId, ModuleTree, OverlaySourceDb, OwnedDef, VisKind,
};
use ridx_syntax::parse;

fn def(krate: u32, idx: u32) -> GlobalDefId {
    GlobalDefId::new(CrateId(krate), idx)
}

fn owned_def(id: GlobalDefId, name: &str, kind: ItemKind, vis: VisKind, ctor: bool) -> OwnedDef {
    OwnedDef { id, name: name.into(), kind, vis, has_value_ctor: ctor }
}

fn binding_key(map: &CrateDefMap, b: ridx_syntax::name_res::ResolvedBinding) -> (ItemKind, VisKind, bool, Option<GlobalDefId>, Option<&str>) {
    (b.kind, b.vis, b.def.is_overlay(), (!b.def.is_overlay()).then_some(b.def), b.local.is_some().then(|| map.item_name(b.local)))
}

fn assert_scopes_match_full_build(mod_tree: &ModuleTree, inc_map: &CrateDefMap, full_map: &CrateDefMap) {
    assert_eq!(inc_map.scopes.len(), full_map.scopes.len());
    for (midx, (inc_sc, full_sc)) in inc_map.scopes.iter().zip(&full_map.scopes).enumerate() {
        assert_eq!(inc_sc.entries.len(), full_sc.entries.len(), "module {midx} entry count mismatch");
        for (k, full_pns) in &full_sc.entries {
            let inc_pns = inc_sc.get(k);
            assert_eq!(inc_pns.types.map(|b| binding_key(inc_map, b)), full_pns.types.map(|b| binding_key(full_map, b)), "mod {midx} type {k:?}");
            assert_eq!(inc_pns.values.map(|b| binding_key(inc_map, b)), full_pns.values.map(|b| binding_key(full_map, b)), "mod {midx} val {k:?}");
            assert_eq!(inc_pns.macros.map(|b| binding_key(inc_map, b)), full_pns.macros.map(|b| binding_key(full_map, b)), "mod {midx} mac {k:?}");
        }
        let mid = ModuleId(midx as u32);
        let inc_tr: Vec<_> = inc_map.traits_in_scope(mod_tree, mid).into_iter().map(|b| binding_key(inc_map, b)).collect();
        let full_tr: Vec<_> = full_map.traits_in_scope(mod_tree, mid).into_iter().map(|b| binding_key(full_map, b)).collect();
        assert_eq!(inc_tr, full_tr, "mod {midx} traits_in_scope");
    }
}

#[test]
fn cfg_macro_and_macro_use_files_avoid_mod_tree_rebuild_and_preserve_textual_macros() {
    let mut sources = MemSourceDb::new();
    let f_lib = sources.add_file("src/lib.rs", "#[macro_use]\nmod macros;\npub mod net;\npub mod io;\n");
    let f_macros = sources.add_file(
        "src/macros.rs",
        "macro_rules! cfg_net { ($($it:item)*) => { $($it)* }; }\n\
         macro_rules! log_net { () => { () }; }\n\
         pub(crate) fn macro_mod_helper() -> u32 { 1 }\n",
    );
    let net_clean = "cfg_net! {\n    pub mod tcp;\n}\npub fn connect() { log_net!(); }\n";
    let f_net = sources.add_file("src/net/mod.rs", net_clean);
    let _f_tcp = sources.add_file("src/net/tcp.rs", "pub struct TcpStream;\n");
    let _f_udp = sources.add_file("src/net/udp.rs", "pub struct UdpSocket;\n");
    let f_io = sources.add_file("src/io.rs", "pub fn flush() { log_net!(); }\n");

    let mut db = MemItemDb::new();
    let krate = db.add_crate("tokio_like", Edition::Edition2021);
    db.set_crate_root_def(krate, def(krate.0, 0));

    let snap = CrateSnapshot::build_clean(&db, &sources, krate, f_lib);
    assert_eq!(snap.mod_tree.modules.len(), 5);

    // 1. Item + `use` edit inside `src/net/mod.rs` (which contains `cfg_net! { pub mod tcp; }`):
    //    must NOT rebuild `ModuleTree` AND must retain inherited `log_net!` textual macro.
    let net_edited = format!("{net_clean}\npub fn __probe_item() {{}}\npub use tcp::TcpStream as ExportedStream;\n");
    let mut ov = OverlaySourceDb::new(&sources, sources.len() as u32);
    ov.set_file_override(f_net, &net_edited);
    let parsed_net = parse(&net_edited);
    let (new_mt, inc_map) = snap.with_dirty_files(&db, &ov, ov.dirty_files(), |fid| (fid == f_net).then_some(&parsed_net));
    assert!(new_mt.is_none(), "item/use edit in cfg-macro file must reuse clean ModuleTree");

    let full_mt = ModuleTree::build(&db, &ov, krate, f_lib);
    let full_map = CrateDefMap::build_cached(&db, &ov, &full_mt, ov.dirty_files(), |_| None);
    assert_scopes_match_full_build(&snap.mod_tree, &inc_map, &full_map);

    let net_res = inc_map.resolve_file_paths(&db, &ov, &snap.mod_tree, f_net);
    assert!(net_res.refs.iter().any(|r| net_edited[r.start as usize..r.end as usize].starts_with("log_net")), "log_net! must stay resolved in dirty net/mod.rs");

    // 2. Non-macro helper edit inside `src/macros.rs` (`#[macro_use]` provider):
    //    must also reuse `ModuleTree` and preserve `log_net!` in clean `src/io.rs`.
    let macros_edited = "macro_rules! cfg_net { ($($it:item)*) => { $($it)* }; }\n\
                         macro_rules! log_net { () => { () }; }\n\
                         pub(crate) fn macro_mod_helper() -> u32 { 42 }\n\
                         fn extra_private() {}\n";
    let mut ov2 = OverlaySourceDb::new(&sources, sources.len() as u32);
    ov2.set_file_override(f_macros, macros_edited);
    let parsed_macros = parse(macros_edited);
    let (new_mt2, inc_map2) = snap.with_dirty_files(&db, &ov2, ov2.dirty_files(), |fid| (fid == f_macros).then_some(&parsed_macros));
    assert!(new_mt2.is_none(), "private helper edit in #[macro_use] file must reuse ModuleTree");
    let io_res = inc_map2.resolve_file_paths(&db, &ov2, &snap.mod_tree, f_io);
    assert!(
        io_res.refs.iter().any(|r| "pub fn flush() { log_net!(); }\n"[r.start as usize..r.end as usize].starts_with("log_net")),
        "log_net! must remain resolved in sibling io.rs"
    );

    // 3. Adding `pub mod udp;` inside `cfg_net! { ... }` MUST trigger `ModuleTree` rebuild.
    let net_mod_added = "cfg_net! {\n    pub mod tcp;\n    pub mod udp;\n}\npub fn connect() { log_net!(); }\n";
    let mut ov3 = OverlaySourceDb::new(&sources, sources.len() as u32);
    ov3.set_file_override(f_net, net_mod_added);
    let parsed_net3 = parse(net_mod_added);
    let (new_mt3, _) = snap.with_dirty_files(&db, &ov3, ov3.dirty_files(), |fid| (fid == f_net).then_some(&parsed_net3));
    let rebuilt = new_mt3.expect("adding `pub mod udp;` inside cfg_net! must rebuild ModuleTree");
    assert_eq!(rebuilt.modules.len(), 6);
}

#[test]
fn tier1_and_tier2_incremental_solves_match_full_rebuild_with_globs_and_deletions() {
    let mut sources = MemSourceDb::new();
    let f_lib = sources.add_file("src/lib.rs", "pub mod alpha;\npub mod beta;\npub use alpha::*;\n");
    let f_alpha = sources.add_file("src/alpha.rs", "pub struct AlphaPub;\npub trait AlphaTrait { fn go(&self); }\nfn alpha_priv() {}\n");
    let f_beta = sources.add_file("src/beta.rs", "use crate::AlphaPub;\nuse crate::AlphaTrait as _;\npub fn run(_x: AlphaPub) {}\n");

    let mut db = MemItemDb::new();
    let ext = db.add_crate("core_ext", Edition::Edition2021);
    let ext_root = def(ext.0, 0);
    let ext_opt = def(ext.0, 1);
    db.set_crate_root_def(ext, ext_root);
    db.add_module_def(ext_root, owned_def(ext_opt, "ExtOpt", ItemKind::Enum, VisKind::Public, false));

    let app = db.add_crate("app", Edition::Edition2021);
    db.set_crate_root_def(app, def(app.0, 0));
    db.add_extern_crate(app, "core_ext", ext);

    let snap = CrateSnapshot::build_clean(&db, &sources, app, f_lib);

    // Tier 1: private-only edit in leaf `src/beta.rs` (zero export changes).
    let beta_priv_edit = "use crate::AlphaPub;\nuse crate::AlphaTrait as _;\nuse core_ext::ExtOpt;\npub fn run(_x: AlphaPub) {}\nfn local_helper(_o: ExtOpt) {}\n";
    let mut ov1 = OverlaySourceDb::new(&sources, sources.len() as u32);
    ov1.set_file_override(f_beta, beta_priv_edit);
    let st_beta = parse(beta_priv_edit);
    let (mt1, inc1) = snap.with_dirty_files(&db, &ov1, ov1.dirty_files(), |fid| (fid == f_beta).then_some(&st_beta));
    assert!(mt1.is_none());
    let full1 = CrateDefMap::build_cached(&db, &ov1, &snap.mod_tree, ov1.dirty_files(), |_| None);
    assert_scopes_match_full_build(&snap.mod_tree, &inc1, &full1);

    // Tier 2: `pub` addition + item deletion in `src/alpha.rs` (propagates through `pub use alpha::*;` into `ModuleId::ROOT` and `beta.rs`).
    let alpha_pub_edit = "pub trait AlphaTrait { fn go(&self); }\npub struct NewAlphaExport;\npub use core_ext::ExtOpt as ReExportedOpt;\n";
    let mut ov2 = OverlaySourceDb::new(&sources, sources.len() as u32);
    ov2.set_file_override(f_alpha, alpha_pub_edit);
    let st_alpha = parse(alpha_pub_edit);
    let (mt2, inc2) = snap.with_dirty_files(&db, &ov2, ov2.dirty_files(), |fid| (fid == f_alpha).then_some(&st_alpha));
    assert!(mt2.is_none());
    let full2 = CrateDefMap::build_cached(&db, &ov2, &snap.mod_tree, ov2.dirty_files(), |_| None);
    assert_scopes_match_full_build(&snap.mod_tree, &inc2, &full2);
    assert!(inc2.scopes[ModuleId::ROOT.0 as usize].get("NewAlphaExport").types.is_some());
    assert!(inc2.scopes[ModuleId::ROOT.0 as usize].get("ReExportedOpt").types.is_some());
    assert!(inc2.scopes[ModuleId::ROOT.0 as usize].get("AlphaPub").types.is_none());
}
