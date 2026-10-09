/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Feature-family LSP handlers re-exported for unified handler dispatch.

#[allow(unused_imports)]
pub use super::editing::*;
#[allow(unused_imports)]
pub use super::hierarchy::*;
#[allow(unused_imports)]
pub use super::intelligence::*;
#[allow(unused_imports)]
pub use super::lifecycle::*;
#[allow(unused_imports)]
pub use super::navigation::*;
#[allow(unused_imports)]
pub use super::presentation::*;
#[allow(unused_imports)]
pub use super::symbols::*;
#[allow(unused_imports)]
pub use super::workspace::*;

#[cfg(test)]
#[allow(hidden_glob_reexports)]
pub(crate) mod tests {
    use super::*;
    use crate::lsp::protocol::PositionConverter;
    use crate::overlay::OverlayManager;
    use serde_json::json;

    #[test]
    pub fn test_is_rust_keyword_and_ident() {
        assert!(is_rust_keyword("fn"));
        assert!(is_rust_keyword("struct"));
        assert!(is_rust_keyword("async"));
        assert!(!is_rust_keyword("my_var"));

        assert!(is_valid_rust_ident("my_var"));
        assert!(is_valid_rust_ident("MyStruct"));
        assert!(is_valid_rust_ident("_hidden"));
        assert!(!is_valid_rust_ident("fn"));
        assert!(!is_valid_rust_ident("123abc"));
        assert!(!is_valid_rust_ident("invalid-ident"));
    }

    #[test]
    pub fn test_handlers_comprehensive() {
        let mut overlay_mgr = OverlayManager::new();
        let file_path = "/sample/src/lib.rs";
        let lib_uri = "file:///sample/src/lib.rs";
        let code = r#"
pub struct MyStruct {
    pub field: i32,
}

pub trait MyTrait {
    fn trait_fn(&self);
}

impl MyTrait for MyStruct {
    fn trait_fn(&self) {}
}

pub fn helper(x: i32) -> i32 {
    let local = x + 1;
    local
}

pub fn caller() {
    let s = MyStruct { field: 42 };
    s.trait_fn();
    let res = helper(1);
    let _ = res;
}
"#;
        overlay_mgr.open_document(lib_uri, file_path, 1, code.to_string());
        let mut conv = PositionConverter::new();

        // 1. Definition in overlay
        let def_msg = json!({
            "params": {
                "textDocument": { "uri": lib_uri },
                "position": { "line": 15, "character": 9 }
            }
        });
        let def_res = handle_definition(&def_msg, None, &overlay_mgr, &mut conv);
        assert!(!def_res.is_null());

        // 2. References in overlay
        let refs_msg = json!({
            "params": {
                "textDocument": { "uri": lib_uri },
                "position": { "line": 15, "character": 9 },
                "context": { "includeDeclaration": true }
            }
        });
        let refs_res = handle_references(&refs_msg, None, &overlay_mgr, &mut conv);
        assert!(refs_res.as_array().unwrap().len() >= 2);

        // 3. Document symbols
        let doc_sym_msg = json!({
            "params": { "textDocument": { "uri": lib_uri } }
        });
        let doc_sym_res = handle_document_symbol(&doc_sym_msg, &overlay_mgr, &mut conv);
        assert!(doc_sym_res.as_array().unwrap().len() >= 3);

        // 4. Selection range
        let sel_msg = json!({
            "params": {
                "textDocument": { "uri": lib_uri },
                "positions": [{ "line": 15, "character": 9 }]
            }
        });
        let sel_res = handle_selection_range(&sel_msg, &overlay_mgr);
        assert!(!sel_res.as_array().unwrap().is_empty());

        // 5. Document links with Unicode support
        let link_code = "// check out https://example.com/docs for info";
        overlay_mgr.open_document(
            "file:///sample/src/links.rs",
            "/sample/src/links.rs",
            1,
            link_code.to_string(),
        );
        let link_msg = json!({
            "params": { "textDocument": { "uri": "file:///sample/src/links.rs" } }
        });
        let link_res = handle_document_link(&link_msg, &overlay_mgr);
        assert_eq!(link_res.as_array().unwrap().len(), 1);
        assert_eq!(link_res[0]["target"], "https://example.com/docs");

        // 6. Prepare rename and rename local variable
        let prep_rename_msg = json!({
            "params": {
                "textDocument": { "uri": lib_uri },
                "position": { "line": 15, "character": 9 }
            }
        });
        let prep_res = handle_prepare_rename(&prep_rename_msg, None, &overlay_mgr, &mut conv)
            .expect("prepare rename succeeds");
        assert!(!prep_res.is_null());
        assert_eq!(prep_res["placeholder"], "local");

        let rename_msg = json!({
            "params": {
                "textDocument": { "uri": lib_uri },
                "position": { "line": 15, "character": 9 },
                "newName": "new_local"
            }
        });
        let rename_res = handle_rename(&rename_msg, None, &overlay_mgr, &mut conv)
            .expect("rename succeeds");
        let changes = rename_res["changes"].as_object().unwrap();
        assert!(changes.contains_key(lib_uri));
        assert_eq!(changes[lib_uri].as_array().unwrap().len(), 2); // Declaration AND usage!
    }
}
