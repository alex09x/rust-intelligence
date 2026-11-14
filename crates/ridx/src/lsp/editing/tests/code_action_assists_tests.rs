/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Unit and `rustc`-verified compile-safety tests for declaration code actions and deferred resolve.

use super::setup_test_editing;
use crate::lsp::editing::actions::{handle_code_action_resolve, handle_code_action_with_options};
use crate::overlay::OverlayManager;
use ridx_format::{DefKind, WorkspaceIndex};
use ridx_test_support::{apply_text_edits, FixtureIndexBuilder};
use serde_json::{json, Value};
use std::fs;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static ASSIST_TEST_ID: AtomicU64 = AtomicU64::new(0);

pub(crate) fn assert_rustc_compiles(code: &str, label: &str) {
    // A template whose trait the user names (`impl _ for Type {}`), as rust-analyzer's: it compiles once named.
    if label.starts_with("Generate trait impl for") {
        assert!(code.contains(" _ for "), "trait impl template expected:\n{code}");
        return;
    }
    let id = ASSIST_TEST_ID.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_assist_rustc_{}_{}", std::process::id(), id));
    let _ = fs::create_dir_all(&dir);
    let src_path = dir.join("check.rs");
    let out_path = dir.join("libcheck.rmeta");
    fs::write(&src_path, code).unwrap();
    let output = Command::new("rustc")
        .current_dir(&dir)
        .args(["--edition=2021", "--crate-type=lib", "--emit=metadata"])
        .arg(&src_path)
        .arg("-o")
        .arg(&out_path)
        .output()
        .expect("rustc must execute");
    let _ = fs::remove_dir_all(&dir);
    assert!(output.status.success(), "rustc failed on [{label}]:\n{}\n--- code ---\n{code}", String::from_utf8_lossy(&output.stderr));
}

pub(crate) fn query_deferred_and_resolve(uri: &str, line: u32, character: u32, ws: &WorkspaceIndex, ov: &OverlayManager) -> Vec<(String, Value)> {
    let req = json!({
        "params": {
            "textDocument": { "uri": uri },
            "range": {
                "start": { "line": line, "character": character },
                "end": { "line": line, "character": character }
            },
            "context": { "diagnostics": [] }
        }
    });
    let list = handle_code_action_with_options(&req, Some(ws), ov, true).unwrap();
    let arr = list.as_array().unwrap();
    let mut resolved_out = Vec::new();
    for item in arr {
        assert!(item.get("edit").is_none(), "Deferred codeAction must omit edit until codeAction/resolve");
        let res_req = json!({ "params": item });
        let resolved = handle_code_action_resolve(&res_req, Some(ws), ov).unwrap();
        let title = resolved["title"].as_str().unwrap().to_string();
        let edits = resolved["edit"]["changes"][uri].clone();
        assert!(edits.is_array(), "Resolved action '{title}' must populate WorkspaceEdit.changes");
        for ed in edits.as_array().unwrap() {
            let nt = ed["newText"].as_str().unwrap_or("");
            assert!(!nt.contains("#[allow("), "Generated edit for '{title}' must never inject #[allow(...)]: {nt}");
        }
        resolved_out.push((title, edits));
    }
    resolved_out
}

#[test]
fn test_doc_template_inner_attrs_and_trait_impl() {
    let (temp, ws, mut ov, _) = setup_test_editing();
    let path = temp.join("src/doc_case.rs");
    let uri = format!("file://{}", path.to_str().unwrap());
    let code = "#![allow(dead_code)]\n\n#[inline]\npub fn compute(x: usize) -> usize {\n    x + 1\n}\n\npub trait Runner {\n    fn run(&self);\n}\nimpl Runner for usize {\n    fn run(&self) {}\n}\n";
    ov.open_document(&uri, path.to_str().unwrap(), 1, code.to_string());

    let actions_fn = query_deferred_and_resolve(&uri, 3, 8, &ws, &ov);
    let titles_fn: Vec<&str> = actions_fn.iter().map(|(t, _)| t.as_str()).collect();
    assert!(titles_fn.contains(&"Generate a documentation template"));
    for (title, edits) in &actions_fn {
        let updated = apply_text_edits(code, edits.as_array().unwrap());
        assert_rustc_compiles(&updated, title);
    }

    let actions_trait_impl = query_deferred_and_resolve(&uri, 11, 8, &ws, &ov);
    let titles_ti: Vec<&str> = actions_trait_impl.iter().map(|(t, _)| t.as_str()).collect();
    // A trait impl's method is documented as rust-analyzer offers it, and the documented file still compiles.
    assert!(titles_ti.contains(&"Generate a documentation template"), "trait impl methods take a doc template");
    for (title, edits) in actions_trait_impl.iter().filter(|(t, _)| t == "Generate a documentation template") {
        let updated = apply_text_edits(code, edits.as_array().unwrap());
        assert_rustc_compiles(&updated, title);
    }
}

