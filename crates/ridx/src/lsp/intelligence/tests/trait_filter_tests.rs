/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use crate::lsp::intelligence::owner_completion::trait_filter::*;
use ridx_format::{DefKind, IndexWriter, RawDefinition, RawReference, WorkspaceIndex};
use std::collections::HashSet;
use std::fs;

#[test]
fn test_trait_filter_parsing() {
    assert_eq!(
        extract_trait_from_def_path("bevy_ecs::world::World::entity"),
        None
    );
    assert_eq!(extract_trait_from_def_path("<[T]>::len"), None);
    assert_eq!(
        extract_trait_from_def_path("<compare::query_runner::SetMetrics as core::fmt::Debug>::fmt"),
        Some("core::fmt::Debug")
    );
    assert_eq!(
        extract_trait_from_def_path("<bevy_ecs::entity::Entity as core::convert::Into<u64>>::into"),
        Some("core::convert::Into<u64>")
    );
    assert_eq!(
        extract_trait_from_def_path("<alloc::vec::Vec<T> as core::clone::Clone>::clone"),
        Some("core::clone::Clone")
    );
    assert_eq!(
        extract_trait_from_def_path(
            "impls::core::option::_::<impl reflect::Reflect for core::option::Option<T>>::into_any"
        ),
        Some("reflect::Reflect")
    );
    assert_eq!(
        extract_trait_from_def_path("core::option::Option::<T>::is_some"),
        None
    );

    // Test extract_trait_and_self_from_def_path
    assert_eq!(
        extract_trait_and_self_from_def_path("<core::option::Option<T> as core::fmt::Debug>::fmt"),
        Some(("core::fmt::Debug", "core::option::Option<T>"))
    );
    assert_eq!(
        extract_trait_and_self_from_def_path(
            "impls::core::option::_::<impl reflect::Reflect for core::option::Option<T>>::into_any"
        ),
        Some(("reflect::Reflect", "core::option::Option<T>"))
    );
    assert_eq!(
        extract_trait_and_self_from_def_path("<T as core::convert::AsRef<[u8]>>::as_ref"),
        Some(("core::convert::AsRef<[u8]>", "T"))
    );
    assert_eq!(
        extract_trait_and_self_from_def_path("core::option::Option::<T>::is_some"),
        None
    );

    // Test extract_parent_type
    assert_eq!(
        extract_parent_type(
            "relationship::relationship_query::<impl system::query::Query<'w, 's, D, F>>"
        ),
        "system::query::Query<'w, 's, D, F>"
    );
    assert_eq!(
        extract_parent_type("<alloc::boxed::Box<dyn core::any::Any>>"),
        "alloc::boxed::Box<dyn core::any::Any>"
    );
    assert_eq!(extract_parent_type("<[T]>"), "[T]");
    assert_eq!(
        extract_parent_type("bevy_ecs::world::World"),
        "bevy_ecs::world::World"
    );

    assert_eq!(
        base_trait_path("core::convert::Into<u64>"),
        "core::convert::Into"
    );
    assert_eq!(base_trait_path("core::fmt::Debug"), "core::fmt::Debug");
    assert_eq!(
        base_trait_path("core::convert::AsRef<[u8]>"),
        "core::convert::AsRef"
    );

    assert_eq!(
        base_type_path("core::option::Option<T>"),
        "core::option::Option"
    );
    assert_eq!(
        base_type_path("addr2line::Context::<R>"),
        "addr2line::Context"
    );
    assert_eq!(
        base_type_path("<alloc::boxed::Box<dyn Any>>"),
        "alloc::boxed::Box"
    );

    assert!(type_matches_owner(
        "core::option::Option<T>",
        "core::option::Option"
    ));
    assert!(type_matches_owner("Option<T>", "core::option::Option"));
    assert!(type_matches_owner(
        "system::query::Query<'w, 's, D, F>",
        "bevy_ecs::system::query::Query"
    ));
    assert!(type_matches_owner(
        "<alloc::boxed::Box<dyn Any>>",
        "alloc::boxed::Box"
    ));
    assert!(type_matches_owner(
        "<alloc::boxed::Box<dyn Any>>",
        "Box"
    ));
    assert!(!type_matches_owner(
        "addr2line::Context::<R>",
        "core::option::Option"
    ));
    assert!(!type_matches_owner("scene::Scene", "core::option::Option"));

    assert_eq!(trait_short_name("core::convert::Into<u64>"), "Into");
    assert_eq!(trait_short_name("core::fmt::Debug"), "Debug");

    assert!(is_in_prelude("core::clone::Clone"));
    assert!(is_in_prelude("core::fmt::Debug"));
    assert!(is_in_prelude("core::fmt::Display"));
    assert!(is_in_prelude("core::ops::deref::Deref"));
    assert!(is_in_prelude("core::ops::deref::DerefMut"));
    assert!(!is_in_prelude("bevy_reflect::Reflect"));
}

