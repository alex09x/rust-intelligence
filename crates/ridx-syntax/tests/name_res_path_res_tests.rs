/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use ridx_syntax::name_res::{
    CrateDefMap, CrateId, Edition, FileId, FileNameRes, GlobalDefId, ImplTargetKey, IndexedImpl, ItemKind, MemItemDb, MemSourceDb, ModuleTree, OwnedDef, OwnedUse, PrimTy,
    ResTarget, VisKind,
};

struct TestEnv {
    db: MemItemDb,
    core_k: CrateId,
    app_k: CrateId,
    opt_def: GlobalDefId,
    some_def: GlobalDefId,
    none_def: GlobalDefId,
    from_trait: GlobalDefId,
    from_fn: GlobalDefId,
    u32_max: GlobalDefId,
}

fn setup_env() -> TestEnv {
    let mut db = MemItemDb::new();
    let core_k = db.add_crate("core", Edition::Edition2021);
    let app_k = db.add_crate("app", Edition::Edition2021);
    let core_root = GlobalDefId::new(core_k, 0);
    let core_prel = GlobalDefId::new(core_k, 1);
    db.set_crate_root_def(core_k, core_root);
    db.set_prelude_module(core_k, core_prel);
    db.set_prelude_module(app_k, core_prel);
    db.add_extern_crate(app_k, "core", core_k);

    let opt_def = GlobalDefId::new(core_k, 10);
    let some_def = GlobalDefId::new(core_k, 11);
    let none_def = GlobalDefId::new(core_k, 12);
    db.add_module_def(core_prel, OwnedDef { id: opt_def, name: "Option".into(), kind: ItemKind::Enum, vis: VisKind::Public, has_value_ctor: false });
    db.add_assoc_item(opt_def, OwnedDef { id: some_def, name: "Some".into(), kind: ItemKind::Variant, vis: VisKind::Public, has_value_ctor: true });
    db.add_assoc_item(opt_def, OwnedDef { id: none_def, name: "None".into(), kind: ItemKind::Variant, vis: VisKind::Public, has_value_ctor: true });
    db.add_module_use(
        core_prel,
        OwnedUse {
            name: "Some".into(),
            type_def: some_def,
            value_def: some_def,
            macro_def: GlobalDefId::NONE,
            vis: VisKind::Public,
            is_glob: false,
            is_reexport: true,
            is_prelude: false,
            is_underscore: false,
            prim_ty_code: 0,
        },
    );
    db.add_module_use(
        core_prel,
        OwnedUse {
            name: "None".into(),
            type_def: none_def,
            value_def: none_def,
            macro_def: GlobalDefId::NONE,
            vis: VisKind::Public,
            is_glob: false,
            is_reexport: true,
            is_prelude: false,
            is_underscore: false,
            prim_ty_code: 0,
        },
    );

    let from_trait = GlobalDefId::new(core_k, 20);
    let from_fn = GlobalDefId::new(core_k, 21);
    db.add_module_def(core_prel, OwnedDef { id: from_trait, name: "From".into(), kind: ItemKind::Trait, vis: VisKind::Public, has_value_ctor: false });
    db.add_assoc_item(from_trait, OwnedDef { id: from_fn, name: "from".into(), kind: ItemKind::AssocFn, vis: VisKind::Public, has_value_ctor: false });

    let u32_imp = GlobalDefId::new(core_k, 30);
    let u32_max = GlobalDefId::new(core_k, 31);
    db.add_impl(IndexedImpl { id: u32_imp, file: FileId::NONE, target: ImplTargetKey::Prim(PrimTy::U32), trait_def: GlobalDefId::NONE, is_negative: false });
    db.add_assoc_item(u32_imp, OwnedDef { id: u32_max, name: "MAX".into(), kind: ItemKind::AssocConst, vis: VisKind::Public, has_value_ctor: false });

    TestEnv { db, core_k, app_k, opt_def, some_def, none_def, from_trait, from_fn, u32_max }
}

