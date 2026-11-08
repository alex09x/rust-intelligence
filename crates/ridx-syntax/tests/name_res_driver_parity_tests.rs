/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Regression suite for Phase `2.4` / `2.5` name-resolution parity with `ridx-driver`:
//! `T::f` via `where` & supertraits, `Self::Variant` / `Alias::Variant`, `or`-patterns,
//! shadowing precedence, `use` trees, `#[proc_macro_derive]`, UFCS in struct `TokenTree`,
//! `$OUT_DIR` `include!`, and `MacroCall` `TokenTree` resolution.

use ridx_syntax::name_res::{
    CrateDefMap, CrateSnapshot, Edition, FileNameRes, GlobalDefId, ImplTargetKey, ItemKind, MemItemDb, MemSourceDb, ModuleId, ModuleTree, OverlaySourceDb, OwnedDef, OwnedUse,
    ResTarget, VisKind,
};

fn find_nth(src: &str, needle: &str, nth: usize) -> u32 {
    let mut off = 0usize;
    for i in 0..=nth {
        let pos = src[off..].find(needle).unwrap_or_else(|| panic!("missing #{nth} of `{needle}`"));
        if i == nth {
            return (off + pos) as u32;
        }
        off += pos + needle.len();
    }
    unreachable!()
}

fn target_at(res: &FileNameRes, src: &str, needle: &str, nth: usize) -> ResTarget {
    let off = find_nth(src, needle, nth);
    res.ref_at_offset(off).unwrap_or_else(|| panic!("no ref at `{needle}` (#{nth}, byte {off})")).target
}

#[test]
fn resolves_where_bounds_after_sig_and_transitive_supertraits() {
    let mut db = MemItemDb::new();
    let ext_k = db.add_crate("ext", Edition::Edition2021);
    let app_k = db.add_crate("app", Edition::Edition2021);
    let ext_root = GlobalDefId::new(ext_k, 0);
    db.set_crate_root_def(ext_k, ext_root);
    db.add_extern_crate(app_k, "ext", ext_k);

    let base_tr = GlobalDefId::new(ext_k, 1);
    let base_ty = GlobalDefId::new(ext_k, 2);
    let base_fn = GlobalDefId::new(ext_k, 3);
    let sub_tr = GlobalDefId::new(ext_k, 4);
    let sub_out = GlobalDefId::new(ext_k, 5);
    db.add_module_def(ext_root, OwnedDef { id: base_tr, name: "IdxBase".into(), kind: ItemKind::Trait, vis: VisKind::Public, has_value_ctor: false });
    db.add_assoc_item(base_tr, OwnedDef { id: base_ty, name: "In".into(), kind: ItemKind::AssocTy, vis: VisKind::Public, has_value_ctor: false });
    db.add_assoc_item(base_tr, OwnedDef { id: base_fn, name: "convert".into(), kind: ItemKind::AssocFn, vis: VisKind::Public, has_value_ctor: false });
    db.add_module_def(ext_root, OwnedDef { id: sub_tr, name: "IdxSub".into(), kind: ItemKind::Trait, vis: VisKind::Public, has_value_ctor: false });
    db.add_assoc_item(sub_tr, OwnedDef { id: sub_out, name: "Out".into(), kind: ItemKind::AssocTy, vis: VisKind::Public, has_value_ctor: false });
    db.add_trait_supertrait(sub_tr, base_tr);

    let src = r#"
use ext::IdxSub;
pub trait LocalRoot { type Seed; fn boot() -> Self::Seed; }
pub trait LocalLeaf: LocalRoot { fn step(s: Self::Seed) -> Self::Seed { s } }
pub fn run_indexed<V>(input: V::In) -> V::Out where V: IdxSub { V::convert(input) }
pub fn run_local<L>(seed: L::Seed) -> L::Seed where L: LocalLeaf {
    let _ = L::boot();
    L::step(seed)
}
"#;
    let mut sources = MemSourceDb::new();
    let fid = sources.add_file("src/lib.rs", src);
    let mt = ModuleTree::build(&db, &sources, app_k, fid);
    let dm = CrateDefMap::build(&db, &sources, &mt);
    let res = dm.resolve_file_paths(&db, &sources, &mt, fid);

    // Trailing `where V: IdxSub` resolves `V::In` (from supertrait `IdxBase`), `V::Out` (on `IdxSub`), and `V::convert`!
    assert_eq!(res.ref_at_offset(find_nth(src, "V::In", 0) + 3).unwrap().target.effective_def_id(), Some(base_ty));
    assert_eq!(res.ref_at_offset(find_nth(src, "V::Out", 0) + 3).unwrap().target.effective_def_id(), Some(sub_out));
    assert_eq!(res.ref_at_offset(find_nth(src, "V::convert", 0) + 3).unwrap().target.effective_def_id(), Some(base_fn));

    // Local CST `trait LocalLeaf: LocalRoot` resolves `Self::Seed` in `LocalLeaf`, `L::Seed`, `L::boot()`, and `L::step()`.
    assert!(matches!(res.ref_at_offset(find_nth(src, "Self::Seed)", 0) + 6).unwrap().target, ResTarget::Def { kind: ItemKind::AssocTy, .. }));
    assert!(matches!(res.ref_at_offset(find_nth(src, "L::Seed)", 0) + 3).unwrap().target, ResTarget::Def { kind: ItemKind::AssocTy, .. }));
    assert!(matches!(res.ref_at_offset(find_nth(src, "L::boot()", 0) + 3).unwrap().target, ResTarget::Def { kind: ItemKind::AssocFn, .. }));
    assert!(matches!(res.ref_at_offset(find_nth(src, "L::step", 0) + 3).unwrap().target, ResTarget::Def { kind: ItemKind::AssocFn, .. }));
}

