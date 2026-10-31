/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use crate::overlay::DocumentOverlay;

#[test]
fn test_overlay_query_edge_cases() {
    let code = "fn compute() {\n    let val = 42;\n    val + 1\n}\n";
    let overlay = DocumentOverlay::new("file:///test.rs", "/test.rs", 1, code.to_string());

    assert_eq!(overlay.get_ident_at(2, 9), Some("val".to_string()));
    assert_eq!(overlay.get_ident_at(99, 1), None);
    assert_eq!(overlay.get_ident_at(1, 1), Some("fn".to_string()));

    // Out of bounds definition
    assert!(overlay.find_definition_at(999, 1).is_none());

    // Local binding references for non-existent symbol
    assert!(overlay.find_local_binding_references(999, 1).is_empty());

    // Document symbols accessor
    assert!(!overlay.document_symbols().is_empty());

    // Token with path suffix abstains from overlay resolution
    let path_code = "fn run() { std::cmp::max(1, 2); }\n";
    let path_ov = DocumentOverlay::new("file:///p.rs", "/p.rs", 1, path_code.to_string());
    assert!(path_ov.find_definition_at(1, 13).is_none());

    // Shadowed local binding references (skips non-matching definitions)
    let shadow_code = "fn outer() {\n    let val = 1;\n    {\n        let val = 2;\n        val;\n    }\n    val;\n}\n";
    let shadow_ov = DocumentOverlay::new("file:///s.rs", "/s.rs", 1, shadow_code.to_string());
    let outer_refs = shadow_ov.find_local_binding_references(2, 9);
    assert!(!outer_refs.is_empty());
    let inner_refs = shadow_ov.find_local_binding_references(4, 13);
    assert!(!inner_refs.is_empty());
    assert_ne!(outer_refs, inner_refs);

    // Ident at exact end of line and past end of line (lines 173-174, 180, 183)
    let eol_code = "let  my_var";
    let eol_ov = DocumentOverlay::new("file:///e.rs", "/e.rs", 1, eol_code.to_string());
    assert_eq!(eol_ov.get_ident_at(1, 10), Some("my_var".to_string()));
    assert_eq!(eol_ov.get_ident_at(1, 12), Some("my_var".to_string()));
    assert_eq!(eol_ov.get_ident_at(1, 5), None); // space not preceded by ident

    // Line out of bounds (line 49)
    assert!(eol_ov.find_definition_at(999, 1).is_none());

    // Find local binding references with different symbol mismatch in overlay (lines 235-237)
    let diff_code = "fn alpha() {}\nfn beta() { let alpha = 1; }\n";
    let diff_ov = DocumentOverlay::new("file:///d.rs", "/d.rs", 1, diff_code.to_string());
    let refs_fn = diff_ov.find_local_binding_references(1, 4);
    assert!(refs_fn.is_empty());
    let refs = diff_ov.find_local_binding_references(2, 21);
    assert!(!refs.is_empty());

    let test_shadow = "let target = 1;\n{\n    let target = 2;\n    target;\n}\ntarget;\n";
    let ov_shadow = DocumentOverlay::new("file:///sh.rs", "/sh.rs", 1, test_shadow.to_string());
    let def1_refs = ov_shadow.find_local_binding_references(1, 5);
    assert_eq!(def1_refs.len(), 2);

    // Empty line in get_ident_at (line 145)
    let empty_ov = DocumentOverlay::new("file:///empty.rs", "/empty.rs", 1, "\n\n".to_string());
    assert_eq!(empty_ov.get_ident_at(1, 1), None);

    // Digit prefix in get_ident_at (line 171, 194)
    let num_ov = DocumentOverlay::new("file:///num.rs", "/num.rs", 1, "123_abc.456".to_string());
    assert_eq!(num_ov.get_ident_at(1, 2), None);
    assert_eq!(num_ov.get_ident_at(1, 8), None);
}

#[test]
fn test_local_binding_references_filter_comments_and_strings() {
    let code = "fn test() {\n    let val = 1;\n    // val in comment\n    /* val in block */\n    let s = \"val in string\";\n    val + 1\n}\n";
    let ov = DocumentOverlay::new("file:///test.rs", "/test.rs", 1, code.to_string());
    let refs = ov.find_local_binding_references(2, 9);
    // Only line 2 (definition) and line 6 (val + 1) should be found, not inside comments or strings
    assert_eq!(refs.len(), 2);
}

#[test]
fn test_overlay_definition_kinds() {
    let code = "const MY_CONST: i32 = 10;\nmod my_submod {\n    pub struct Item;\n}\nfn run() {\n    my_submod;\n}\n";
    let ov = DocumentOverlay::new("file:///kinds.rs", "/kinds.rs", 1, code.to_string());
    let def_const = ov.find_definition_at(1, 7);
    assert!(def_const.is_some());
    let def_mod = ov.find_definition_at(6, 6);
    assert!(def_mod.is_some());
    assert_eq!(def_mod.unwrap().name, "my_submod");

    // Local binding references finding matching definition (lines 257-259 in query.rs)
    let local_code = "fn calc() {\n    let val = 10;\n    let sum = val + val;\n}\n";
    let ov_local = DocumentOverlay::new("file:///local.rs", "/local.rs", 1, local_code.to_string());
    let refs = ov_local.find_local_binding_references(2, 9);
    assert_eq!(refs.len(), 3);
}

