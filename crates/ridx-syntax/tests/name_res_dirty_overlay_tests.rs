/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for Step 2.3: dirty-file local items, `impl` blocks, trait associated items,
//! type-alias `impl` heads, blanket/negative `impl`s, and incremental [`CrateSnapshot`] splicing.

use ridx_syntax::name_res::{
    CrateId, CrateSnapshot, Edition, GlobalDefId, ImplTargetKey, IndexedImpl, ItemKind, MemItemDb, MemSourceDb, ModuleId, NsMask, OverlaySourceDb, OwnedDef, PathAnchor, VisKind,
};

fn def(krate: u32, idx: u32) -> GlobalDefId {
    GlobalDefId::new(CrateId(krate), idx)
}

fn owned_def(id: GlobalDefId, name: &str, kind: ItemKind, vis: VisKind, ctor: bool) -> OwnedDef {
    OwnedDef { id, name: name.into(), kind, vis, has_value_ctor: ctor }
}

fn mk_impl(id: GlobalDefId, file: ridx_syntax::name_res::FileId, target: ImplTargetKey, trait_def: GlobalDefId) -> IndexedImpl {
    IndexedImpl { id, file, target, trait_def, is_negative: false }
}

#[test]
fn dirty_file_overrides_deleted_and_added_items_preserving_surviving_def_ids() {
    let mut sources = MemSourceDb::new();
    let f_lib = sources.add_file("src/lib.rs", "pub mod worker;\npub use worker::*;\npub struct RootKeep;\n");
    let f_worker = sources.add_file(
        "src/worker.rs",
        "pub struct OldStruct;\npub struct SurvivingStruct(pub u32);\npub fn old_fn() {}\n\
         pub fn surviving_fn() -> u32 { 1 }\npub enum Status { Idle, RemovedVar(u8) }\n",
    );

    let mut db = MemItemDb::new();
    let app = db.add_crate("app", Edition::Edition2021);
    let root_def = def(app.0, 0);
    let worker_mod_def = def(app.0, 1);
    db.set_crate_root_def(app, root_def);

    // Clean `src/lib.rs` items, including a synthetic proc-macro item `CleanDerivedHelper` only in `ItemDb`.
    let root_keep_def = def(app.0, 2);
    let clean_macro_def = def(app.0, 3);
    db.add_module_def_in_file(root_def, f_lib, owned_def(worker_mod_def, "worker", ItemKind::Mod, VisKind::Public, false));
    db.add_module_def_in_file(root_def, f_lib, owned_def(root_keep_def, "RootKeep", ItemKind::Struct, VisKind::Public, true));
    db.add_module_def_in_file(root_def, f_lib, owned_def(clean_macro_def, "clean_derived_helper", ItemKind::Fn, VisKind::Public, false));

    // Indexed `src/worker.rs` items + a stale macro-generated fn attributed to `f_worker`.
    let old_struct_def = def(app.0, 10);
    let surv_struct_def = def(app.0, 11);
    let old_fn_def = def(app.0, 12);
    let surv_fn_def = def(app.0, 13);
    let status_def = def(app.0, 14);
    let idle_var_def = def(app.0, 15);
    let removed_var_def = def(app.0, 16);
    let stale_worker_macro_def = def(app.0, 17);

    for d in [
        owned_def(old_struct_def, "OldStruct", ItemKind::Struct, VisKind::Public, true),
        owned_def(surv_struct_def, "SurvivingStruct", ItemKind::Struct, VisKind::Public, true),
        owned_def(old_fn_def, "old_fn", ItemKind::Fn, VisKind::Public, false),
        owned_def(surv_fn_def, "surviving_fn", ItemKind::Fn, VisKind::Public, false),
        owned_def(status_def, "Status", ItemKind::Enum, VisKind::Public, false),
        owned_def(stale_worker_macro_def, "stale_macro_gen", ItemKind::Fn, VisKind::Public, false),
    ] {
        db.add_module_def_in_file(worker_mod_def, f_worker, d);
    }
    db.add_assoc_item(status_def, owned_def(idle_var_def, "Idle", ItemKind::Variant, VisKind::Public, true));
    db.add_assoc_item(status_def, owned_def(removed_var_def, "RemovedVar", ItemKind::Variant, VisKind::Public, true));

    let snap = CrateSnapshot::build_clean(&db, &sources, app, f_lib);
    assert_eq!(snap.clean_def_map.scope(ModuleId::ROOT).get("OldStruct").types.map(|b| b.def), Some(old_struct_def));
    assert_eq!(snap.clean_def_map.scope(ModuleId::ROOT).get("clean_derived_helper").values.map(|b| b.def), Some(clean_macro_def));

    // Edit ONLY `src/worker.rs` in an unsaved buffer: delete `OldStruct`/`old_fn`/`RemovedVar`, add new items.
    let mut overlay = OverlaySourceDb::new(&sources, sources.len() as u32);
    overlay.set_file_override(
        f_worker,
        r#"
pub struct SurvivingStruct(pub u32);
pub struct BrandNewStruct;
pub fn surviving_fn() -> u32 { 42 }
pub fn brand_new_fn() -> BrandNewStruct { BrandNewStruct }
pub enum Status {
    Idle,
    FreshVar { code: u16 },
}

#[cfg(test)]
mod tests {}
"#,
    );

    let (rebuilt_tree, dm) = snap.with_dirty_files(&db, &overlay, overlay.dirty_files(), |_| None);
    assert!(rebuilt_tree.is_none(), "disabled #[cfg(test)] mod tests must take the 1-file splice fast path");
    assert!(dm.is_dirty_file(f_worker) && !dm.is_dirty_file(f_lib));

    let root_sc = dm.scope(ModuleId::ROOT);
    // Deleted items & stale dirty-file macro items vanish from both `worker` and `pub use worker::*;`:
    assert!(root_sc.get("OldStruct").is_empty());
    assert!(root_sc.get("old_fn").is_empty());
    assert!(root_sc.get("stale_macro_gen").is_empty());
    assert!(dm.resolve_path(&db, &snap.mod_tree, ModuleId::ROOT, PathAnchor::Bare, &["Status", "RemovedVar"]).is_empty());

    // Clean file's synthetic `ItemDb`-only item stays intact:
    assert_eq!(root_sc.get("clean_derived_helper").values.map(|b| b.def), Some(clean_macro_def));

    // Surviving items in dirty file keep their exact compiler `GlobalDefId`s:
    assert_eq!(root_sc.get("SurvivingStruct").types.map(|b| b.def), Some(surv_struct_def));
    assert_eq!(root_sc.get("SurvivingStruct").values.map(|b| b.def), Some(surv_struct_def));
    assert_eq!(root_sc.get("surviving_fn").values.map(|b| b.def), Some(surv_fn_def));
    let idle_res = dm.resolve_path(&db, &snap.mod_tree, ModuleId::ROOT, PathAnchor::Bare, &["Status", "Idle"]);
    assert_eq!(idle_res.values.map(|b| b.def), Some(idle_var_def));

    // Newly added items receive deterministic `LOCAL_OVERLAY` `GlobalDefId`s:
    let new_struct_ty = root_sc.get("BrandNewStruct").types.expect("BrandNewStruct type").def;
    let new_fn_val = root_sc.get("brand_new_fn").values.expect("brand_new_fn val").def;
    assert!(new_struct_ty.is_overlay() && new_fn_val.is_overlay());
    let fresh_var = dm.resolve_path(&db, &snap.mod_tree, ModuleId::ROOT, PathAnchor::Bare, &["Status", "FreshVar"]);
    assert!(fresh_var.types.expect("record variant in TYPE ns").def.is_overlay());
    assert!(fresh_var.values.is_none(), "record struct variant has no value ctor");
}

