/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Regression tests for targeted single-function path resolution and local CST scanning:
//! 1. `body_walk::walk_fn_item` walks `WhereClause` even when targeting `RetType` / `ParamList`.
//! 2. `body_walk_expr::walk_block_expr` and `local_defs::scan_container` preserve block-scoped items
//!    and body macro expansions even when nested inside a function containing an inner `fn`.

use ridx_syntax::name_res::{CrateId, CrateSnapshot, Edition, FileNameRes, GlobalDefId, ItemDb, ItemKind, MemItemDb, MemSourceDb, ResTarget};
use ridx_syntax::parse;
use ridx_syntax::syntax_kind::SyntaxKind as K;
use ridx_syntax::ty_infer::{BodyInferCtx, EmptyTyDefDb, FnInferResult, InferTyId, InferTyKind, LocalCstDefs};

fn find_fn<'a>(st: &'a ridx_syntax::SyntaxTree<'a>, name: &str) -> ridx_syntax::NodeId {
    st.descendants(st.root()).find(|&n| st.kind(n) == K::Fn && st.child(n, K::Name).is_some_and(|nn| st.text(nn) == name)).expect("fn not found")
}

#[test]
fn targeted_ret_type_resolves_where_clause_assoc_type() {
    let src = r#"
        pub trait MyIter {
            type Output;
        }
        pub fn fetch<T>() -> T::Output
        where
            T: MyIter,
        {
            loop {}
        }
    "#;
    let mut item_db = MemItemDb::new();
    let krate = item_db.add_crate("app", Edition::Edition2021);
    item_db.set_crate_root_def(krate, GlobalDefId::new(krate, 0));
    let mut sources = MemSourceDb::new();
    let fid = sources.add_file("src/lib.rs", src);

    let snap = CrateSnapshot::build_clean(&item_db, &sources, krate, fid);
    let st = parse(src);
    let fn_node = find_fn(&st, "fetch");
    let ret_node = st.child(fn_node, K::RetType).expect("RetType node");

    // Resolving specifically against `RetType` (the pattern fixed by walking `WhereClause`
    // unconditionally in `walk_fn_item`) must still resolve `T::Output` from `where T: MyIter`.
    let mut ret_only_res = FileNameRes::new();
    snap.resolve_fn_paths_into(&item_db, fid, &st, ret_node, &mut ret_only_res);
    let output_off = src.find("T::Output").expect("T::Output") as u32 + 3;
    let ret_ref = ret_only_res.ref_at_offset(output_off).expect("T::Output ref when targeting RetType");
    assert!(matches!(ret_ref.target, ResTarget::Def { kind: ItemKind::AssocTy, .. }), "expected AssocTy, got {:?}", ret_ref.target);

    // Resolving against the enclosing `Fn` must match.
    let mut fn_res = FileNameRes::new();
    snap.resolve_fn_paths_into(&item_db, fid, &st, fn_node, &mut fn_res);
    let fn_ref = fn_res.ref_at_offset(output_off).expect("T::Output ref when targeting Fn");
    assert_eq!(fn_ref.target, ret_ref.target);
}

#[test]
fn targeted_inner_stmt_retains_block_scoped_item_and_outer_body_macro() {
    let src = r#"
        pub struct TopBox(pub u32);
        macro_rules! mk_box {
            ($e:expr) => { TopBox($e) };
        }
        pub fn outer() -> u32 {
            enum LocalField {
                Val,
            }
            impl LocalField {
                fn get(&self) -> u32 { 42 }
            }
            let y = LocalField::Val.get() + mk_box!(7u32).0;
            fn inner_unrelated() {}
            inner_unrelated();
            y
        }
    "#;
    let mut item_db = MemItemDb::new();
    let krate = item_db.add_crate("app", Edition::Edition2021);
    item_db.set_crate_root_def(CrateId(krate.0), GlobalDefId::new(krate, 0));
    let mut sources = MemSourceDb::new();
    let fid = sources.add_file("src/lib.rs", src);

    let snap = CrateSnapshot::build_clean(&item_db, &sources, krate, fid);
    let st = parse(src);
    let outer_fn = find_fn(&st, "outer");
    // Target the `let y = ...;` statement *inside* `outer` so `walk_block_expr` sees `partial = true`
    // and `scan_container`'s `K::Fn` arm sees `overlaps = true` while `target` is strictly inside `outer`.
    let y_stmt = st.descendants(outer_fn).find(|&n| st.kind(n) == K::LetStmt && st.text(n).contains("LocalField::Val")).expect("let y stmt");

    let mut file_res = FileNameRes::new();
    snap.resolve_fn_paths_into(&item_db, fid, &st, y_stmt, &mut file_res);
    let mut fn_res = FileNameRes::new();
    snap.resolve_fn_paths_into(&item_db, fid, &st, outer_fn, &mut fn_res);
    let mut local_defs = LocalCstDefs::new();
    local_defs.index_target_fn_with_item_db(&st, Some(&snap.clean_def_map), Some(&item_db as &dyn ItemDb), fid, y_stmt);

    let mut ctx = BodyInferCtx::new();
    let mut res = FnInferResult::new();
    ctx.infer_fn_into(&st, &fn_res, &EmptyTyDefDb, &local_defs, outer_fn, &mut res);

    // Verify `LocalField::Val` expression resolved even with partial block walk on `y_stmt`,
    // and `mk_box!(7u32)` was scanned/expanded inside `outer_fn` even though `target` was inner `y_stmt`.
    let val_off = src.find("LocalField::Val").expect("LocalField::Val") as u32 + "LocalField::".len() as u32;
    assert!(file_res.ref_at_offset(val_off).is_some(), "expected LocalField::Val ref with partial block walk");
    let mc = st.descendants(y_stmt).find(|&n| st.kind(n) == K::MacroCall).expect("mk_box MacroCall");
    let bx_ty = res.expr_tys.iter().rev().find(|&&(n, _)| n == mc).map(|&(_, t)| t).expect("mk_box expr type");
    assert!(matches!(ctx.table.arena.kind(bx_ty), InferTyKind::Adt { .. }), "expected TopBox ADT for `mk_box!` when targeting inner stmt, got {:?}", ctx.table.arena.kind(bx_ty));

    let mut full_defs = LocalCstDefs::new();
    full_defs.index_target_fn_with_item_db(&st, Some(&snap.clean_def_map), Some(&item_db as &dyn ItemDb), fid, outer_fn);
    let mut full_res = FnInferResult::new();
    ctx.infer_fn_into(&st, &fn_res, &EmptyTyDefDb, &full_defs, outer_fn, &mut full_res);
    let y_lid = fn_res.locals.iter().enumerate().find(|(_, l)| st.text(l.decl_node) == "y").map(|(i, _)| ridx_syntax::name_res::LocalBindId(i as u32)).expect("local y");
    assert_eq!(full_res.local_ty(y_lid), Some(InferTyId::U32));
}
