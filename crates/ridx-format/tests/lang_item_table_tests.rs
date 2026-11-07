/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use std::io::Cursor;

use ridx_format::{DefKind, IndexReader, IndexWriter, LangItemKind, RawDefinition, RawLangItem, RawTyDef, WorkspaceIndex};

#[repr(C, align(8))]
struct AlignedBuf([u8; 64 * 1024]);

impl AlignedBuf {
    fn from_vec(v: &[u8]) -> (Box<Self>, usize) {
        let mut b = Box::new(Self([0u8; 64 * 1024]));
        b.0[..v.len()].copy_from_slice(v);
        (b, v.len())
    }
}

struct SliceRef<'a>(&'a [u8]);
impl AsRef<[u8]> for SliceRef<'_> {
    fn as_ref(&self) -> &[u8] {
        self.0
    }
}

#[test]
fn lang_item_table_roundtrips_sorted_and_deduplicated() {
    let mut w = IndexWriter::new("core", "1.98.1");
    let try_idx = w.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Trait,
        visibility: 0,
        is_macro_expansion: false,
        name: "Try".into(),
        path: "core::ops::try_trait::Try".into(),
        signature: None,
        file: "ops/try_trait.rs".into(),
        start_line: 10,
        start_col: 1,
        end_line: 20,
        end_col: 2,
    });
    let fn_call_idx = w.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::AssocFn,
        visibility: 0,
        is_macro_expansion: false,
        name: "call".into(),
        path: "core::ops::function::Fn::call".into(),
        signature: None,
        file: "ops/function.rs".into(),
        start_line: 30,
        start_col: 5,
        end_line: 30,
        end_col: 40,
    });

    let try_td = w.intern_ty_def(RawTyDef::Local { path: "core::ops::try_trait::Try", local_def_idx: try_idx });
    let fn_call_td = w.intern_ty_def(RawTyDef::Local { path: "core::ops::function::Fn::call", local_def_idx: fn_call_idx });
    let sized_td = w.intern_ty_def(RawTyDef::Local { path: "core::marker::Sized", local_def_idx: u32::MAX });

    // Insert out of order + duplicate to verify canonical sort & dedup by `LangItemKind`.
    w.add_lang_item(RawLangItem { kind: LangItemKind::FnCall, ty_def: fn_call_td, def_index: Some(fn_call_idx) });
    w.add_lang_item(RawLangItem { kind: LangItemKind::Sized, ty_def: sized_td, def_index: None });
    w.add_lang_item(RawLangItem { kind: LangItemKind::Try, ty_def: try_td, def_index: Some(try_idx) });
    w.add_lang_item(RawLangItem { kind: LangItemKind::Try, ty_def: try_td, def_index: Some(try_idx) });
    assert_eq!(w.lang_item_count(), 4);

    let mut raw = Vec::new();
    w.write_to(&mut Cursor::new(&mut raw)).unwrap();
    let (aligned, len) = AlignedBuf::from_vec(&raw);
    let reader = IndexReader::from_buffer(SliceRef(&aligned.0[..len])).unwrap();
    let arena = reader.ty_arena();

    let items: Vec<_> = reader.lang_items_iter().collect();
    assert_eq!(items.len(), 3);
    assert_eq!(items[0].kind, LangItemKind::Sized);
    assert_eq!(items[0].def_index, None);
    assert_eq!(items[1].kind, LangItemKind::Try);
    assert_eq!(items[1].def_index, Some(try_idx));
    assert_eq!(items[2].kind, LangItemKind::FnCall);
    assert_eq!(items[2].def_index, Some(fn_call_idx));

    let try_hit = reader.lang_item(LangItemKind::Try).expect("Try lang item");
    assert_eq!(arena.def(try_hit.ty_def).unwrap().path, "core::ops::try_trait::Try");
    assert_eq!(try_hit.def_index, Some(try_idx));

    let call_hit = reader.lang_item(LangItemKind::FnCall).expect("FnCall lang item");
    assert_eq!(arena.def(call_hit.ty_def).unwrap().path, "core::ops::function::Fn::call");
    assert_eq!(reader.lang_item(LangItemKind::Future), None);
    assert_eq!(LangItemKind::from_u16(9999), LangItemKind::Other);
}

#[test]
fn workspace_index_resolves_lang_items_across_core_and_alloc_shards() {
    let tmp = std::env::temp_dir().join(format!("ridx_lang_ws_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let sys_dir = tmp.join("sysroot").join("cas_test");
    let proj_dir = tmp.join("proj");
    std::fs::create_dir_all(&sys_dir).unwrap();
    std::fs::create_dir_all(&proj_dir).unwrap();

    let mut core_w = IndexWriter::new("core", "1.98.1");
    let deref_td = core_w.intern_ty_def(RawTyDef::Local { path: "core::ops::deref::Deref", local_def_idx: u32::MAX });
    core_w.add_lang_item(RawLangItem { kind: LangItemKind::Deref, ty_def: deref_td, def_index: None });
    let mut buf = Vec::new();
    core_w.write_to(&mut Cursor::new(&mut buf)).unwrap();
    std::fs::write(sys_dir.join("core.ridx"), &buf).unwrap();

    let mut alloc_w = IndexWriter::new("alloc", "1.98.1");
    let box_td = alloc_w.intern_ty_def(RawTyDef::Local { path: "alloc::boxed::Box", local_def_idx: u32::MAX });
    alloc_w.add_lang_item(RawLangItem { kind: LangItemKind::OwnedBox, ty_def: box_td, def_index: None });
    buf.clear();
    alloc_w.write_to(&mut Cursor::new(&mut buf)).unwrap();
    std::fs::write(sys_dir.join("alloc.ridx"), &buf).unwrap();

    let mut user_w = IndexWriter::new("app", "0.1.0");
    user_w.set_unit_key("app-0.1.0");
    buf.clear();
    user_w.write_to(&mut Cursor::new(&mut buf)).unwrap();
    std::fs::write(proj_dir.join("app.ridx"), &buf).unwrap();

    let mut ws = WorkspaceIndex::open(&proj_dir).unwrap();
    ws.attach_sysroot_shards_for_key_in(&tmp.join("sysroot"), "cas_test");

    let (core_ridx, deref_view) = ws.lang_item(LangItemKind::Deref).expect("Deref in core");
    assert_eq!(ws.readers()[core_ridx].crate_name(), "core");
    assert_eq!(ws.readers()[core_ridx].ty_arena().def(deref_view.ty_def).unwrap().path, "core::ops::deref::Deref");

    let (alloc_ridx, box_view) = ws.lang_item(LangItemKind::OwnedBox).expect("OwnedBox in alloc");
    assert_eq!(ws.readers()[alloc_ridx].crate_name(), "alloc");
    assert_eq!(ws.readers()[alloc_ridx].ty_arena().def(box_view.ty_def).unwrap().path, "alloc::boxed::Box");

    let _ = std::fs::remove_dir_all(&tmp);
}
