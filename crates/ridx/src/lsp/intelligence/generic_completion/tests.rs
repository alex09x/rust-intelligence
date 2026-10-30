/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use super::*;
use ridx_format::DefKind;

#[test]
fn test_parse_generic_params_inline_bounds() {
    let header = "pub fn get_mut<F: WorldEntityFetch>(&mut self, entities: F) -> Result<F::DeferredMut, Error>";
    let params = parse_generic_params_from_header(header);
    assert_eq!(params.len(), 1);
    assert_eq!(params[0].name, "F");
    assert_eq!(params[0].bounds, vec!["WorldEntityFetch".to_string()]);
}

#[test]
fn test_parse_generic_params_where_clause() {
    let header = r#"
impl<Marker, In, Out, F> System for FunctionSystem<Marker, In, Out, F>
where
    Marker: 'static,
    In: SystemInput + 'static,
    Out: 'static,
    F: SystemParamFunction<Marker, In: FromInput<In>, Out: IntoResult<Out>>,
"#;
    let params = parse_generic_params_from_header(header);
    let f_param = params.iter().find(|p| p.name == "F").unwrap();
    assert_eq!(f_param.bounds, vec!["SystemParamFunction".to_string()]);

    let in_param = params.iter().find(|p| p.name == "In").unwrap();
    assert_eq!(in_param.bounds, vec!["SystemInput".to_string()]);

    let marker_param = params.iter().find(|p| p.name == "Marker").unwrap();
    assert!(marker_param.bounds.is_empty());
}

#[test]
fn test_find_generic_params_nested_scopes() {
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
    let params = find_generic_params_in_scope(src, 8, "        let x = F::");
    let names: Vec<&str> = params.iter().map(|p| p.name.as_str()).collect();
    assert!(names.contains(&"A"));
    assert!(names.contains(&"B"));
    assert!(names.contains(&"F"));

    let f = params.iter().find(|p| p.name == "F").unwrap();
    assert_eq!(f.bounds, vec!["WorldEntityFetch".to_string()]);

    let a = params.iter().find(|p| p.name == "A").unwrap();
    assert_eq!(a.bounds, vec!["System".to_string()]);
}

#[test]
fn test_find_generic_params_in_signature() {
    let src = r#"
pub fn get_mut<F: WorldEntityFetch>(
    &mut self,
    entities: F,
) -> Result<F::DeferredMut<'_>, EntityMutableFetchError> {
"#;
    let params = find_generic_params_in_scope(src, 5, ") -> Result<F::");
    let f = params.iter().find(|p| p.name == "F").unwrap();
    assert_eq!(f.bounds, vec!["WorldEntityFetch".to_string()]);
}

