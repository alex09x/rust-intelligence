/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Domain 3: `textDocument/didOpen` synchronization tests (part 2: definitions and symbols).

use ridx::lsp::dispatch::dispatch_lsp_message;
use ridx::lsp::lifecycle::LifecycleManager;
use ridx::lsp::protocol::uri_to_path;
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::OverlayManager;
use ridx_format::DefKind;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::io::Cursor;
use std::sync::{Arc, Mutex};

fn dispatch_did_open(
    msg: Value,
    lifecycle: &mut LifecycleManager,
    multi_root: &mut MultiRootWorkspace,
    overlay_mgr: &mut OverlayManager,
) -> (bool, Vec<u8>) {
    let mut out = Cursor::new(Vec::new());
    let cancelled = Arc::new(Mutex::new(HashSet::new()));
    let res = dispatch_lsp_message(
        msg,
        &mut out,
        lifecycle,
        multi_root,
        overlay_mgr,
        &cancelled,
        None,
    );
    (res.unwrap_or(false), out.into_inner())
}

#[test]
fn test_did_open_struct_definition_symbols() {
    let mut lifecycle = LifecycleManager::new();
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/struct_test.rs";
    let file = uri_to_path(uri);
    let code = "pub struct Point { x: i32, y: i32 }\n";

    let (ok, out) = dispatch_did_open(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": { "textDocument": { "uri": uri, "version": 1, "text": code } }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
    );

    assert_eq!(ok, true);
    assert_eq!(out.len(), 0);
    let overlay = overlay_mgr.get(&file).expect("overlay must exist");
    assert_eq!(overlay.version, 1);
    assert_eq!(overlay.text, code);
    assert_eq!(overlay.symbols.len(), 1);
    assert_eq!(overlay.symbols[0].name, "Point");
    assert_eq!(overlay.symbols[0].kind, DefKind::Struct);
}

#[test]
fn test_did_open_enum_definition_symbols() {
    let mut lifecycle = LifecycleManager::new();
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/enum_test.rs";
    let file = uri_to_path(uri);
    let code = "pub enum Color { Red, Green, Blue }\n";

    let (ok, _) = dispatch_did_open(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": { "textDocument": { "uri": uri, "version": 2, "text": code } }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
    );

    assert_eq!(ok, true);
    let overlay = overlay_mgr.get(&file).expect("overlay must exist");
    assert_eq!(overlay.symbols.len(), 1);
    assert_eq!(overlay.symbols[0].name, "Color");
    assert_eq!(overlay.symbols[0].kind, DefKind::Enum);
}

#[test]
fn test_did_open_trait_definition_symbols() {
    let mut lifecycle = LifecycleManager::new();
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/trait_test.rs";
    let file = uri_to_path(uri);
    let code = "pub trait Display { fn show(&self); }\n";

    let (ok, _) = dispatch_did_open(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": { "textDocument": { "uri": uri, "version": 1, "text": code } }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
    );

    assert_eq!(ok, true);
    let overlay = overlay_mgr.get(&file).expect("overlay must exist");
    assert_eq!(overlay.symbols.len(), 2);
    assert_eq!(overlay.symbols[0].name, "Display");
    assert_eq!(overlay.symbols[0].kind, DefKind::Trait);
    assert_eq!(overlay.symbols[1].name, "show");
    assert_eq!(overlay.symbols[1].kind, DefKind::Fn);
}

#[test]
fn test_did_open_type_alias_symbols() {
    let mut lifecycle = LifecycleManager::new();
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/alias_test.rs";
    let file = uri_to_path(uri);
    let code = "pub type Kilometers = u32;\n";

    let (ok, _) = dispatch_did_open(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": { "textDocument": { "uri": uri, "version": 1, "text": code } }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
    );

    assert_eq!(ok, true);
    let overlay = overlay_mgr.get(&file).expect("overlay must exist");
    assert_eq!(overlay.symbols.len(), 1);
    assert_eq!(overlay.symbols[0].name, "Kilometers");
    assert_eq!(overlay.symbols[0].kind, DefKind::TyAlias);
}