#[test]
fn dirty_file_impls_inherent_trait_blanket_primitive_and_type_alias_merge_over_snapshot() {
    let mut sources = MemSourceDb::new();
    let f_lib = sources.add_file(
        "src/lib.rs",
        r#"
pub mod ops;

pub struct Buffer;
pub trait Encode {
    type Out;
    const TAG: u8;
    fn encode(&self) -> Self::Out;
    fn summary(&self) -> usize { 0 }
}
pub trait BlanketMark {
    fn mark(&self) -> bool;
}
pub auto trait SendMarker {}

impl Buffer {
    pub fn clean_len(&self) -> usize { 0 }
}
"#,
    );
    let f_ops = sources.add_file(
        "src/ops.rs",
        r#"
use crate::{Buffer, Encode};

impl Buffer {
    pub fn stale_op(&self) {}
    pub fn keep_op(&self) -> u32 { 1 }
}

impl Encode for Buffer {
    type Out = u32;
    const TAG: u8 = 1;
    fn encode(&self) -> u32 { 0 }
}
"#,
    );

    let mut db = MemItemDb::new();
    let app = db.add_crate("app", Edition::Edition2021);
    let root_def = def(app.0, 0);
    let ops_mod_def = def(app.0, 1);
    let buf_def = def(app.0, 10);
    let encode_def = def(app.0, 20);
    let blanket_def = def(app.0, 25);
    let send_def = def(app.0, 26);
    db.set_crate_root_def(app, root_def);

    for d in [
        owned_def(ops_mod_def, "ops", ItemKind::Mod, VisKind::Public, false),
        owned_def(buf_def, "Buffer", ItemKind::Struct, VisKind::Public, true),
        owned_def(encode_def, "Encode", ItemKind::Trait, VisKind::Public, false),
        owned_def(blanket_def, "BlanketMark", ItemKind::Trait, VisKind::Public, false),
        owned_def(send_def, "SendMarker", ItemKind::Trait, VisKind::Public, false),
    ] {
        db.add_module_def_in_file(root_def, f_lib, d);
    }
    let summary_default_def = def(app.0, 24);
    db.add_assoc_item(encode_def, owned_def(def(app.0, 21), "Out", ItemKind::TypeAlias, VisKind::Public, false));
    db.add_assoc_item(encode_def, owned_def(def(app.0, 22), "TAG", ItemKind::Const, VisKind::Public, false));
    db.add_assoc_item(encode_def, owned_def(def(app.0, 23), "encode", ItemKind::Fn, VisKind::Public, true));
    db.add_assoc_item(encode_def, owned_def(summary_default_def, "summary", ItemKind::Fn, VisKind::Public, true));

    // Clean inherent impl in `src/lib.rs` + indexed impls previously in `src/ops.rs`:
    let (clean_impl_id, clean_len_def) = (def(app.0, 40), def(app.0, 41));
    db.add_impl(mk_impl(clean_impl_id, f_lib, ImplTargetKey::Adt(buf_def), GlobalDefId::NONE));
    db.add_assoc_item(clean_impl_id, owned_def(clean_len_def, "clean_len", ItemKind::Fn, VisKind::Public, true));

    let (dirty_inh_impl_id, stale_op_def, keep_op_def) = (def(app.0, 50), def(app.0, 51), def(app.0, 52));
    db.add_impl(mk_impl(dirty_inh_impl_id, f_ops, ImplTargetKey::Adt(buf_def), GlobalDefId::NONE));
    db.add_assoc_item(dirty_inh_impl_id, owned_def(stale_op_def, "stale_op", ItemKind::Fn, VisKind::Public, true));
    db.add_assoc_item(dirty_inh_impl_id, owned_def(keep_op_def, "keep_op", ItemKind::Fn, VisKind::Public, true));

    let (dirty_tr_impl_id, enc_fn_def) = (def(app.0, 60), def(app.0, 63));
    db.add_impl(mk_impl(dirty_tr_impl_id, f_ops, ImplTargetKey::Adt(buf_def), encode_def));
    db.add_assoc_item(dirty_tr_impl_id, owned_def(def(app.0, 61), "Out", ItemKind::TypeAlias, VisKind::Public, false));
    db.add_assoc_item(dirty_tr_impl_id, owned_def(def(app.0, 62), "TAG", ItemKind::Const, VisKind::Public, false));
    db.add_assoc_item(dirty_tr_impl_id, owned_def(enc_fn_def, "encode", ItemKind::Fn, VisKind::Public, true));

    let snap = CrateSnapshot::build_clean(&db, &sources, app, f_lib);
    let ops_mod = snap.mod_tree.children(ModuleId::ROOT)[0].child;

    let mut overlay = OverlaySourceDb::new(&sources, sources.len() as u32);
    overlay.set_file_override(
        f_ops,
        r#"
use crate::{BlanketMark, Buffer, Encode, SendMarker};

pub type BufAlias = Buffer;

impl Buffer {
    pub const CAP: usize = 64;
    pub fn keep_op(&self) -> u32 { 2 }
    pub fn new_op(&mut self) {}
    fn private_helper() {}
}

impl BufAlias {
    pub fn via_alias() -> Buffer { Buffer }
}

impl Encode for Buffer {
    type Out = u64;
    const TAG: u8 = 9;
    fn encode(&self) -> u64 { 99 }
}

impl<T> BlanketMark for T {
    fn mark(&self) -> bool { true }
}

impl Encode for &mut [u8] {
    type Out = usize;
    const TAG: u8 = 2;
    fn encode(&self) -> usize { self.len() }
}

impl !SendMarker for Buffer {}
"#,
    );

    let (rebuilt_mt, dm) = snap.with_dirty_files(&db, &overlay, overlay.dirty_files(), |_| None);
    assert!(rebuilt_mt.is_none(), "editing impls/type aliases must stay on the fast splice path");

    let buf_key = ImplTargetKey::Adt(buf_def);
    // 1. Stale `stale_op` is gone; clean `clean_len` + surviving `keep_op` keep compiler DefIds:
    assert!(dm.resolve_inherent_assoc(&db, &snap.mod_tree, ModuleId::ROOT, buf_key, "stale_op", NsMask::VALUE).is_none());
    assert_eq!(dm.resolve_inherent_assoc(&db, &snap.mod_tree, ModuleId::ROOT, buf_key, "clean_len", NsMask::VALUE).map(|a| a.def), Some(clean_len_def));
    assert_eq!(dm.resolve_inherent_assoc(&db, &snap.mod_tree, ModuleId::ROOT, buf_key, "keep_op", NsMask::VALUE).map(|a| a.def), Some(keep_op_def));

    // 2. Newly added `new_op`, `CAP`, and `impl BufAlias { fn via_alias }` resolve on `Buffer`:
    let new_op = dm.resolve_inherent_assoc(&db, &snap.mod_tree, ModuleId::ROOT, buf_key, "new_op", NsMask::VALUE).unwrap();
    assert!(new_op.def.is_overlay() && new_op.has_self_receiver);
    let cap_c = dm.resolve_inherent_assoc(&db, &snap.mod_tree, ModuleId::ROOT, buf_key, "CAP", NsMask::VALUE).unwrap();
    assert!(cap_c.def.is_overlay() && cap_c.kind == ItemKind::Const);
    let via_alias = dm.resolve_inherent_assoc(&db, &snap.mod_tree, ModuleId::ROOT, buf_key, "via_alias", NsMask::VALUE).unwrap();
    assert!(via_alias.def.is_overlay() && !via_alias.has_self_receiver);

    // 3. Private inherent fn `private_helper` is visible inside `ops`, hidden in `ROOT`:
    assert!(dm.resolve_inherent_assoc(&db, &snap.mod_tree, ops_mod, buf_key, "private_helper", NsMask::VALUE).is_some());
    assert!(dm.resolve_inherent_assoc(&db, &snap.mod_tree, ModuleId::ROOT, buf_key, "private_helper", NsMask::VALUE).is_none());

    // 4. `<Buffer as Encode>` preserves `enc_fn_def` and falls back to trait default `summary`:
    let enc_m = dm.resolve_trait_impl_assoc(&db, buf_key, encode_def, "encode", NsMask::VALUE).unwrap();
    assert_eq!(enc_m.def, enc_fn_def);
    let sum_m = dm.resolve_trait_impl_assoc(&db, buf_key, encode_def, "summary", NsMask::VALUE).unwrap();
    assert_eq!(sum_m.def, summary_default_def);

    // 5. Blanket `impl<T> BlanketMark for T` resolves as `ImplTargetKey::Param`, not an ADT:
    let mark_m = dm.resolve_trait_impl_assoc(&db, buf_key, blanket_def, "mark", NsMask::VALUE).unwrap();
    assert!(mark_m.def.is_overlay());

    // 6. `&mut [u8]` impl target + negative `impl !SendMarker for Buffer`:
    let ref_mut_key = ImplTargetKey::Ref { is_mut: true };
    assert!(dm.resolve_trait_impl_assoc(&db, ref_mut_key, encode_def, "encode", NsMask::VALUE).is_some());
    let mut buf_impls = Vec::new();
    dm.merged_impls_for_target(&db, buf_key, false, &mut buf_impls);
    assert!(buf_impls.iter().any(|i| i.trait_def == send_def && i.is_negative));
}