#[test]
fn test_resolve_generic_param_owners_from_driver_data() {
    let temp_dir = std::env::temp_dir().join(format!("ridx_gc_driver_{}", std::process::id()));
    let var_dir = temp_dir.join("variants/dev");
    std::fs::create_dir_all(&var_dir).unwrap();

    let mut writer = ridx_format::IndexWriter::new("test_crate", "0.1.0");

    // 1. Trait MyTrait
    writer.add_definition(ridx_format::RawDefinition {
        def_id: 1,
        kind: DefKind::Trait,
        visibility: 0,
        is_macro_expansion: false,
        name: "MyTrait".to_string(),
        path: "test_crate::MyTrait".to_string(),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 8,
        end_col: 2,
    });

    // 2. Associated type Assoc inside MyTrait with bound BoundTrait
    writer.add_definition_with_owners(
        ridx_format::RawDefinition {
            def_id: 2,
            kind: DefKind::AssocTy,
            visibility: 0,
            is_macro_expansion: false,
            name: "Assoc".to_string(),
            path: "test_crate::MyTrait::Assoc".to_string(),
            signature: Some("test_crate::BoundTrait".to_string()),
            file: "src/lib.rs".to_string(),
            start_line: 2,
            start_col: 5,
            end_line: 2,
            end_col: 25,
        },
        vec!["test_crate::MyTrait".to_string(), "test_crate::BoundTrait".to_string()],
    );

    // 3. Trait BoundTrait
    writer.add_definition(ridx_format::RawDefinition {
        def_id: 3,
        kind: DefKind::Trait,
        visibility: 0,
        is_macro_expansion: false,
        name: "BoundTrait".to_string(),
        path: "test_crate::BoundTrait".to_string(),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: 10,
        start_col: 1,
        end_line: 12,
        end_col: 2,
    });

    // 4. Function my_fn spanning lines 20..40
    writer.add_definition(ridx_format::RawDefinition {
        def_id: 4,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "my_fn".to_string(),
        path: "test_crate::my_fn".to_string(),
        signature: Some("fn<F: MyTrait>()".to_string()),
        file: "src/lib.rs".to_string(),
        start_line: 20,
        start_col: 1,
        end_line: 40,
        end_col: 2,
    });

    // 5. TyParam F spanning lines 20..40 with bound MyTrait
    writer.add_definition_with_owners(
        ridx_format::RawDefinition {
            def_id: 5,
            kind: DefKind::TyParam,
            visibility: 0,
            is_macro_expansion: false,
            name: "F".to_string(),
            path: "test_crate::my_fn::F".to_string(),
            signature: Some("test_crate::MyTrait".to_string()),
            file: "src/lib.rs".to_string(),
            start_line: 20,
            start_col: 1,
            end_line: 40,
            end_col: 2,
        },
        vec!["test_crate::MyTrait".to_string()],
    );

    let idx_path = var_dir.join("test_crate.ridx");
    let f = std::fs::File::create(&idx_path).unwrap();
    writer.write_to(&mut std::io::BufWriter::new(f)).unwrap();

    let manifest = serde_json::json!({
        "version": "2.0",
        "format_version": 3,
        "extractor_version": ridx_format::EXTRACTOR_VERSION,
        "active_variant_id": "dev",
        "variant_dir": "variants/dev",
        "workspace_root": temp_dir.display().to_string(),
        "manifest_path": temp_dir.join("Cargo.toml").display().to_string(),
        "profile": "dev",
    });
    std::fs::write(temp_dir.join("ridx_manifest.json"), serde_json::to_string_pretty(&manifest).unwrap()).unwrap();

    let ws = WorkspaceIndex::open(&temp_dir).unwrap();

    // 1. Direct F:: query from driver data (overlay_text = None, is_dirty = false)
    let owners_f = resolve_generic_param_owners(&ws, "src/lib.rs", 25, "    F::", None, false, "F");
    assert!(owners_f.is_some(), "F:: should resolve from index");
    assert_eq!(owners_f.unwrap(), vec!["test_crate::MyTrait".to_string()]);

    // 2. Query with open clean document buffer (overlay_text present, is_dirty = false)
    let open_buffer = "pub trait MyTrait { ... }";
    let owners_open = resolve_generic_param_owners(&ws, "src/lib.rs", 25, "    F::", Some(open_buffer), false, "F");
    assert_eq!(owners_open, Some(vec!["test_crate::MyTrait".to_string()]), "Open clean buffer should use driver fast path");

    // 3. Chained F::Assoc:: query from driver data
    let owners_assoc = resolve_generic_param_owners(&ws, "src/lib.rs", 25, "    F::Assoc::", None, false, "F::Assoc");
    assert!(owners_assoc.is_some(), "F::Assoc:: should resolve from index");
    assert_eq!(owners_assoc.unwrap(), vec!["test_crate::BoundTrait".to_string()]);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_resolve_generic_param_owners_impl_and_method_generics() {
    let temp_dir = std::env::temp_dir().join(format!("ridx_gc_impl_meth_{}", std::process::id()));
    let var_dir = temp_dir.join("variants/dev");
    std::fs::create_dir_all(&var_dir).unwrap();

    let mut writer = ridx_format::IndexWriter::new("test_crate", "0.1.0");

    // Trait Tr
    writer.add_definition(ridx_format::RawDefinition {
        def_id: 1,
        kind: DefKind::Trait,
        visibility: 0,
        is_macro_expansion: false,
        name: "Tr".to_string(),
        path: "test_crate::Tr".to_string(),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 5,
        end_col: 2,
    });

    // Trait MethodTr
    writer.add_definition(ridx_format::RawDefinition {
        def_id: 2,
        kind: DefKind::Trait,
        visibility: 0,
        is_macro_expansion: false,
        name: "MethodTr".to_string(),
        path: "test_crate::MethodTr".to_string(),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: 6,
        start_col: 1,
        end_line: 8,
        end_col: 2,
    });

    // Struct S
    writer.add_definition(ridx_format::RawDefinition {
        def_id: 3,
        kind: DefKind::Struct,
        visibility: 0,
        is_macro_expansion: false,
        name: "S".to_string(),
        path: "test_crate::S".to_string(),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: 9,
        start_col: 1,
        end_line: 9,
        end_col: 18,
    });

    // TyParam T on `impl<T: Tr> S<T>`:
    // Ident span is line 10, col 6..7; enclosing impl scope recorded in owners as "scope:10:1:35:2"
    writer.add_definition_with_owners(
        ridx_format::RawDefinition {
            def_id: 4,
            kind: DefKind::TyParam,
            visibility: 0,
            is_macro_expansion: false,
            name: "T".to_string(),
            path: "<test_crate::S>::T".to_string(),
            signature: Some("test_crate::Tr".to_string()),
            file: "src/lib.rs".to_string(),
            start_line: 10,
            start_col: 6,
            end_line: 10,
            end_col: 7,
        },
        vec!["scope:10:1:35:2".to_string()],
    );

    // TyParam M on method `fn f<M: MethodTr>()`:
    // Ident span is line 15, col 10..11; enclosing method scope recorded as "scope:15:5:25:6"
    writer.add_definition_with_owners(
        ridx_format::RawDefinition {
            def_id: 5,
            kind: DefKind::TyParam,
            visibility: 0,
            is_macro_expansion: false,
            name: "M".to_string(),
            path: "<test_crate::S>::f::M".to_string(),
            signature: Some("test_crate::MethodTr".to_string()),
            file: "src/lib.rs".to_string(),
            start_line: 15,
            start_col: 10,
            end_line: 15,
            end_col: 11,
        },
        vec!["scope:15:5:25:6".to_string()],
    );

    let idx_path = var_dir.join("test_crate.ridx");
    let f = std::fs::File::create(&idx_path).unwrap();
    writer.write_to(&mut std::io::BufWriter::new(f)).unwrap();

    let manifest = serde_json::json!({
        "version": "2.0",
        "format_version": 3,
        "extractor_version": ridx_format::EXTRACTOR_VERSION,
        "active_variant_id": "dev",
        "variant_dir": "variants/dev",
        "workspace_root": temp_dir.display().to_string(),
        "manifest_path": temp_dir.join("Cargo.toml").display().to_string(),
        "profile": "dev",
    });
    std::fs::write(temp_dir.join("ridx_manifest.json"), serde_json::to_string_pretty(&manifest).unwrap()).unwrap();

    let ws = WorkspaceIndex::open(&temp_dir).unwrap();

    // Verification with open clean document buffer
    // To PROVE the answer comes from the driver path rather than the lexer,
    // we use a buffer containing unparseable syntax that would make the lexer fail:
    let corrupt_buffer = "/* non-lexical content that lexer cannot extract generics from */";

    // 1. Inside method `fn f` at line 18:
    // a) Query `T::` (from enclosing impl<T: Tr>)
    let res_t = resolve_generic_param_owners(&ws, "src/lib.rs", 18, "        T::", Some(corrupt_buffer), false, "T");
    assert_eq!(res_t, Some(vec!["test_crate::Tr".to_string()]), "impl generic T must resolve from driver path when document buffer is clean");

    // b) Query `M::` (from enclosing fn f<M: MethodTr>)
    let res_m = resolve_generic_param_owners(&ws, "src/lib.rs", 18, "        M::", Some(corrupt_buffer), false, "M");
    assert_eq!(res_m, Some(vec!["test_crate::MethodTr".to_string()]), "method generic M must resolve from driver path when document buffer is clean");

    // 2. Outside method `fn f`, but inside `impl` at line 30:
    // a) Query `T::` -> still in scope
    let res_t_impl = resolve_generic_param_owners(&ws, "src/lib.rs", 30, "    T::", Some(corrupt_buffer), false, "T");
    assert_eq!(res_t_impl, Some(vec!["test_crate::Tr".to_string()]), "impl generic T must remain in scope across the entire impl body");

    // b) Query `M::` -> method generic M is out of scope outside fn f
    let res_m_out = resolve_generic_param_owners(&ws, "src/lib.rs", 30, "    M::", Some(corrupt_buffer), false, "M");
    assert_eq!(res_m_out, None, "method generic M must NOT be in scope outside the method");

    // 3. Negative check: if buffer is dirty (is_dirty = true), driver path is bypassed and lexer fails on corrupt buffer
    let res_dirty = resolve_generic_param_owners(&ws, "src/lib.rs", 18, "        T::", Some(corrupt_buffer), true, "T");
    assert_eq!(res_dirty, None, "dirty buffer must bypass driver path and attempt lexer, returning None on non-lexical buffer");

    let _ = std::fs::remove_dir_all(&temp_dir);
}