#[test]
fn resolves_self_and_type_alias_enum_variants_across_indexed_dirty_and_block_scopes() {
    let mut db = MemItemDb::new();
    let ext_k = db.add_crate("ext", Edition::Edition2021);
    let app_k = db.add_crate("app", Edition::Edition2021);
    let ext_root = GlobalDefId::new(ext_k, 0);
    db.set_crate_root_def(ext_k, ext_root);
    db.add_extern_crate(app_k, "ext", ext_k);

    let idx_enum = GlobalDefId::new(ext_k, 10);
    let idx_var_a = GlobalDefId::new(ext_k, 11);
    let idx_var_b = GlobalDefId::new(ext_k, 12);
    let idx_alias = GlobalDefId::new(ext_k, 13);
    db.add_module_def(ext_root, OwnedDef { id: idx_enum, name: "RemoteMode".into(), kind: ItemKind::Enum, vis: VisKind::Public, has_value_ctor: false });
    db.add_assoc_item(idx_enum, OwnedDef { id: idx_var_a, name: "Idle".into(), kind: ItemKind::Variant, vis: VisKind::Public, has_value_ctor: true });
    db.add_assoc_item(idx_enum, OwnedDef { id: idx_var_b, name: "Busy".into(), kind: ItemKind::Variant, vis: VisKind::Public, has_value_ctor: true });
    db.add_module_def(ext_root, OwnedDef { id: idx_alias, name: "RemoteAlias".into(), kind: ItemKind::TypeAlias, vis: VisKind::Public, has_value_ctor: false });
    db.set_type_alias_target(idx_alias, ImplTargetKey::Adt(idx_enum));

    let src = r#"
use ext::RemoteAlias;

pub enum Status {
    Ready,
    Code(u32),
    Named { ok: bool },
}

pub type StatusAlias = Status;

impl Status {
    pub fn check(self) -> u32 {
        match self {
            Self::Ready => 0,
            Self::Code(n) => n,
            Self::Named { ok } => if ok { 1 } else { 2 },
        }
    }

    pub fn make_default() -> Self {
        Self::Code(7)
    }
}

pub fn exercise(r: RemoteAlias) -> u32 {
    let base = match r {
        RemoteAlias::Idle => StatusAlias::Ready,
        RemoteAlias::Busy => StatusAlias::Code(9),
    };
    let blk = {
        let b = BlkAlias::Pair(3);
        enum BlkEnum { Unit, Pair(u32) }
        type BlkAlias = BlkEnum;
        match b {
            BlkAlias::Unit => 0,
            BlkAlias::Pair(v) => v,
        }
    };
    match base {
        StatusAlias::Code(k) => k + blk,
        _ => blk,
    }
}
"#;
    let mut sources = MemSourceDb::new();
    let fid = sources.add_file("src/lib.rs", src);
    let snap = CrateSnapshot::build_clean(&db, &sources, app_k, fid);
    let res = snap.clean_def_map.resolve_file_paths(&db, &sources, &snap.mod_tree, fid);

    // `Self::Ready`, `Self::Code`, `Self::Named` inside `impl Status` (pat + expr).
    assert!(matches!(res.ref_at_offset(find_nth(src, "Self::Ready", 0) + 6).unwrap().target, ResTarget::Def { kind: ItemKind::Variant, .. }));
    assert!(matches!(res.ref_at_offset(find_nth(src, "Self::Code(n)", 0) + 6).unwrap().target, ResTarget::Def { kind: ItemKind::Variant, .. }));
    assert!(matches!(res.ref_at_offset(find_nth(src, "Self::Named", 0) + 6).unwrap().target, ResTarget::Def { kind: ItemKind::Variant, .. }));
    assert!(matches!(res.ref_at_offset(find_nth(src, "Self::Code(7)", 0) + 6).unwrap().target, ResTarget::Def { kind: ItemKind::Variant, .. }));

    // Indexed `RemoteAlias::Idle` / `RemoteAlias::Busy` and local `StatusAlias::Ready` / `StatusAlias::Code`.
    assert_eq!(res.ref_at_offset(find_nth(src, "RemoteAlias::Idle", 0) + 13).unwrap().target.effective_def_id(), Some(idx_var_a));
    assert_eq!(res.ref_at_offset(find_nth(src, "RemoteAlias::Busy", 0) + 13).unwrap().target.effective_def_id(), Some(idx_var_b));
    assert!(matches!(res.ref_at_offset(find_nth(src, "StatusAlias::Ready", 0) + 13).unwrap().target, ResTarget::Def { kind: ItemKind::Variant, .. }));
    assert!(matches!(res.ref_at_offset(find_nth(src, "StatusAlias::Code(9)", 0) + 13).unwrap().target, ResTarget::Def { kind: ItemKind::Variant, .. }));

    // Block-local forward-referenced `BlkAlias::Pair` and `BlkAlias::Unit`.
    assert!(matches!(res.ref_at_offset(find_nth(src, "BlkAlias::Pair(3)", 0) + 10).unwrap().target, ResTarget::Def { kind: ItemKind::Variant, .. }));
    assert!(matches!(res.ref_at_offset(find_nth(src, "BlkAlias::Unit", 0) + 10).unwrap().target, ResTarget::Def { kind: ItemKind::Variant, .. }));

    // Dirty overlay adds a new variant + new alias in edited buffer and resolves `DirtyAlias::Fresh`.
    let dirty_src = src.replace("Ready,", "Ready,\n    Fresh,") + "\npub type DirtyAlias = Status;\npub fn fresh() -> Status { DirtyAlias::Fresh }\n";
    let mut ov = OverlaySourceDb::new(&sources, 1);
    ov.set_file_override(fid, dirty_src.clone());
    let (mt_opt, dirty_dm) = snap.with_dirty_files(&db, &ov, &[fid], |_| None);
    let dirty_res = dirty_dm.resolve_file_paths(&db, &ov, mt_opt.as_ref().unwrap_or(&snap.mod_tree), fid);
    let fresh_t = dirty_res.ref_at_offset(find_nth(&dirty_src, "DirtyAlias::Fresh", 0) + 12).unwrap().target;
    assert!(matches!(fresh_t, ResTarget::Def { kind: ItemKind::Variant, .. }));
    assert!(fresh_t.effective_def_id().unwrap().is_overlay());
}