fn find_nth(src: &str, needle: &str, nth: usize) -> u32 {
    let mut off = 0usize;
    for i in 0..=nth {
        let pos = src[off..].find(needle).unwrap_or_else(|| panic!("missing occurrence #{nth} of `{needle}`"));
        if i == nth {
            return (off + pos) as u32;
        }
        off += pos + needle.len();
    }
    unreachable!()
}

fn target_at(res: &FileNameRes, src: &str, needle: &str, nth: usize) -> ResTarget {
    let off = find_nth(src, needle, nth);
    res.ref_at_offset(off).unwrap_or_else(|| panic!("no ResolvedRef at `{needle}` (#{nth}, byte {off})")).target
}

#[test]
fn resolves_locals_shadowing_let_chains_or_pat_and_isolation_barriers() {
    let env = setup_env();
    let src = r#"
pub fn helper() -> u32 { 7 }

pub fn demo(x: Option<u32>, fallback: u32) -> u32 {
    let x = x;
    let Some(base) = x else {
        return fallback;
    };
    let chained = if let Some(a) = Some(base) && let Some(b) = Some(a) {
        a + b
    } else {
        fallback
    };
    let matched = match (Some(chained), Some(base)) {
        (Some(v), _) | (_, Some(v)) if v > 0 => v,
        _ => 0,
    };
    let mut acc = 0u32;
    for acc in [matched] {
        let f = |delta: u32| acc + delta + helper();
        let _ = f(1);
    }
    fn isolated() -> u32 {
        helper()
    }
    let _ = const { helper() };
    acc + isolated()
}
"#;
    let mut sources = MemSourceDb::new();
    let fid = sources.add_file("src/lib.rs", src);
    let mt = ModuleTree::build(&env.db, &sources, env.app_k, fid);
    let dm = CrateDefMap::build(&env.db, &sources, &mt);
    let res = dm.resolve_file_paths(&env.db, &sources, &mt, fid);

    // `let x = x;` — RHS `x` resolves to parameter `x` (#0), NOT the newly bound `let x` (#1)!
    let param_x = match target_at(&res, src, "x", 2) {
        ResTarget::Local(id) => id,
        other => panic!("expected Local on RHS `x`, got {other:?}"),
    };
    assert_eq!(res.local_data(param_x).name_start, find_nth(src, "x: Option", 0));

    // `let Some(base) = x else { return fallback; }` — `x` resolves to shadowed `let x`, `fallback` resolves to param.
    let shadowed_x = match target_at(&res, src, "x else", 0) {
        ResTarget::Local(id) => id,
        other => panic!("expected Local on `x else`, got {other:?}"),
    };
    assert_ne!(shadowed_x, param_x);
    assert!(target_at(&res, src, "fallback;", 0).is_local());

    // `if let Some(a) = ... && let Some(b) = Some(a) { a + b }` — `a` in `Some(a)` and `a + b` share the same LocalBindId!
    let a_in_cond = target_at(&res, src, "a)", 1);
    let a_in_then = target_at(&res, src, "a + b", 0);
    assert!(a_in_cond.is_local());
    assert_eq!(a_in_cond, a_in_then);

    // `(Some(v), _) | (_, Some(v)) if v > 0 => v` — guard `v` and arm `v` both point to the primary first-branch `v`!
    let v_guard = match target_at(&res, src, "v > 0", 0) {
        ResTarget::Local(id) => id,
        other => panic!("expected Local on guard `v`, got {other:?}"),
    };
    assert_eq!(res.ref_at_offset(find_nth(src, "=> v", 0) + 3).unwrap().target, ResTarget::Local(v_guard));
    assert_eq!(res.local_data(v_guard).name_start, find_nth(src, "Some(v)", 0) + 5);

    // `for acc in [matched]`: outer `mut acc` survives after the `for` loop (`acc + isolated()`), distinct from loop `acc`!
    let loop_acc = match target_at(&res, src, "acc + delta", 0) {
        ResTarget::Local(id) => id,
        other => panic!("expected Local on `acc + delta`, got {other:?}"),
    };
    let tail_acc = match target_at(&res, src, "acc + isolated", 0) {
        ResTarget::Local(id) => id,
        other => panic!("expected Local on `acc + isolated`, got {other:?}"),
    };
    assert_ne!(loop_acc, tail_acc);
    assert!(res.local_data(tail_acc).is_mut);
    assert!(!res.local_data(loop_acc).is_mut);

    // Inner `fn isolated()` resolves `block_ribs` to `isolated()` call in tail!
    match target_at(&res, src, "isolated()", 1) {
        ResTarget::Def { kind: ItemKind::Fn, .. } => {}
        other => panic!("expected block-local Fn for `isolated()`, got {other:?}"),
    }
}

