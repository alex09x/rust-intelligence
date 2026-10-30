/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Unit tests for scope caching, trait supertraits, and associated type bounds parsing.

use super::*;
use crate::lsp::intelligence::self_completion::EnclosingScope;
use std::sync::Arc;

#[test]
fn test_scope_block_enclosing_lookup() {
    let src = r#"
impl MyStruct {
    pub fn do_something(&self) {
        unsafe {
            Self::
        }
    }
}
"#;
    let data = parse_file_scopes(Arc::from(src));
    let cursor = data.cursor_byte(5, "            Self::");
    let scope = data.find_enclosing_scope(cursor);
    assert_eq!(scope, Some(EnclosingScope::Impl { trait_name: None, type_name: "MyStruct".to_string() }));
}

#[test]
fn test_scope_block_generic_params_lookup() {
    let src = r#"
impl<A, B> MyStruct<A, B>
where
    A: System,
    B: System,
{
    fn combine<F: WorldEntityFetch>(&mut self) {
        let x = F::
    }
}
"#;
    let data = parse_file_scopes(Arc::from(src));
    let cursor = data.cursor_byte(8, "        let x = F::");
    let params = data.find_generic_params_in_scope(cursor);
    let names: Vec<&str> = params.iter().map(|p| p.name.as_str()).collect();
    assert!(names.contains(&"A"));
    assert!(names.contains(&"B"));
    assert!(names.contains(&"F"));

    let f = params.iter().find(|p| p.name == "F").unwrap();
    assert_eq!(f.bounds, vec!["WorldEntityFetch".to_string()]);
}

#[test]
fn test_parse_assoc_bounds_edge_cases() {
    let src1 = "type Item: 'a + EntityEquivalent + Clone;\n";
    assert_eq!(parse_assoc_bounds_from_source(src1, 1), vec!["EntityEquivalent".to_string(), "Clone".to_string()]);

    let src2 = "type Item: Clone where Self: 'a;\n";
    assert_eq!(parse_assoc_bounds_from_source(src2, 1), vec!["Clone".to_string()]);

    let src3 = "type Item = Vec<u8>;\n";
    assert!(parse_assoc_bounds_from_source(src3, 1).is_empty());

    let src4 = "type Assoc: 'static + Send + Sync;\n";
    assert_eq!(parse_assoc_bounds_from_source(src4, 1), vec!["Send".to_string(), "Sync".to_string()]);

    let src5 = r#"
    type Output:
        Debug
        + Display;
"#;
    assert_eq!(parse_assoc_bounds_from_source(src5, 2), vec!["Debug".to_string(), "Display".to_string()]);
}

#[test]
fn test_parse_trait_supertraits() {
    let src = "pub trait ContiguousQueryData: ArchetypeQueryData + 'static + IterQueryData {\n";
    assert_eq!(parse_trait_supertraits_from_source(src, 1), vec!["ArchetypeQueryData".to_string(), "IterQueryData".to_string()]);
}

#[test]
fn test_overlay_cache_key_change_after_64_bytes() {
    let prefix = "// ------------------------------------------------------------------------\n";
    assert!(prefix.len() > 64);
    let src1 = format!("{prefix}impl Foo {{\n    fn bar() {{\n        Self::\n    }}\n}}\n");
    let src2 = format!("{prefix}impl Baz {{\n    fn bar() {{\n        Self::\n    }}\n}}\n");
    assert_eq!(src1.len(), src2.len());

    let file = "memory_overlay_test.rs";
    let scope1 = get_or_parse_file_scopes(file, Some(&src1)).expect("should parse scope1");
    let cursor1 = scope1.cursor_byte(4, "        Self::");
    let enclosing1 = scope1.find_enclosing_scope(cursor1);
    assert_eq!(enclosing1, Some(EnclosingScope::Impl { trait_name: None, type_name: "Foo".to_string() }));

    let scope2 = get_or_parse_file_scopes(file, Some(&src2)).expect("should parse scope2");
    let cursor2 = scope2.cursor_byte(4, "        Self::");
    let enclosing2 = scope2.find_enclosing_scope(cursor2);
    assert_eq!(enclosing2, Some(EnclosingScope::Impl { trait_name: None, type_name: "Baz".to_string() }));
}

#[test]
fn test_get_or_resolve_deref_chain_caching() {
    let (_tmp, ws, _overlay, _conv) = crate::lsp::intelligence::tests::setup_test_intelligence();
    let res1 = get_or_resolve_deref_chain(&ws, "test_intel::User", 2);
    let res2 = get_or_resolve_deref_chain(&ws, "test_intel::User", 2);
    assert!(Arc::ptr_eq(&res1, &res2));
    assert!(res1.is_empty());
}