#[test]
fn test_fn_type_aliases_generics_self_supertrait_and_elision() {
    let (temp, ws, mut ov, _) = setup_test_editing();
    let path = temp.join("src/alias_case.rs");
    let uri = format!("file://{}", path.to_str().unwrap());
    let code = concat!(
        "pub struct Cell<T>(pub T);\n",
        "impl<T> Cell<T> {\n",
        "    pub fn get_or_try<F, E>(&self, f: F) -> Result<&T, E>\n",
        "    where\n",
        "        F: FnOnce() -> Result<T, E>,\n",
        "    {\n",
        "        let _ = f;\n",
        "        Ok(&self.0)\n",
        "    }\n",
        "}\n",
        "pub trait Base {\n",
        "    type Output;\n",
        "}\n",
        "pub trait Sub: Base {\n",
        "    fn transform(&self, tag: &str) -> Self::Output;\n",
        "}\n",
    );
    ov.open_document(&uri, path.to_str().unwrap(), 1, code.to_string());

    let inherent_actions = query_deferred_and_resolve(&uri, 2, 12, &ws, &ov);
    assert_eq!(inherent_actions.len(), 3);
    for (title, edits) in &inherent_actions {
        let updated = apply_text_edits(code, edits.as_array().unwrap());
        assert_rustc_compiles(&updated, title);
    }

    let supertrait_actions = query_deferred_and_resolve(&uri, 14, 9, &ws, &ov);
    let st_titles: Vec<&str> = supertrait_actions.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(
        st_titles,
        vec!["Generate a documentation template", "Generate a type alias for function with named params", "Generate a type alias for function with unnamed params",]
    );
    for (title, edits) in &supertrait_actions {
        let updated = apply_text_edits(code, edits.as_array().unwrap());
        assert_rustc_compiles(&updated, title);
    }
}

#[test]
fn test_implement_default_members_cross_file_and_unimported_guard() {
    let id = ASSIST_TEST_ID.fetch_add(1, Ordering::Relaxed);
    let temp = std::env::temp_dir().join(format!("ridx_default_mem_{}_{}", std::process::id(), id));
    let _ = fs::create_dir_all(temp.join("src"));
    let defs_file = temp.join("src/defs.rs");
    let impl_file = temp.join("src/imp.rs");

    let mut builder = FixtureIndexBuilder::new("def_crate", "0.1.0");
    let defs_raw = concat!(
        "fn private_helper() -> usize { 7 }\n",
        "pub trait «SafeSpec» {\n",
        "    fn id(&self) -> &str;\n",
        "    fn has_id(&self) -> bool {\n",
        "        !self.id().is_empty()\n",
        "    }\n",
        "}\n",
        "pub trait «LeakySpec» {\n",
        "    fn count(&self) -> usize {\n",
        "        private_helper()\n",
        "    }\n",
        "    fn label(&self) -> &str;\n",
        "}\n",
    );
    let (defs_clean, _) = builder.add_defs_with_ids(defs_raw, defs_file.to_str().unwrap(), "def_crate::defs", DefKind::Trait);
    fs::write(&defs_file, &defs_clean).unwrap();

    let impl_code = format!(
        concat!(
            "#[path = {:?}]\n",
            "pub mod defs;\n",
            "use defs::{{LeakySpec, SafeSpec}};\n",
            "pub struct Entry;\n",
            "impl SafeSpec for Entry {{\n",
            "    fn id(&self) -> &str {{ \"ok\" }}\n",
            "}}\n",
            "impl LeakySpec for Entry {{\n",
            "    fn label(&self) -> &str {{ \"no\" }}\n",
            "}}\n",
        ),
        defs_file.to_str().unwrap()
    );
    fs::write(&impl_file, &impl_code).unwrap();
    let ridx_path = temp.join("index.ridx");
    builder.write_to_file(&ridx_path);
    let mut ws = WorkspaceIndex::open(&ridx_path).unwrap();
    ws.set_workspace_root(Some(temp.clone()));

    let ov = OverlayManager::new();
    let uri = format!("file://{}", impl_file.to_str().unwrap());

    let safe_actions = query_deferred_and_resolve(&uri, 5, 8, &ws, &ov);
    let safe_titles: Vec<&str> = safe_actions.iter().map(|(t, _)| t.as_str()).collect();
    assert!(safe_titles.contains(&"Implement default members"), "In-scope default member should be offered");
    for (title, edits) in &safe_actions {
        let updated = apply_text_edits(&impl_code, edits.as_array().unwrap());
        assert_rustc_compiles(&updated, title);
    }

    let leaky_actions = query_deferred_and_resolve(&uri, 8, 8, &ws, &ov);
    let leaky_titles: Vec<&str> = leaky_actions.iter().map(|(t, _)| t.as_str()).collect();
    assert!(!leaky_titles.contains(&"Implement default members"), "Default member calling unimported private helper must be suppressed");
    let _ = fs::remove_dir_all(&temp);
}

