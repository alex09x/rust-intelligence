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

use ridx_format::audit::{audit_reader_items, def_kind_slot};
use ridx_format::{DefKind, IndexReader, IndexWriter, RawDefinition};

#[repr(C, align(8))]
struct AlignedBytes {
    _align: [u64; 0],
    buf: Vec<u8>,
}

impl AsRef<[u8]> for AlignedBytes {
    fn as_ref(&self) -> &[u8] {
        &self.buf
    }
}

fn to_aligned_reader(writer: &mut IndexWriter) -> IndexReader<AlignedBytes> {
    let mut cur = Cursor::new(Vec::new());
    writer.write_to(&mut cur).expect("serialize ridx");
    let raw = cur.into_inner();
    let mut words = vec![0u64; raw.len().div_ceil(8)];
    let dst = unsafe { std::slice::from_raw_parts_mut(words.as_mut_ptr() as *mut u8, raw.len()) };
    dst.copy_from_slice(&raw);
    let vec_u8 = unsafe {
        let ptr = words.as_mut_ptr() as *mut u8;
        let cap = words.capacity() * 8;
        std::mem::forget(words);
        Vec::from_raw_parts(ptr, raw.len(), cap)
    };
    IndexReader::from_buffer(AlignedBytes { _align: [], buf: vec_u8 }).expect("open aligned ridx")
}

#[allow(clippy::too_many_arguments)]
fn raw_def(id: u32, kind: DefKind, name: &str, path: &str, sig: Option<&str>) -> RawDefinition {
    RawDefinition {
        def_id: id,
        kind,
        visibility: 0,
        is_macro_expansion: false,
        name: name.to_string(),
        path: path.to_string(),
        signature: sig.map(str::to_string),
        file: "src/lib.rs".to_string(),
        start_line: id,
        start_col: 1,
        end_line: id,
        end_col: 10,
    }
}

