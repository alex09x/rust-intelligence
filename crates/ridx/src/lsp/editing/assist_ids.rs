/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! The stable id of every code action, in `data.id`: rust-analyzer's assist id where the assist is the same, so a
//! tool asks for `extract_variable` by name, never by a position in the list (prod-code, m=20226). An id, once
//! published, keeps its meaning (docs/integration.md §6).

use super::action_assists as a;
use serde_json::Value;

/// The id of each action kind ridx resolves (`data.kind`).
const BY_KIND: &[(&str, &str)] = &[
    ("source.organizeImports", "organize_imports"),
    ("refactor.extract", "extract_variable"),
    ("quickfix.auto_import", "auto_import"),
    (a::KIND_IMPL_DEFAULT_MEMBERS, "add_impl_default_members"),
    (a::KIND_DOC_TEMPLATE, "generate_documentation_template"),
    (a::KIND_FN_ALIAS_NAMED, "generate_fn_type_alias"),
    (a::KIND_FN_ALIAS_UNNAMED, "generate_fn_type_alias"),
    (a::KIND_DEFAULT_FROM_NEW, "generate_default_from_new"),
    (a::KIND_GENERATE_NEW, "generate_new"),
    (a::KIND_GENERATE_IMPL, "generate_impl"),
    (a::KIND_GENERATE_TRAIT_IMPL, "generate_trait_impl"),
    (a::KIND_SINGLE_FIELD_FROM, "generate_single_field_struct_from"),
    (a::KIND_ENUM_DISCRIMINANTS, "add_explicit_enum_discriminant"),
    (a::KIND_CONV_NAMED_STRUCT, "convert_tuple_struct_to_named_struct"),
    (a::KIND_CONV_TUPLE_STRUCT, "convert_named_struct_to_tuple_struct"),
    (super::action_extract_lit::KIND_EXTRACT_LIT_VAR, "extract_variable"),
    (super::action_extract_lit::KIND_EXTRACT_LIT_CONST, "extract_constant"),
    (super::action_extract_lit::KIND_EXTRACT_LIT_STATIC, "extract_static"),
    (a::KIND_INLINE_ALL_CALLERS, "inline_into_callers"),
    (a::KIND_INLINE_TYPE_ALIAS, "inline_type_alias_uses"),
    (a::KIND_NESTED_FN_CLOSURE, "convert_nested_function_to_closure"),
    (super::action_raw_string::KIND_STR_TO_RAW, "make_raw_string"),
    (super::action_raw_string::KIND_STR_TO_REGULAR, "make_usual_string"),
    (super::action_raw_string::KIND_STR_ADD_HASH, "add_hash"),
    (super::action_raw_string::KIND_STR_REMOVE_HASH, "remove_hash"),
    (super::action_inline_macro::KIND_INLINE_MACRO, "inline_macro"),
];

/// The id of an action by its title, for the actions sent with their edit and no kind of their own.
const BY_TITLE: &[(&str, &str)] = &[
    ("Add explicit type", "add_explicit_type"),
    ("Replace qualified path with use", "replace_qualified_name_with_use"),
    ("Extract variable", "extract_variable"),
    ("Extract into variable", "extract_variable"),
    ("Organize Imports", "organize_imports"),
    ("Import `", "auto_import"),
    ("Prefix with underscore", "prefix_with_underscore"),
    ("Remove unnecessary `mut`", "remove_unnecessary_mut"),
    ("Inline macro", "inline_macro"),
    ("Inline variable", "inline_local_variable"),
    ("Promote local to constant", "promote_local_to_const"),
    ("Wrap return type in Option", "wrap_return_type_in_option"),
    ("Wrap return type in Result", "wrap_return_type_in_result"),
    ("Rewrite as raw string", "make_raw_string"),
    ("Rewrite as regular string", "make_usual_string"),
    ("Add #", "add_hash"),
    ("Remove #", "remove_hash"),
];

/// The stable id of `action`.
pub fn assist_id(action: &Value) -> Option<&'static str> {
    let kind = action["data"]["kind"].as_str();
    if let Some((_, id)) = kind.and_then(|k| BY_KIND.iter().find(|(kk, _)| *kk == k)) {
        return Some(id);
    }
    let title = action["title"].as_str()?;
    BY_TITLE.iter().find(|(t, _)| title.starts_with(t)).map(|(_, id)| *id)
}

/// Sets `data.id` on every action that has one: a `data` object is added to an action sent with its edit, which a
/// resolve returns as it is.
pub fn set_ids(actions: &mut [Value]) {
    for action in actions {
        let Some(id) = assist_id(action) else { continue };
        if !action["data"].is_object() {
            action["data"] = serde_json::json!({});
        }
        action["data"]["id"] = id.into();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_every_action_has_rust_analyzers_id() {
        assert_eq!(assist_id(&json!({ "title": "Add explicit type", "kind": "refactor.rewrite" })), Some("add_explicit_type"));
        assert_eq!(assist_id(&json!({ "title": "Extract variable", "data": { "kind": "refactor.extract" } })), Some("extract_variable"));
        assert_eq!(assist_id(&json!({ "title": "Generate impl for `S`", "data": { "kind": a::KIND_GENERATE_IMPL } })), Some("generate_impl"));
        assert_eq!(assist_id(&json!({ "title": "Import `a::B`" })), Some("auto_import"));
        assert_eq!(assist_id(&json!({ "title": "Something else" })), None);
        let mut actions = [json!({ "title": "Add explicit type", "edit": {} }), json!({ "title": "Extract variable", "data": { "kind": "refactor.extract", "uri": "u" } })];
        set_ids(&mut actions);
        assert_eq!(actions[0]["data"], json!({ "id": "add_explicit_type" }));
        assert_eq!(actions[1]["data"]["id"], "extract_variable");
        assert_eq!(actions[1]["data"]["uri"], "u", "the rest of the data stays");
    }
}