#[test]
fn test_generate_impl_and_new_with_unsized_and_existing_new_guards() {
    let (temp, ws, mut ov, _) = setup_test_editing();
    let path = temp.join("src/adt_case.rs");
    let uri = format!("file://{}", path.to_str().unwrap());
    let code = concat!(
        "#[cfg(unix)]\n",
        "pub struct Packet<'a, T = u8> {\n",
        "    pub header: &'a str,\n",
        "    pub payload: T,\n",
        "}\n",
        "pub struct UnsizedTail<T: ?Sized> {\n",
        "    pub inner: T,\n",
        "}\n",
        "pub struct HasNew(pub usize);\n",
        "impl HasNew {\n",
        "    pub fn new() -> Self { Self(0) }\n",
        "}\n",
    );
    ov.open_document(&uri, path.to_str().unwrap(), 1, code.to_string());

    let pkt_actions = query_deferred_and_resolve(&uri, 1, 12, &ws, &ov);
    let pkt_titles: Vec<&str> = pkt_actions.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(pkt_titles, vec!["Generate impl for `Packet`", "Generate trait impl for `Packet`", "Generate `new`"]);
    for (title, edits) in &pkt_actions {
        let updated = apply_text_edits(code, edits.as_array().unwrap());
        assert_rustc_compiles(&updated, title);
    }

    let unsized_actions = query_deferred_and_resolve(&uri, 5, 12, &ws, &ov);
    let unsized_titles: Vec<&str> = unsized_actions.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(unsized_titles, vec!["Generate impl for `UnsizedTail`", "Generate trait impl for `UnsizedTail`"]);

    let has_new_actions = query_deferred_and_resolve(&uri, 8, 12, &ws, &ov);
    let has_new_titles: Vec<&str> = has_new_actions.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(has_new_titles, vec!["Generate impl for `HasNew`", "Generate trait impl for `HasNew`", "Generate single field `From`"]);
}

#[test]
fn test_already_sorted_imports_not_offered_in_workspace() {
    let (temp, ws, mut ov, _) = setup_test_editing();
    let path = temp.join("src/sorted.rs");
    let uri = format!("file://{}", path.to_str().unwrap());
    let code = "use std::collections::HashMap;\nuse std::fmt::Debug;\n\npub struct Clean;\n";
    ov.open_document(&uri, path.to_str().unwrap(), 1, code.to_string());

    let actions = query_deferred_and_resolve(&uri, 0, 5, &ws, &ov);
    assert!(actions.is_empty(), "Already-sorted imports must not offer no-op Organize Imports");
}

#[test]
fn test_fn_type_alias_lifetime_self_gat_and_unbracketed_nominal() {
    let (temp, ws, mut ov, _) = setup_test_editing();
    let path = temp.join("src/lt_cases.rs");
    let uri = format!("file://{}", path.to_str().unwrap());
    let code = concat!(
        "pub struct View<'a>(pub &'a [u8]);\n",
        "pub struct Cursor<'a>(pub &'a [u8]);\n",
        "impl<'a> View<'a> {\n",
        "    pub fn bytes(&self) -> &[u8] { self.0 }\n",
        "    #[allow(mismatched_lifetime_syntaxes)]\n",
        "    pub fn cursor(&self) -> Cursor { Cursor(self.0) }\n",
        "}\n",
        "pub trait Spec { type Item<'w, 's>; }\n",
        "pub struct Bag<'w, 's, T>(pub &'w mut &'s (), pub std::marker::PhantomData<T>);\n",
        "impl<T: Spec> Bag<'_, '_, Vec<T>> {\n",
        "    pub fn get_mut(&mut self, index: usize) -> T::Item<'_, '_> { let _ = index; loop {} }\n",
        "}\n",
        "pub trait CmdExt {\n",
        "    fn tag(&mut self, label: impl Into<std::borrow::Cow<'static, str>>) -> &mut Self;\n",
        "}\n",
        "impl CmdExt for View<'_> {\n",
        "    fn tag(&mut self, _label: impl Into<std::borrow::Cow<'static, str>>) -> &mut Self { self }\n",
        "}\n",
    );
    ov.open_document(&uri, path.to_str().unwrap(), 1, code.to_string());

    for (line0, col) in [(3u32, 12u32), (10, 12), (16, 8)] {
        let actions = query_deferred_and_resolve(&uri, line0, col, &ws, &ov);
        assert!(actions.iter().any(|(t, _)| t.contains("type alias")), "Expected fn type alias at line {line0}");
        for (title, edits) in &actions {
            let updated = apply_text_edits(code, edits.as_array().unwrap());
            assert_rustc_compiles(&updated, title);
        }
    }

    let hidden_lt_actions = query_deferred_and_resolve(&uri, 5, 12, &ws, &ov);
    assert!(!hidden_lt_actions.iter().any(|(t, _)| t.contains("type alias")), "Unbracketed lifetime nominal with multi-lifetime receiver must not emit broken E0106 alias");
}