#[test]
fn test_signature_receiver_detection() {
    // Lower-case receiver
    assert!(signature_takes_receiver("fn(&self)"));
    assert!(signature_takes_receiver("fn(&'a self, x: i32) -> bool"));
    assert!(signature_takes_receiver("fn(&mut self)"));
    assert!(signature_takes_receiver("fn(self)"));
    assert!(signature_takes_receiver("fn(mut self)"));

    // Capitalized Self receiver (sysroot compiler trait methods)
    assert!(signature_takes_receiver(
        "for<'a, 'b, 'c> fn(&'a Self, &'b mut std::fmt::Formatter<'c>) -> std::result::Result<(), std::fmt::Error>"
    ));
    assert!(signature_takes_receiver("fn(&'a Self) -> &'a Self::Target"));
    assert!(signature_takes_receiver("fn(&Self)"));
    assert!(signature_takes_receiver("fn(&mut Self)"));

    // Non-receiver methods (including by-value Self parameter)
    assert!(!signature_takes_receiver("fn(Self)"));
    assert!(!signature_takes_receiver("fn(Self, Self)"));
    assert!(!signature_takes_receiver("fn(ComponentId, &mut Registrator)"));
    assert!(!signature_takes_receiver("fn(self_param: i32)"));
    assert!(!signature_takes_receiver("fn() -> i32"));
}

#[test]
fn test_categorize_definition_rules() {
    let empty_traits = HashSet::new();
    let mut in_scope_traits = HashSet::new();
    in_scope_traits.insert("IntoBytes".to_string());

    // 1. Inherent method on Option
    let cat = categorize_definition(
        DefKind::AssocFn,
        "core::option::Option::<T>::is_some",
        "core::option::Option",
        "core::option::Option",
        &empty_traits,
    );
    assert_eq!(cat, DefScopeCategory::Inherent);

    // 2. Inherent method with module::<impl Type>::method (e.g. Query::iter_ancestors)
    let cat = categorize_definition(
        DefKind::AssocFn,
        "relationship::relationship_query::<impl system::query::Query<'w, 's, D, F>>::iter_ancestors",
        "bevy_ecs::system::query::Query",
        "bevy_ecs::system::query::Query",
        &empty_traits,
    );
    assert_eq!(cat, DefScopeCategory::Inherent);

    // 3. Inherent method on <Box<dyn Any>>::downcast
    let cat = categorize_definition(
        DefKind::AssocFn,
        "<alloc::boxed::Box<dyn core::any::Any>>::downcast",
        "alloc::boxed::Box",
        "Box",
        &empty_traits,
    );
    assert_eq!(cat, DefScopeCategory::Inherent);

    // 4. Mismatched self type on impl Clone: <scene::Scene as Clone>::clone indexed under Option
    let cat = categorize_definition(
        DefKind::AssocFn,
        "<scene::Scene as core::clone::Clone>::clone",
        "core::option::Option",
        "core::option::Option",
        &empty_traits,
    );
    assert_eq!(cat, DefScopeCategory::OutOfScopeTrait);

    // 5. Matching self type for prelude trait: <core::option::Option<T> as Clone>::clone
    let cat = categorize_definition(
        DefKind::AssocFn,
        "<core::option::Option<T> as core::clone::Clone>::clone",
        "core::option::Option",
        "core::option::Option",
        &empty_traits,
    );
    assert_eq!(cat, DefScopeCategory::PreludeTrait);

    // 6. Direct prelude trait method: core::fmt::Debug::fmt on Option
    let cat = categorize_definition(
        DefKind::AssocFn,
        "core::fmt::Debug::fmt",
        "core::fmt::Debug",
        "core::option::Option",
        &empty_traits,
    );
    assert_eq!(cat, DefScopeCategory::PreludeTrait);

    // 7. Direct prelude deref trait method: core::ops::deref::Deref::deref on RwLockReadGuard
    let cat = categorize_definition(
        DefKind::AssocFn,
        "core::ops::deref::Deref::deref",
        "core::ops::deref::Deref",
        "RwLockReadGuard",
        &empty_traits,
    );
    assert_eq!(cat, DefScopeCategory::PreludeTrait);

    // 8. Trait method when in scope vs out of scope
    let cat_out = categorize_definition(
        DefKind::AssocFn,
        "zerocopy::IntoBytes::as_bytes",
        "core::option::Option",
        "core::option::Option",
        &empty_traits,
    );
    assert_eq!(cat_out, DefScopeCategory::OutOfScopeTrait);

    let cat_in = categorize_definition(
        DefKind::AssocFn,
        "zerocopy::IntoBytes::as_bytes",
        "core::option::Option",
        "core::option::Option",
        &in_scope_traits,
    );
    assert_eq!(cat_in, DefScopeCategory::InScopeTrait);
}