#[test]
fn test_find_global_symbol_definition_scope_and_items() {
    use crate::overlay::OverlayManager;

    let mut mgr = OverlayManager::new();

    let file_a = "/workspace/src/main.rs";
    let code_a = "fn main() {\n    let config = 42;\n    config.x;\n}\n";
    mgr.open_or_update("file:///workspace/src/main.rs", file_a, 1, code_a.to_string());

    let file_b = "/workspace/src/utils.rs";
    let code_b = "pub fn config() {}\npub fn helper() {}\n";
    mgr.open_or_update("file:///workspace/src/utils.rs", file_b, 1, code_b.to_string());

    // 1. Local variable `config` in main.rs must NOT jump to `fn config()` in utils.rs
    assert_eq!(mgr.find_global_symbol_definition(file_a, "config", None), None);

    // 2. Unqualified `helper` without module prefix must NOT jump across files
    assert_eq!(mgr.find_global_symbol_definition(file_a, "helper", None), None);

    // 3. Qualified `utils::helper` MUST resolve to utils.rs
    let def_helper = mgr.find_global_symbol_definition(file_a, "helper", Some("utils"));
    assert!(def_helper.is_some());
    let def = def_helper.unwrap();
    assert_eq!(def.file, file_b);
    assert_eq!(def.name, "helper");

    // 4. Same-file unqualified lookup in utils.rs should resolve
    let def_local = mgr.find_global_symbol_definition(file_b, "helper", None);
    assert!(def_local.is_some());
    assert_eq!(def_local.unwrap().file, file_b);
}

#[test]
fn test_find_definition_at_end_col_boundary() {
    let code = "fn test_sym() {}\n";
    let overlay = DocumentOverlay::new("file:///test.rs", "/test.rs", 1, code.to_string());
    let sym = overlay.symbols.iter().find(|s| s.name == "test_sym").expect("symbol test_sym");
    let line = sym.start_line;
    let end_col = sym.end_col;

    // At end_col (inclusive boundary): returns Some("test_sym")
    let at_end = overlay.find_definition_at(line, end_col);
    assert_eq!(at_end.map(|d| d.name), Some("test_sym".to_string()));

    // At end_col + 1 (outside boundary): returns None
    let past_end = overlay.find_definition_at(line, end_col + 1);
    assert_eq!(past_end.map(|d| d.name), None);
}

#[test]
fn test_find_definition_at_multiline_span() {
    use crate::overlay::OverlaySymbol;
    use ridx_format::DefKind;
    let code = "struct BigStruct {\n    field: i32,\n}\n";
    let mut overlay = DocumentOverlay::new("file:///test.rs", "/test.rs", 1, code.to_string());
    overlay.symbols.push(OverlaySymbol {
        name: "BigStruct".to_string(),
        kind: DefKind::Struct,
        signature: None,
        start_line: 1,
        start_col: 1,
        end_line: 3,
        end_col: 2,
        scope_id: 0,
        decl_start_byte: 0,
        decl_end_byte: 0,
        scope_start_byte: 0,
    });

    // Intermediate line 2: must match inside the multi-line definition
    let mid = overlay.find_definition_at(2, 5);
    assert_eq!(mid.map(|d| d.name), Some("BigStruct".to_string()));
}

#[test]
fn test_definition_fallback_independent_of_polluting_did_open_history() {
    use crate::lsp::navigation::handle_definition;
    use crate::lsp::protocol::PositionConverter;
    use crate::overlay::OverlayManager;
    use serde_json::json;

    let mut mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let arch_file = "/ws/crates/bevy_ecs/src/archetype.rs";
    let arch_uri = "file:///ws/crates/bevy_ecs/src/archetype.rs";
    let arch_src = "\
/// [module level documentation]: crate::archetype
pub struct Archetype {
    edges: Edges,
    entities: Vec<ArchetypeEntity>,
}

impl Archetype {
    pub fn entities(&self) -> &[ArchetypeEntity] {
        &self.entities
    }
}
";
    mgr.open_with_source_hash(arch_uri, arch_file, 1, arch_src.to_string(), Some(ridx_format::normalized_source_hash(arch_src.as_bytes())));
    let ov = mgr.get(arch_file).unwrap();
    // Unqualified `entities:` on line 4 col 13 must NOT pick up `crate` from line 1 doc comment
    assert_eq!(ov.get_path_prefix_at(4, 13), None);

    let req = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "textDocument/definition",
        "params": {
            "textDocument": { "uri": arch_uri },
            "position": { "line": 3, "character": 12 }
        }
    });
    let before = handle_definition(&req, None, &mgr, &mut conv);
    assert_eq!(before["uri"], arch_uri);
    assert_eq!(before["range"]["start"]["line"], 7);

    // Open a polluting sequence of other files under `/crates/` with a same-named `pub fn entities`
    for idx in 0..64 {
        let other_file = format!("/ws/crates/bevy_ecs/src/world/pollute_{idx}.rs");
        let other_uri = format!("file://{other_file}");
        let other_src = "impl UnsafeWorldCell {\n    pub fn entities(self) {}\n}\n";
        mgr.open_with_source_hash(&other_uri, &other_file, 1, other_src.to_string(), Some(ridx_format::normalized_source_hash(other_src.as_bytes())));
    }

    let after = handle_definition(&req, None, &mgr, &mut conv);
    assert_eq!(after, before);
}
