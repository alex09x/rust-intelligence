/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Which imports count as workspace symbols: re-exports, decided from the `use` statement's source.

use super::{declared_pub_super, path_without_generics, use_is_pub};
use crate::workspace::routing_tests::build_mock_workspace;
use crate::{DefKind, IndexWriter, RawDefinition};
use std::collections::HashMap;

#[test]
fn test_a_path_reads_without_the_generic_lists_of_its_segments() {
    assert_eq!(path_without_generics("mpsc::Receiver::<T>::recv_many"), "mpsc::Receiver::recv_many");
    assert_eq!(path_without_generics("queue::Queue::<T, N>::claim_bounded"), "queue::Queue::claim_bounded");
    assert_eq!(path_without_generics("a::Map<K, Vec<V>>::get"), "a::Map::get");
    assert_eq!(path_without_generics("plain::path"), "plain::path");
    // Review of #14: a qualified self keeps its type, and the `>` of a `->` closes nothing.
    assert_eq!(path_without_generics("<a::Foo<T> as core::fmt::Debug>::fmt"), "a::Foo::fmt");
    assert_eq!(path_without_generics("m::Handler::<fn() -> a::B>::call"), "m::Handler::call");
    assert_eq!(super::path_segments("m::Handler::<fn() -> a::B>::call"), ["m", "Handler", "<fn() -> a::B>", "call"]);
}

/// prod-code, m=23269: in a crate depending on rapidfire, `rapidfire::mpsc::bounded` and `mpsc::bounded` came back
/// empty while `bounded` found it, and `Receiver::recv_many` could not match `mpsc::Receiver::<T>::recv_many`.
#[test]
fn test_a_path_query_finds_a_dependencys_item_by_its_crate_and_type() {
    let dir = std::env::temp_dir().join(format!("ridx_wsym_paths_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let shard = |crate_name: &str, defs: &[(&str, &str, DefKind)]| {
        let mut w = IndexWriter::new(crate_name, "0.1.0");
        for (i, (name, path, kind)) in defs.iter().enumerate() {
            let line = i as u32 + 1;
            w.add_definition(RawDefinition {
                def_id: i as u32 + 1,
                kind: *kind,
                visibility: 0,
                is_macro_expansion: false,
                file: "src/lib.rs".into(),
                name: name.to_string(),
                start_line: line,
                start_col: 1,
                end_line: line,
                end_col: 2,
                path: path.to_string(),
                signature: None,
            });
        }
        let p = dir.join(format!("{crate_name}.ridx"));
        w.write_to(&mut std::fs::File::create(&p).unwrap()).unwrap();
        p
    };
    let app = shard("app", &[("main", "main", DefKind::Fn)]);
    let dep = shard("rapidfire", &[("bounded", "mpsc::bounded", DefKind::Fn), ("recv_many", "mpsc::Receiver::<T>::recv_many", DefKind::AssocFn)]);
    let mut ws = build_mock_workspace(vec![(&app, None, None, HashMap::new()), (&dep, None, None, HashMap::new())], dir.clone());
    ws.reader_is_dependency[1] = true;
    for q in ["rapidfire::mpsc::bounded", "mpsc::bounded"] {
        let found: Vec<&str> = ws.search_workspace_symbols(q, 128).iter().map(|s| s.path).collect();
        assert_eq!(found, ["mpsc::bounded"], "{q}");
    }
    for q in ["Receiver::recv_many", "rapidfire::mpsc::Receiver::<T>::recv_many", "mpsc::Receiver::<T>::recv_many", "Receiver::recv_many::<a::Key>"] {
        let found: Vec<&str> = ws.search_workspace_symbols(q, 128).iter().map(|s| s.name).collect();
        assert_eq!(found, ["recv_many"], "{q}");
    }
    assert!(ws.search_workspace_symbols("app::main", 128).iter().any(|s| s.name == "main"), "a local path still matches first");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_a_child_items_pub_super_is_read_from_its_declaration() {
    let src = "mod state {\n    pub(super) struct Version(usize);\n    pub(crate) struct Other;\n    struct Hidden;\n}\n";
    assert!(declared_pub_super(src, 2, 23), "pub(super) item");
    assert!(!declared_pub_super(src, 3, 23), "pub(crate) item keeps the private import implicit");
    assert!(!declared_pub_super(src, 4, 12), "private item");
}

#[test]
fn test_use_is_pub_reads_the_statement_visibility() {
    let src = "use std::fmt::Display;\npub use crate::a::B;\npub(crate) use self::c::D;\nfn f() {}\nuse x::{\n    y::{Z},\n    W,\n};\npub use q::{\n    R,\n};\n";
    assert!(!use_is_pub(src, 1, 15), "private import");
    assert!(use_is_pub(src, 2, 19), "pub re-export");
    assert!(use_is_pub(src, 3, 25), "pub(crate) re-export");
    assert!(!use_is_pub(src, 6, 9), "nested group of a private import");
    assert!(!use_is_pub(src, 7, 5), "after a nested group, still private");
    assert!(use_is_pub(src, 10, 5), "multi-line pub group");
}