#[test]
fn test_collect_file_referenced_traits_with_globs() {
    let temp_dir = std::env::temp_dir().join(format!("ridx_glob_test_{}", std::process::id()));
    let _ = fs::create_dir_all(&temp_dir);
    let ridx_file = temp_dir.join("test.ridx");

    let mut writer = IndexWriter::new("test_glob_crate", "0.1.0");

    // Add trait in prelude module
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Trait,
        visibility: 0,
        is_macro_expansion: false,
        file: "src/prelude.rs".to_string(),
        name: "MyPreludeTrait".to_string(),
        start_line: 5,
        start_col: 1,
        end_line: 5,
        end_col: 20,
        path: "test_glob_crate::prelude::MyPreludeTrait".to_string(),
        signature: None,
    });

    // Add struct in prelude module (must NOT be collected into file_traits)
    writer.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::Struct,
        visibility: 0,
        is_macro_expansion: false,
        file: "src/prelude.rs".to_string(),
        name: "MyPreludeStruct".to_string(),
        start_line: 10,
        start_col: 1,
        end_line: 10,
        end_col: 25,
        path: "test_glob_crate::prelude::MyPreludeStruct".to_string(),
        signature: None,
    });

    // Add glob reference `test_glob_crate::prelude::*` in `src/main.rs`
    writer.add_reference(RawReference {
        file: "src/main.rs".to_string(),
        start_line: 1,
        start_col: 5,
        end_line: 1,
        end_col: 30,
        target_def_id: None,
        target_path: "test_glob_crate::prelude::*".to_string(),
        target_file: None,
        target_unit: None,
        target_line: 0,
        target_col: 0,
        kind: DefKind::Use,
        is_write: false,
        is_macro_expansion: false,
    });

    let mut f = fs::File::create(&ridx_file).unwrap();
    writer.write_to(&mut f).unwrap();
    drop(f);

    let ws = WorkspaceIndex::open(&ridx_file).unwrap();
    let traits = collect_file_referenced_traits(&ws, "src/main.rs");

    assert!(traits.contains("MyPreludeTrait"), "Expected MyPreludeTrait from glob expansion");
    assert!(
        traits.contains("test_glob_crate::prelude::MyPreludeTrait"),
        "Expected full trait path from glob expansion"
    );
    assert!(
        !traits.contains("MyPreludeStruct"),
        "Struct in prelude module must NOT be collected as a trait"
    );
    assert!(
        !traits.contains("test_glob_crate::prelude::MyPreludeStruct"),
        "Full struct path must NOT be collected as a trait"
    );

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_primitive_type_matches_owner() {
    assert!(type_matches_owner("[T]", "slice"));
    assert!(type_matches_owner("&[entity::Entity]", "slice"));
    assert!(type_matches_owner("&'_ [Entity]", "slice"));
    assert!(type_matches_owner("<[core::mem::maybe_uninit::MaybeUninit<T>]>", "slice"));
    assert!(type_matches_owner("slice", "slice"));
    assert!(type_matches_owner("[T; 4]", "array"));
    assert!(type_matches_owner("&[u8; 32]", "array"));
    assert!(type_matches_owner("*const u8", "pointer"));
    assert!(type_matches_owner("*mut std::ffi::c_void", "pointer"));
    assert!(type_matches_owner("str", "str"));
    assert!(!type_matches_owner("[T]", "str"));
    assert!(!type_matches_owner("[T; 4]", "slice"));
    assert!(!type_matches_owner("[T]", "array"));
}

#[test]
fn test_is_non_deref_type() {
    assert!(is_non_deref_type("usize"));
    assert!(is_non_deref_type("u32"));
    assert!(is_non_deref_type("bool"));
    assert!(is_non_deref_type("core::option::Option<T>"));
    assert!(is_non_deref_type("core::result::Result<T, E>"));
    assert!(is_non_deref_type("[T]"));
    assert!(is_non_deref_type("&[Entity]"));
    assert!(is_non_deref_type("*const u8"));
    assert!(!is_non_deref_type("alloc::vec::Vec<T>"));
    assert!(!is_non_deref_type("alloc::boxed::Box<T>"));
    assert!(!is_non_deref_type("std::path::PathBuf"));
    assert!(!is_non_deref_type("bevy_ecs::world::EntityWorldMut"));
}

#[test]
fn test_short_name_collision_behavior() {
    // Documents intentional P3 heuristic: short names match across un-normalized crate boundaries
    assert!(type_matches_owner("foo::Error", "bar::Error"));
    assert!(type_matches_owner("foo::Error", "foo::Error"));
}