#[test]
fn resolves_multi_arm_or_patterns_and_nested_shadowing_precedence() {
    let mut db = MemItemDb::new();
    let prel_k = db.add_crate("prel", Edition::Edition2021);
    let app_k = db.add_crate("app", Edition::Edition2021);
    let prel_mod = GlobalDefId::new(prel_k, 0);
    let prel_tok = GlobalDefId::new(prel_k, 1);
    db.set_crate_root_def(prel_k, prel_mod);
    db.set_prelude_module(app_k, prel_mod);
    db.add_module_def(prel_mod, OwnedDef { id: prel_tok, name: "Item".into(), kind: ItemKind::Struct, vis: VisKind::Public, has_value_ctor: true });

    let src = r#"
pub mod glob_src { pub struct Item; }
pub mod explicit_src { pub struct Item; }
use glob_src::*;
use explicit_src::Item;

pub enum Msg {
    Pair(u32, u32),
    Swap(u32, u32),
    Empty,
}

pub fn compute() -> u32 { 100 }

pub fn eval(m: Msg, compute: u32) -> (Item, u32) {
    use Msg::Empty;
    let total = match m {
        whole @ (Msg::Pair(a, b) | Msg::Swap(b, a)) if a + b > 0 => {
            let _keep = whole;
            a + b + compute
        }
        Empty => {
            let compute = {
                let compute = compute + 1;
                compute * 2
            };
            compute
        }
        _ => 0,
    };
    if let Msg::Pair(x, _) | Msg::Swap(_, x) = Msg::Pair(total, 0) {
        (Item, x)
    } else {
        (Item, total)
    }
}
"#;
    let mut sources = MemSourceDb::new();
    let fid = sources.add_file("src/lib.rs", src);
    let mt = ModuleTree::build(&db, &sources, app_k, fid);
    let dm = CrateDefMap::build(&db, &sources, &mt);
    let res = dm.resolve_file_paths(&db, &sources, &mt, fid);

    // Explicit `use explicit_src::Item` beats `use glob_src::*` AND prelude `prel::Item`.
    let item_ty = target_at(&res, src, "Item, u32", 0);
    let explicit_def = target_at(&res, src, "Item;\n\npub enum", 0);
    assert_eq!(item_ty, explicit_def);
    assert_ne!(item_ty.effective_def_id(), Some(prel_tok));

    // `whole @ (Msg::Pair(a, b) | Msg::Swap(b, a))`: `whole`, `a`, and `b` unify to primary arm bindings!
    let whole_ref = target_at(&res, src, "whole;", 0);
    assert!(whole_ref.is_local());
    let a_guard = match target_at(&res, src, "a + b > 0", 0) {
        ResTarget::Local(id) => id,
        other => panic!("expected Local `a`, got {other:?}"),
    };
    let b_guard = match res.ref_at_offset(find_nth(src, "a + b > 0", 0) + 4).unwrap().target {
        ResTarget::Local(id) => id,
        other => panic!("expected Local `b`, got {other:?}"),
    };
    assert_eq!(res.local_data(a_guard).name_start, find_nth(src, "Pair(a, b)", 0) + 5);
    assert_eq!(res.local_data(b_guard).name_start, find_nth(src, "Pair(a, b)", 0) + 8);
    assert_eq!(target_at(&res, src, "a + b + compute", 0), ResTarget::Local(a_guard));

    // Bare `Empty =>` resolves to `Msg::Empty` variant, NOT a fresh local binding!
    assert!(matches!(target_at(&res, src, "Empty =>", 0), ResTarget::Def { kind: ItemKind::Variant, .. }));

    // Parameter `compute: u32` shadows `pub fn compute()`, and nested `let compute = ...` ribs stay distinct!
    let param_c = match res.ref_at_offset(find_nth(src, "+ compute", 0) + 2).unwrap().target {
        ResTarget::Local(id) => id,
        other => panic!("expected param `compute`, got {other:?}"),
    };
    let inner_rhs_c = match res.ref_at_offset(find_nth(src, "= compute + 1", 0) + 2).unwrap().target {
        ResTarget::Local(id) => id,
        other => panic!("expected param `compute` on RHS, got {other:?}"),
    };
    assert_eq!(param_c, inner_rhs_c);
    let inner_mul_c = match target_at(&res, src, "compute * 2", 0) {
        ResTarget::Local(id) => id,
        other => panic!("expected inner `compute`, got {other:?}"),
    };
    let outer_ret_c = match target_at(&res, src, "compute\n        }", 1) {
        ResTarget::Local(id) => id,
        other => panic!("expected middle `compute`, got {other:?}"),
    };
    assert_ne!(inner_mul_c, param_c);
    assert_ne!(outer_ret_c, inner_mul_c);
    assert_ne!(outer_ret_c, param_c);

    // `if let Msg::Pair(x, _) | Msg::Swap(_, x)` binds `x` across `|` branches into then-block `(Item, x)`.
    assert!(res.ref_at_offset(find_nth(src, "(Item, x)", 0) + 7).unwrap().target.is_local());
}