#[test]
fn disambiguates_bare_ident_patterns_fields_and_block_inner_items() {
    let mut env = setup_env();
    let app_root = GlobalDefId::new(env.app_k, 0);
    let pt_def = GlobalDefId::new(env.app_k, 1);
    let pt_x = GlobalDefId::new(env.app_k, 2);
    let pt_y = GlobalDefId::new(env.app_k, 3);
    let limit_def = GlobalDefId::new(env.app_k, 4);
    let unit_tag = GlobalDefId::new(env.app_k, 5);
    env.db.set_crate_root_def(env.app_k, app_root);

    let src = r#"
pub struct Point { pub x: u32, pub y: u32 }
pub struct UnitTag;
pub const LIMIT: u32 = 10;
pub fn shadow_fn() -> u32 { 1 }

pub fn classify(opt: Option<Point>, shadow_fn: u32) -> u32 {
    let p = Point { x: shadow_fn, y: LIMIT };
    match (opt, UnitTag) {
        (None, UnitTag) => 0,
        (Some(Point { x: 0..=LIMIT, y }), _) => {
            enum LocalMode { Fast, Slow }
            impl LocalMode { fn bonus() -> u32 { 5 } }
            use LocalMode::*;
            let m = Fast;
            let _ = m;
            y + LocalMode::bonus() + p.x
        }
        (Some(Point { x, y: other_y }), _) => x + other_y,
    }
}
"#;
    let mut sources = MemSourceDb::new();
    let fid = sources.add_file("src/lib.rs", src);
    env.db.add_module_def_in_file(app_root, fid, OwnedDef { id: pt_def, name: "Point".into(), kind: ItemKind::Struct, vis: VisKind::Public, has_value_ctor: false });
    env.db.add_assoc_item(pt_def, OwnedDef { id: pt_x, name: "x".into(), kind: ItemKind::Field, vis: VisKind::Public, has_value_ctor: false });
    env.db.add_assoc_item(pt_def, OwnedDef { id: pt_y, name: "y".into(), kind: ItemKind::Field, vis: VisKind::Public, has_value_ctor: false });
    env.db.add_module_def_in_file(app_root, fid, OwnedDef { id: unit_tag, name: "UnitTag".into(), kind: ItemKind::Struct, vis: VisKind::Public, has_value_ctor: true });
    env.db.add_module_def_in_file(app_root, fid, OwnedDef { id: limit_def, name: "LIMIT".into(), kind: ItemKind::Const, vis: VisKind::Public, has_value_ctor: false });

    let mt = ModuleTree::build(&env.db, &sources, env.app_k, fid);
    let dm = CrateDefMap::build(&env.db, &sources, &mt);
    let res = dm.resolve_file_paths(&env.db, &sources, &mt, fid);

    // Parameter `shadow_fn: u32` shadows module `pub fn shadow_fn()`, so `x: shadow_fn` resolves to `Local`!
    assert!(target_at(&res, src, "shadow_fn,", 0).is_local());
    // Non-shorthand field `x:` in `Point { x: shadow_fn, y: LIMIT }` resolves to `pt_x` (`ItemKind::Field`)!
    assert_eq!(target_at(&res, src, "x: shadow_fn", 0).effective_def_id(), Some(pt_x));
    // Bare `None` and `UnitTag` in `(None, UnitTag) => 0` resolve to `none_def` and `unit_tag` (NOT locals!).
    assert_eq!(target_at(&res, src, "None", 0).effective_def_id(), Some(env.none_def));
    assert_eq!(target_at(&res, src, "UnitTag) =>", 0).effective_def_id(), Some(unit_tag));
    // Range end `0..=LIMIT` resolves `LIMIT` to `limit_def`, and non-shorthand `x: 0..=LIMIT` resolves `x` to `pt_x`!
    assert_eq!(target_at(&res, src, "x: 0..=LIMIT", 0).effective_def_id(), Some(pt_x));
    assert_eq!(res.ref_at_offset(find_nth(src, "0..=LIMIT", 0) + 4).unwrap().target.effective_def_id(), Some(limit_def));
    // Shorthand `y` in `Point { x: 0..=LIMIT, y }` binds a local variable `y`, used in `y + LocalMode::bonus()`!
    assert!(target_at(&res, src, "y + LocalMode", 0).is_local());
    // Block-local `use LocalMode::*; let m = Fast;` + `LocalMode::bonus()` resolve to `Variant` and `AssocFn`!
    assert!(matches!(target_at(&res, src, "Fast;", 0), ResTarget::Def { kind: ItemKind::Variant, .. }));
    assert!(matches!(target_at(&res, src, "bonus()", 1), ResTarget::Def { kind: ItemKind::AssocFn, .. }));
}