#[test]
fn test_did_open_const_symbols() {
    let mut lifecycle = LifecycleManager::new();
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/const_test.rs";
    let file = uri_to_path(uri);
    let code = "pub const BUFFER_SIZE: usize = 1024;\n";

    let (ok, _) = dispatch_did_open(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": { "textDocument": { "uri": uri, "version": 1, "text": code } }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
    );

    assert_eq!(ok, true);
    let overlay = overlay_mgr.get(&file).expect("overlay must exist");
    assert_eq!(overlay.symbols.len(), 1);
    assert_eq!(overlay.symbols[0].name, "BUFFER_SIZE");
    assert_eq!(overlay.symbols[0].kind, DefKind::Const);
}

#[test]
fn test_did_open_static_symbols() {
    let mut lifecycle = LifecycleManager::new();
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/static_test.rs";
    let file = uri_to_path(uri);
    let code = "pub static COUNTER: i32 = 0;\n";

    let (ok, _) = dispatch_did_open(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": { "textDocument": { "uri": uri, "version": 1, "text": code } }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
    );

    assert_eq!(ok, true);
    let overlay = overlay_mgr.get(&file).expect("overlay must exist");
    assert_eq!(overlay.symbols.len(), 1);
    assert_eq!(overlay.symbols[0].name, "COUNTER");
    assert_eq!(overlay.symbols[0].kind, DefKind::Static);
}

#[test]
fn test_did_open_mod_declaration_symbols() {
    let mut lifecycle = LifecycleManager::new();
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/mod_test.rs";
    let file = uri_to_path(uri);
    let code = "pub mod config;\n";

    let (ok, _) = dispatch_did_open(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": { "textDocument": { "uri": uri, "version": 1, "text": code } }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
    );

    assert_eq!(ok, true);
    let overlay = overlay_mgr.get(&file).expect("overlay must exist");
    assert_eq!(overlay.symbols.len(), 1);
    assert_eq!(overlay.symbols[0].name, "config");
    assert_eq!(overlay.symbols[0].kind, DefKind::Mod);
}

#[test]
fn test_did_open_fn_parameters_symbols() {
    let mut lifecycle = LifecycleManager::new();
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/params_test.rs";
    let file = uri_to_path(uri);
    let code = "pub fn process(alpha: i32, beta: &str) {}\n";

    let (ok, _) = dispatch_did_open(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": { "textDocument": { "uri": uri, "version": 1, "text": code } }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
    );

    assert_eq!(ok, true);
    let overlay = overlay_mgr.get(&file).expect("overlay must exist");
    assert_eq!(overlay.symbols.len(), 3);
    assert_eq!(overlay.symbols[0].name, "process");
    assert_eq!(overlay.symbols[0].kind, DefKind::Fn);
    assert_eq!(overlay.symbols[1].name, "alpha");
    assert_eq!(overlay.symbols[1].kind, DefKind::Local);
    assert_eq!(overlay.symbols[2].name, "beta");
    assert_eq!(overlay.symbols[2].kind, DefKind::Local);
}

#[test]
fn test_did_open_let_bindings_symbols() {
    let mut lifecycle = LifecycleManager::new();
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/let_test.rs";
    let file = uri_to_path(uri);
    let code = "pub fn compute() { let val = 42; }\n";

    let (ok, _) = dispatch_did_open(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": { "textDocument": { "uri": uri, "version": 1, "text": code } }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
    );

    assert_eq!(ok, true);
    let overlay = overlay_mgr.get(&file).expect("overlay must exist");
    assert_eq!(overlay.symbols.len(), 2);
    assert_eq!(overlay.symbols[0].name, "compute");
    assert_eq!(overlay.symbols[0].kind, DefKind::Fn);
    assert_eq!(overlay.symbols[1].name, "val");
    assert_eq!(overlay.symbols[1].kind, DefKind::Local);
}

#[test]
fn test_did_open_raw_identifier_fn_symbol() {
    let mut lifecycle = LifecycleManager::new();
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/raw_ident.rs";
    let file = uri_to_path(uri);
    let code = "pub fn standard_type() {}\n";

    let (ok, _) = dispatch_did_open(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": { "textDocument": { "uri": uri, "version": 1, "text": code } }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
    );

    assert_eq!(ok, true);
    let overlay = overlay_mgr.get(&file).expect("overlay must exist");
    assert_eq!(overlay.symbols.len(), 1);
    assert_eq!(overlay.symbols[0].name, "standard_type");
    assert_eq!(overlay.symbols[0].kind, DefKind::Fn);
}