#[test]
fn audit_items_classifies_sigs_fields_impls_generics_derefs_and_reexports() {
    let mut w = IndexWriter::new("demo", "0.1.0");

    // Struct + named field + tuple struct ctor (missing tuple field .0)
    w.add_definition(raw_def(1, DefKind::Struct, "Buf", "demo::Buf", None));
    w.add_definition_with_owners(raw_def(2, DefKind::Field, "len", "demo::Buf::len", Some("usize")), vec!["demo::Buf".to_string()]);
    w.add_definition(raw_def(3, DefKind::Struct, "Wrap", "demo::Wrap", None));
    w.add_definition(raw_def(4, DefKind::StructCtor, "Wrap", "demo::Wrap", None));

    // Enum + variant + enum struct-variant field (attached to Enum in crate_items, parent is Variant)
    w.add_definition(raw_def(5, DefKind::Enum, "Msg", "demo::Msg", None));
    w.add_definition_with_owners(raw_def(6, DefKind::Variant, "Data", "demo::Msg::Data", None), vec!["demo::Msg".to_string()]);
    w.add_definition_with_owners(raw_def(7, DefKind::Field, "code", "demo::Msg::Data::code", Some("u32")), vec!["demo::Msg".to_string()]);
    w.add_definition(raw_def(8, DefKind::VariantCtor, "Ping", "demo::Msg::Ping", None));

    // Trait + Supertrait + AssocFn + AssocTy (with string bounds) + AssocConst (no sig)
    w.add_definition(raw_def(9, DefKind::Trait, "Base", "demo::Base", None));
    w.add_definition(raw_def(10, DefKind::Trait, "Stream", "demo::Stream", None));
    w.add_definition_with_owners(raw_def(11, DefKind::AssocTy, "Item", "demo::Stream::Item", Some("demo::Base")), vec!["demo::Stream".to_string()]);
    w.add_definition_with_owners(raw_def(12, DefKind::AssocFn, "poll", "demo::Stream::poll", Some("fn(&mut self) -> Self::Item")), vec!["demo::Stream".to_string()]);
    w.add_definition_with_owners(raw_def(13, DefKind::AssocConst, "ID", "demo::Stream::ID", None), vec!["demo::Stream".to_string()]);

    // Impl block (sig=None) + inherent AssocFn + trait-impl AssocTy (sig=None)
    w.add_definition(raw_def(14, DefKind::Impl, "Buf", "demo::<impl demo::Buf>", None));
    w.add_definition_with_owners(raw_def(15, DefKind::AssocFn, "as_bytes", "demo::Buf::as_bytes", Some("fn(&self) -> &[u8]")), vec!["demo::Buf".to_string()]);
    w.add_definition_with_owners(raw_def(16, DefKind::AssocTy, "Item", "demo::<impl demo::Stream for demo::Buf>::Item", None), vec!["demo::Buf".to_string()]);

    // Free Fn + TyAlias + Const + Generics + Use + Reexport
    w.add_definition(raw_def(17, DefKind::Fn, "make", "demo::make", Some("unsafe fn(usize) -> demo::Buf")));
    w.add_definition(raw_def(18, DefKind::TyAlias, "Alias", "demo::Alias", None));
    w.add_definition(raw_def(19, DefKind::Const, "MAX", "demo::MAX", None));
    w.add_definition_with_owners(raw_def(20, DefKind::TyParam, "T", "demo::make::T", Some("demo::Base")), vec!["scope:src/lib.rs:17:1:17:10".to_string()]);
    w.add_definition(raw_def(21, DefKind::LifetimeParam, "'a", "demo::make::'a", None));
    w.add_definition(raw_def(22, DefKind::ConstParam, "N", "demo::make::N", None));
    w.add_definition(raw_def(23, DefKind::Use, "Buf", "demo::sub::Buf", Some("demo::Buf")));
    w.add_definition(raw_def(24, DefKind::Use, "*", "demo::sub::*", Some("demo")));
    w.add_reexport_with_owners(raw_def(25, DefKind::Struct, "BufRe", "demo::pub_api::BufRe", Some("demo::Buf")), vec!["demo::pub_api".to_string()]);

    // DerefEntry conflation: 1 actual Deref::Target (Buf -> slice), 1 trait impl (Buf -> Stream), 1 supertrait (Stream -> Base)
    w.add_deref("demo::Buf", "slice");
    w.add_deref("demo::Buf", "demo::Stream");
    w.add_deref("demo::Stream", "demo::Base");

    let reader = to_aligned_reader(&mut w);
    let rep = audit_reader_items(&reader);

    assert_eq!(rep.total_defs, 25);
    assert_eq!(rep.reexports.non_public_vis_defs, 0);

    // Fn & AssocFn
    assert_eq!(rep.fn_sigs.fn_total, 1);
    assert_eq!(rep.fn_sigs.fn_with_sig, 1);
    assert_eq!(rep.fn_sigs.sigs_with_unsafe_or_extern, 1);
    assert_eq!(rep.fn_sigs.assoc_fn_total, 2);
    assert_eq!(rep.fn_sigs.assoc_fn_with_self, 2);
    assert_eq!(rep.fn_sigs.sigs_with_where, 0);

    // Fields & ctors
    assert_eq!(rep.fields.named_fields, 2);
    assert_eq!(rep.fields.tuple_fields, 0);
    assert_eq!(rep.fields.fields_with_sig, 2);
    assert_eq!(rep.fields.owner_has_struct, 1);
    assert_eq!(rep.fields.owner_has_enum, 1);
    assert_eq!(rep.fields.owner_has_variant, 1);
    assert_eq!(rep.fields.struct_ctors, 1);
    assert_eq!(rep.fields.variant_ctors, 1);
    assert_eq!(rep.fields.ctors_with_sig, 0);

    // Impls, traits, assoc items
    assert_eq!(rep.impls_traits.impl_defs, 1);
    assert_eq!(rep.impls_traits.impl_with_sig, 0);
    assert_eq!(rep.impls_traits.assoc_fn_owned_by_trait, 1);
    assert_eq!(rep.impls_traits.assoc_fn_owned_by_type, 1);
    assert_eq!(rep.impls_traits.assoc_ty_owned_by_trait, 1);
    assert_eq!(rep.impls_traits.assoc_ty_owned_by_trait_with_sig, 1);
    assert_eq!(rep.impls_traits.assoc_ty_owned_by_type, 1);
    assert_eq!(rep.impls_traits.assoc_ty_owned_by_type_with_sig, 0);
    assert_eq!(rep.impls_traits.assoc_const_total, 1);
    assert_eq!(rep.impls_traits.assoc_const_with_sig, 0);
    assert_eq!(rep.impls_traits.ty_alias_with_sig, 0);
    assert_eq!(rep.impls_traits.const_or_static_with_sig, 0);

    // Generics
    assert_eq!(rep.generics.ty_params, 1);
    assert_eq!(rep.generics.ty_params_with_bounds, 1);
    assert_eq!(rep.generics.ty_params_scope_owned, 1);
    assert_eq!(rep.generics.lifetime_params, 1);
    assert_eq!(rep.generics.const_params, 1);
    assert_eq!(rep.generics.const_params_with_sig, 0);

    // Deref 3-way disambiguation
    assert_eq!(rep.derefs.total_entries, 3);
    assert_eq!(rep.derefs.actual_deref_to_type, 1);
    assert_eq!(rep.derefs.trait_impl_edges, 1);
    assert_eq!(rep.derefs.supertrait_edges, 1);
    assert_eq!(rep.derefs.unresolved_target_edges, 0);

    // Re-exports & Use
    assert_eq!(rep.reexports.use_defs, 2);
    assert_eq!(rep.reexports.use_with_sig, 2);
    assert_eq!(rep.reexports.use_glob, 1);
    assert_eq!(rep.reexports.flag_reexport_defs, 1);
    assert_eq!(rep.by_kind[def_kind_slot(DefKind::Struct)].reexport_flag, 1);
}