#[test]
fn dirty_trait_definition_and_unsaved_new_submodule_rebuild_tree_when_needed() {
    let mut sources = MemSourceDb::new();
    let f_lib = sources.add_file(
        "src/lib.rs",
        r#"
pub mod svc;
"#,
    );
    let f_svc = sources.add_file(
        "src/svc.rs",
        r#"
pub trait Service {
    fn ping(&self) -> bool;
    fn old_call(&self);
}
"#,
    );

    let mut db = MemItemDb::new();
    let app = db.add_crate("app", Edition::Edition2021);
    let root_def = def(app.0, 0);
    let svc_mod_def = def(app.0, 1);
    let svc_trait_def = def(app.0, 10);
    let ping_def = def(app.0, 11);
    let old_call_def = def(app.0, 12);
    db.set_crate_root_def(app, root_def);
    db.add_module_def_in_file(root_def, f_lib, owned_def(svc_mod_def, "svc", ItemKind::Mod, VisKind::Public, false));
    db.add_module_def_in_file(svc_mod_def, f_svc, owned_def(svc_trait_def, "Service", ItemKind::Trait, VisKind::Public, false));
    db.add_assoc_item(svc_trait_def, owned_def(ping_def, "ping", ItemKind::Fn, VisKind::Public, true));
    db.add_assoc_item(svc_trait_def, owned_def(old_call_def, "old_call", ItemKind::Fn, VisKind::Public, true));

    let snap = CrateSnapshot::build_clean(&db, &sources, app, f_lib);

    // Edit `src/svc.rs` trait + add brand-new unsaved `src/extra.rs` wired from dirty `src/lib.rs`!
    let mut overlay = OverlaySourceDb::new(&sources, sources.len() as u32);
    overlay.set_file_override(
        f_svc,
        r#"
pub trait Service {
    type Output;
    fn ping(&self) -> bool;
    fn shutdown(&mut self);
}
"#,
    );
    overlay.set_file_override(
        f_lib,
        r#"
pub mod svc;
pub mod extra;
pub use extra::FreshWidget;
"#,
    );
    overlay.upsert_path(
        "src/extra.rs",
        r#"
pub struct FreshWidget;
impl FreshWidget {
    pub fn build() -> Self { FreshWidget }
}
"#,
    );

    let (rebuilt_mt, dm) = snap.with_dirty_files(&db, &overlay, overlay.dirty_files(), |_| None);
    let new_mt = rebuilt_mt.expect("adding `pub mod extra;` must rebuild ModuleTree");
    assert_eq!(new_mt.modules.len(), 3);

    // Dirty trait `Service` masks `old_call`, preserves `ping_def`, and exposes `Output` + `shutdown`:
    assert!(dm.resolve_trait_assoc(&db, svc_trait_def, "old_call", NsMask::VALUE).is_none());
    assert_eq!(dm.resolve_trait_assoc(&db, svc_trait_def, "ping", NsMask::VALUE).map(|a| a.def), Some(ping_def));
    assert!(dm.resolve_trait_assoc(&db, svc_trait_def, "Output", NsMask::TYPE).unwrap().def.is_overlay());
    assert!(dm.resolve_trait_assoc(&db, svc_trait_def, "shutdown", NsMask::VALUE).unwrap().def.is_overlay());

    // Brand-new unsaved submodule item & inherent impl resolve cleanly:
    let widget_def = dm.scope(ModuleId::ROOT).get("FreshWidget").types.expect("FreshWidget re-export").def;
    assert!(widget_def.is_overlay());
    let build_fn = dm.resolve_inherent_assoc(&db, &new_mt, ModuleId::ROOT, ImplTargetKey::Adt(widget_def), "build", NsMask::VALUE).expect("FreshWidget::build");
    assert!(build_fn.def.is_overlay() && !build_fn.has_self_receiver);
}