#[test]
fn test_inline_child_module_scopes_default_members_accurately() {
    let (temp, ws, mut ov, _) = setup_test_editing();
    let path = temp.join("src/inline_mod.rs");
    let uri = format!("file://{}", path.to_str().unwrap());
    let code = concat!(
        "pub struct RootCtx;\n",
        "pub trait Handler {\n",
        "    fn run(&mut self);\n",
        "    fn on_ctx(&mut self, _ctx: &RootCtx) {}\n",
        "}\n",
        "pub mod missing_import {\n",
        "    use super::Handler;\n",
        "    pub struct Thin;\n",
        "    impl Handler for Thin {\n",
        "        fn run(&mut self) {}\n",
        "    }\n",
        "}\n",
        "pub mod with_import {\n",
        "    use super::{Handler, RootCtx};\n",
        "    pub struct Full(pub RootCtx);\n",
        "    impl Handler for Full {\n",
        "        fn run(&mut self) {}\n",
        "    }\n",
        "}\n",
    );
    ov.open_document(&uri, path.to_str().unwrap(), 1, code.to_string());

    let thin_actions = query_deferred_and_resolve(&uri, 9, 12, &ws, &ov);
    assert!(!thin_actions.iter().any(|(t, _)| t == "Implement default members"), "Inline child module missing RootCtx import must suppress Implement default members");

    let full_actions = query_deferred_and_resolve(&uri, 16, 12, &ws, &ov);
    assert!(full_actions.iter().any(|(t, _)| t == "Implement default members"), "Inline child module importing RootCtx must offer Implement default members");
    for (title, edits) in &full_actions {
        let updated = apply_text_edits(code, edits.as_array().unwrap());
        assert_rustc_compiles(&updated, title);
    }
}

#[test]
fn test_nested_impl_trait_and_default_member_e0403_e0034_compile() {
    let (temp, ws, mut ov, _) = setup_test_editing();
    let path = temp.join("src/nested_and_ufcs.rs");
    let uri = format!("file://{}", path.to_str().unwrap());
    let code = concat!(
        "pub struct Stream;\n",
        "impl Stream {\n",
        "    pub fn feed(&self, _pairs: impl IntoIterator<Item = (impl Into<u32>, impl AsRef<str>)>) -> usize {\n",
        "        0\n",
        "    }\n",
        "}\n",
        "pub trait RawReader {\n",
        "    fn decode(&self, buf: &[u8]) -> usize;\n",
        "}\n",
        "pub trait Codec {\n",
        "    fn decode(&self, buf: &[u8]) -> usize;\n",
        "    fn decode_twice(&self, buf: &[u8]) -> usize {\n",
        "        self.decode(buf) + Self::decode(self, buf)\n",
        "    }\n",
        "    fn convert<T: Clone, E: From<T>>(&self, item: T) -> E {\n",
        "        E::from(item)\n",
        "    }\n",
        "}\n",
        "pub struct Adapter<T, E>(pub core::marker::PhantomData<(T, E)>);\n",
        "impl<T, E> RawReader for Adapter<T, E> {\n",
        "    fn decode(&self, buf: &[u8]) -> usize { buf.len() }\n",
        "}\n",
        "impl<T, E> Codec for Adapter<T, E> {\n",
        "    fn decode(&self, buf: &[u8]) -> usize { buf.len() * 2 }\n",
        "}\n",
    );
    ov.open_document(&uri, path.to_str().unwrap(), 1, code.to_string());

    let fn_actions = query_deferred_and_resolve(&uri, 2, 12, &ws, &ov);
    assert!(fn_actions.iter().any(|(t, _)| t == "Generate a type alias for function with named params"));
    for (title, edits) in &fn_actions {
        let updated = apply_text_edits(code, edits.as_array().unwrap());
        assert_rustc_compiles(&updated, title);
    }

    let impl_actions = query_deferred_and_resolve(&uri, 23, 8, &ws, &ov);
    assert!(impl_actions.iter().any(|(t, _)| t == "Implement default members"));
    for (title, edits) in &impl_actions {
        let updated = apply_text_edits(code, edits.as_array().unwrap());
        assert_rustc_compiles(&updated, title);
    }
}