#[test]
fn resolves_qpaths_self_generics_trait_bounds_type_aliases_and_adt_fields() {
    let mut env = setup_env();
    let app_root = GlobalDefId::new(env.app_k, 0);
    let make_tr = GlobalDefId::new(env.app_k, 10);
    let make_out = GlobalDefId::new(env.app_k, 11);
    let make_fn = GlobalDefId::new(env.app_k, 12);
    let box_st = GlobalDefId::new(env.app_k, 20);
    let inh_imp = GlobalDefId::new(env.app_k, 21);
    let new_fn = GlobalDefId::new(env.app_k, 22);
    let make_imp = GlobalDefId::new(env.app_k, 23);
    let make_spec_fn = GlobalDefId::new(env.app_k, 24);
    let from_imp = GlobalDefId::new(env.app_k, 25);
    let from_spec_fn = GlobalDefId::new(env.app_k, 26);
    let alias_def = GlobalDefId::new(env.app_k, 27);
    env.db.set_crate_root_def(env.app_k, app_root);

    let src = r#"
pub trait Make {
    type Out;
    fn make() -> Self::Out;
}

pub struct Widget<T, const N: usize> {
    pub items: [T; N],
    pub opt: Option<T>,
}

pub type WidgetAlias = Widget<u32, 4>;

impl<T: Default, const N: usize> Widget<T, N> {
    pub fn new() -> Self {
        todo!()
    }
}

impl Make for Widget<u32, 4> {
    type Out = u32;
    fn make() -> Self::Out {
        u32::MAX
    }
}

impl From<u32> for Widget<u32, 4> {
    fn from(_v: u32) -> Self {
        Self::new()
    }
}

pub fn use_all<M: Make>() -> (<Widget<u32, 4> as Make>::Out, M::Out) {
    let _a = WidgetAlias::new();
    let _b = <Widget<u32, 4> as Make>::make();
    let _c = Widget::<u32, 4>::from(1);
    let _d: &dyn Make<Out = u32>;
    todo!()
}
"#;
    let mut sources = MemSourceDb::new();
    let fid = sources.add_file("src/lib.rs", src);
    env.db.add_module_def_in_file(app_root, fid, OwnedDef { id: make_tr, name: "Make".into(), kind: ItemKind::Trait, vis: VisKind::Public, has_value_ctor: false });
    env.db.add_assoc_item(make_tr, OwnedDef { id: make_out, name: "Out".into(), kind: ItemKind::AssocTy, vis: VisKind::Public, has_value_ctor: false });
    env.db.add_assoc_item(make_tr, OwnedDef { id: make_fn, name: "make".into(), kind: ItemKind::AssocFn, vis: VisKind::Public, has_value_ctor: false });

    env.db.add_module_def_in_file(app_root, fid, OwnedDef { id: box_st, name: "Widget".into(), kind: ItemKind::Struct, vis: VisKind::Public, has_value_ctor: false });
    env.db.add_module_def_in_file(app_root, fid, OwnedDef { id: alias_def, name: "WidgetAlias".into(), kind: ItemKind::TypeAlias, vis: VisKind::Public, has_value_ctor: false });
    env.db.set_type_alias_target(alias_def, ImplTargetKey::Adt(box_st));

    env.db.add_impl(IndexedImpl { id: inh_imp, file: fid, target: ImplTargetKey::Adt(box_st), trait_def: GlobalDefId::NONE, is_negative: false });
    env.db.add_assoc_item(inh_imp, OwnedDef { id: new_fn, name: "new".into(), kind: ItemKind::AssocFn, vis: VisKind::Public, has_value_ctor: false });

    env.db.add_impl(IndexedImpl { id: make_imp, file: fid, target: ImplTargetKey::Adt(box_st), trait_def: make_tr, is_negative: false });
    env.db.add_assoc_item(make_imp, OwnedDef { id: make_spec_fn, name: "make".into(), kind: ItemKind::AssocFn, vis: VisKind::Public, has_value_ctor: false });

    env.db.add_impl(IndexedImpl { id: from_imp, file: fid, target: ImplTargetKey::Adt(box_st), trait_def: env.from_trait, is_negative: false });
    env.db.add_assoc_item(from_imp, OwnedDef { id: from_spec_fn, name: "from".into(), kind: ItemKind::AssocFn, vis: VisKind::Public, has_value_ctor: false });

    let mt = ModuleTree::build(&env.db, &sources, env.app_k, fid);
    let dm = CrateDefMap::build(&env.db, &sources, &mt);
    let res = dm.resolve_file_paths(&env.db, &sources, &mt, fid);

    // Inside `trait Make`: `Self::Out` resolves `Out` to `make_out`!
    assert_eq!(res.ref_at_offset(find_nth(src, "Self::Out;", 0) + 6).unwrap().target.effective_def_id(), Some(make_out));
    // Inside `struct Widget<T, const N: usize>`: `[T; N]` and `Option<T>` in `TokenTree` resolve `T`, `N`, and `Option`!
    match target_at(&res, src, "T; N", 0) {
        ResTarget::Generic(gid) => assert!(!res.generic_data(gid).is_const),
        other => panic!("expected TypeParam `T` in struct field `[T; N]`, got {other:?}"),
    }
    match res.ref_at_offset(find_nth(src, "T; N", 0) + 3).unwrap().target {
        ResTarget::Generic(gid) => assert!(res.generic_data(gid).is_const),
        other => panic!("expected ConstParam `N` in struct field `[T; N]`, got {other:?}"),
    }
    assert_eq!(target_at(&res, src, "Option<T>", 0).effective_def_id(), Some(env.opt_def));

    // `u32::MAX` resolves `u32` to `Prim(U32)` and `MAX` to `u32_max`!
    assert_eq!(target_at(&res, src, "u32::MAX", 0), ResTarget::Prim(PrimTy::U32));
    assert_eq!(res.ref_at_offset(find_nth(src, "u32::MAX", 0) + 5).unwrap().target.effective_def_id(), Some(env.u32_max));

    // `Self` and `Self::new()` inside `impl From<u32> for Widget` resolve to `box_st` and `new_fn`!
    assert_eq!(target_at(&res, src, "Self {", 1).effective_def_id(), Some(box_st));
    assert_eq!(res.ref_at_offset(find_nth(src, "Self::new()", 0) + 6).unwrap().target.effective_def_id(), Some(new_fn));

    // `M::Out` via `<M: Make>` bound resolves `Out` to `make_out`!
    assert_eq!(res.ref_at_offset(find_nth(src, "M::Out", 0) + 3).unwrap().target.effective_def_id(), Some(make_out));
    // `WidgetAlias::new()` resolves through `TypeAlias` to `new_fn`!
    assert_eq!(res.ref_at_offset(find_nth(src, "WidgetAlias::new", 0) + 13).unwrap().target.effective_def_id(), Some(new_fn));
    // `<Widget<u32, 4> as Make>::make()` resolves to specialized `make_spec_fn`!
    assert_eq!(res.ref_at_offset(find_nth(src, "::make()", 0) + 2).unwrap().target.effective_def_id(), Some(make_spec_fn));
    // `Widget::<u32, 4>::from(1)` resolves via prelude `From` trait to specialized `from_spec_fn`!
    assert_eq!(res.ref_at_offset(find_nth(src, "::from(1)", 0) + 2).unwrap().target.effective_def_id(), Some(from_spec_fn));
    // `&dyn Make<Out = u32>` (#1; #0 is `type Out = u32;`) resolves associated type constraint `Out` to `make_out`!
    assert_eq!(target_at(&res, src, "Out = u32", 1).effective_def_id(), Some(make_out));
    let _ = (env.core_k, env.some_def, env.from_fn);
}