#[test]
fn resolves_2_5_corpus_edge_cases_use_trees_ufcs_struct_tt_out_dir_and_macro_tt() {
    let mut db = MemItemDb::new();
    let ext_k = db.add_crate("ext", Edition::Edition2021);
    let app_k = db.add_crate("app", Edition::Edition2021);
    let ext_root = GlobalDefId::new(ext_k, 0);
    let app_root = GlobalDefId::new(app_k, 0);
    db.set_crate_root_def(ext_k, ext_root);
    db.set_crate_root_def(app_k, app_root);
    db.add_extern_crate(app_k, "ext", ext_k);

    // `ext::sub`: module with both `Trait` and `Macro` sharing name `Reflect` + glob export.
    let sub_mod = GlobalDefId::new(ext_k, 1);
    let refl_tr = GlobalDefId::new(ext_k, 2);
    let refl_mac = GlobalDefId::new(ext_k, 3);
    let iter_tr = GlobalDefId::new(ext_k, 4);
    let iter_assoc = GlobalDefId::new(ext_k, 5);
    let box_adt = GlobalDefId::new(ext_k, 6);
    db.add_module_def(ext_root, OwnedDef { id: sub_mod, name: "sub".into(), kind: ItemKind::Mod, vis: VisKind::Public, has_value_ctor: false });
    db.add_module_def(sub_mod, OwnedDef { id: refl_tr, name: "Reflect".into(), kind: ItemKind::Trait, vis: VisKind::Public, has_value_ctor: false });
    db.add_module_def(sub_mod, OwnedDef { id: refl_mac, name: "Reflect".into(), kind: ItemKind::Macro, vis: VisKind::Public, has_value_ctor: false });
    db.add_module_def(ext_root, OwnedDef { id: iter_tr, name: "IntoIter".into(), kind: ItemKind::Trait, vis: VisKind::Public, has_value_ctor: false });
    db.add_assoc_item(iter_tr, OwnedDef { id: iter_assoc, name: "Iter".into(), kind: ItemKind::AssocTy, vis: VisKind::Public, has_value_ctor: false });
    db.add_module_def(ext_root, OwnedDef { id: box_adt, name: "Buf".into(), kind: ItemKind::Struct, vis: VisKind::Public, has_value_ctor: false });

    // Simulate `$OUT_DIR` generated item + re-export indexed under `app_root` (`def_file == FileId::NONE`).
    let gen_struct = GlobalDefId::new(app_k, 50);
    let gen_reexport = GlobalDefId::new(ext_k, 51);
    db.add_module_def(app_root, OwnedDef { id: gen_struct, name: "OutDirTable".into(), kind: ItemKind::Struct, vis: VisKind::Public, has_value_ctor: true });
    db.add_module_def(ext_root, OwnedDef { id: gen_reexport, name: "OutDirConst".into(), kind: ItemKind::Const, vis: VisKind::Public, has_value_ctor: false });
    db.add_module_use(
        app_root,
        OwnedUse {
            name: "OutDirConst".into(),
            type_def: GlobalDefId::NONE,
            value_def: gen_reexport,
            macro_def: GlobalDefId::NONE,
            vis: VisKind::Public,
            is_glob: false,
            is_reexport: true,
            is_prelude: false,
            is_underscore: false,
            prim_ty_code: 0,
        },
    );

    let src = r#"
extern crate ext as ext_alias;
use ext_alias::sub::{self};
use ext_alias::sub::*;
use ext::sub::Reflect;
include!(concat!(env!("OUT_DIR"), "/generated.rs"));
#[proc_macro_derive(CustomDerive)]
pub fn derive_custom(_ts: OutDirTable) -> OutDirTable { OutDirTable }
pub struct Holder<T> { pub iter: <ext::Buf as ext::IntoIter>::Iter, pub marker: T }
pub fn run_macros(seed: u32) -> (OutDirTable, u32) {
    let msg = format!("{seed} = {}", OutDirConst);
    let _ = vec![msg];
    (OutDirTable, seed)
}
"#;
    let mut sources = MemSourceDb::new();
    let fid = sources.add_file("src/lib.rs", src);
    let mt = ModuleTree::build(&db, &sources, app_k, fid);
    let dm = CrateDefMap::build(&db, &sources, &mt);
    let res = dm.resolve_file_paths(&db, &sources, &mt, fid);

    // `extern crate ext as ext_alias;` emits crate-root ref on `ext`.
    assert_eq!(target_at(&res, src, "ext as ext_alias", 0).effective_def_id(), Some(ext_root));
    // `use ext_alias::sub::{self};` emits exactly ONE ref on `self` -> `sub_mod`.
    let self_off = find_nth(src, "self}", 0);
    let self_refs: Vec<_> = res.refs.iter().filter(|r| r.start == self_off).collect();
    assert_eq!(self_refs.len(), 1);
    assert_eq!(self_refs[0].target.effective_def_id(), Some(sub_mod));
    // `use ext_alias::sub::*;` emits prefix refs on `ext_alias` and `sub`.
    assert_eq!(target_at(&res, src, "sub::*", 0).effective_def_id(), Some(sub_mod));
    // `use ext::sub::Reflect;` emits BOTH `Trait` (`refl_tr`) and `Macro` (`refl_mac`) on `Reflect`!
    let refl_off = find_nth(src, "Reflect;", 0);
    let refl_defs: Vec<_> = res.refs.iter().filter(|r| r.start == refl_off).filter_map(|r| r.target.effective_def_id()).collect();
    assert!(refl_defs.contains(&refl_tr) && refl_defs.contains(&refl_mac));
    // `#[proc_macro_derive(CustomDerive)]` exports `CustomDerive` in root `macros` scope.
    assert!(dm.scopes[ModuleId::ROOT.0 as usize].get("CustomDerive").macros.is_some());
    // UFCS `<ext::Buf as ext::IntoIter>::Iter` inside `struct Holder<T>` `TokenTree` resolves `Buf`, `IntoIter`, and `Iter`!
    assert_eq!(res.ref_at_offset(find_nth(src, "ext::Buf", 0) + 5).unwrap().target.effective_def_id(), Some(box_adt));
    assert_eq!(res.ref_at_offset(find_nth(src, "ext::IntoIter", 0) + 5).unwrap().target.effective_def_id(), Some(iter_tr));
    assert_eq!(res.ref_at_offset(find_nth(src, ">::Iter", 0) + 3).unwrap().target.effective_def_id(), Some(iter_assoc));
    // `$OUT_DIR` `include!` supplements `OutDirTable` and `OutDirConst`, and `format!("{seed} = {}", OutDirConst)` resolves both!
    assert_eq!(target_at(&res, src, "OutDirTable,", 0).effective_def_id(), Some(gen_struct));
    assert!(res.ref_at_offset(find_nth(src, "{seed}", 0) + 1).unwrap().target.is_local());
    assert_eq!(target_at(&res, src, "OutDirConst)", 0).effective_def_id(), Some(gen_reexport));
    assert!(target_at(&res, src, "msg]", 0).is_local());
}