#[test]
fn resolves_use_trees_and_dirty_snapshot_override_paths() {
    let env = setup_env();
    let clean_src =
        "pub mod sub { pub struct Counter; impl Counter { pub fn old_val() -> u32 { 1 } } }\npub use crate::sub::{self, Counter};\npub fn read() -> u32 { Counter::old_val() }\n";
    let mut sources = MemSourceDb::new();
    let fid = sources.add_file("src/lib.rs", clean_src);
    let snap = ridx_syntax::name_res::CrateSnapshot::build_clean(&env.db, &sources, env.app_k, fid);
    let mut out = FileNameRes::new();
    snap.resolve_file_paths_into(&env.db, fid, &ridx_syntax::parse(clean_src), &mut out);
    assert!(matches!(target_at(&out, clean_src, "old_val()", 1), ResTarget::Def { kind: ItemKind::Fn | ItemKind::AssocFn, .. }));
    assert!(matches!(target_at(&out, clean_src, "self, Counter", 0), ResTarget::Def { kind: ItemKind::Mod, .. }));

    let dirty_src =
        "pub mod sub { pub struct Counter; impl Counter { pub fn live_val() -> u32 { 2 } } }\npub use crate::sub::{self, Counter};\npub fn read() -> u32 { Counter::live_val() }\n";
    let mut overlay_sources = ridx_syntax::name_res::OverlaySourceDb::new(&sources, 1);
    overlay_sources.set_file_override(fid, dirty_src);
    let (mt_opt, dirty_dm) = snap.with_dirty_files(&env.db, &overlay_sources, &[fid], |_| None);
    let active_mt = mt_opt.as_ref().unwrap_or(&snap.mod_tree);
    let dirty_res = dirty_dm.resolve_file_paths(&env.db, &overlay_sources, active_mt, fid);
    let resolved_live = target_at(&dirty_res, dirty_src, "live_val()", 1);
    assert!(matches!(resolved_live, ResTarget::Def { kind: ItemKind::Fn | ItemKind::AssocFn, .. }));
    assert!(resolved_live.effective_def_id().unwrap().is_overlay());
}
