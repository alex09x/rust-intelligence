/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Inline into all callers with call arguments: each is bound by a typed `let`, and a reference argument chained
//! through another call is refused.

use crate::lsp::editing::actions::{handle_code_action_resolve, handle_code_action_with_options};
use crate::overlay::OverlayManager;
use ridx_format::{DefKind, WorkspaceIndex};
use ridx_test_support::fixture::FixtureIndexBuilder;
use serde_json::{json, Value};
use std::fs;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static ID: AtomicU64 = AtomicU64::new(0);

const TITLE: &str = "Inline into all callers";

/// Indexes `code` (definition `greet` marked, called once on line 7) and returns the file text with the inline
/// edit applied, or `None` when the assist is not offered.
fn inline_greet(code: &str) -> (String, Option<String>) {
    let temp_dir = std::env::temp_dir().join(format!("ridx_inline_args_{}_{}", std::process::id(), ID.fetch_add(1, Ordering::Relaxed)));
    let _ = fs::create_dir_all(&temp_dir);
    let lib_file = temp_dir.join("lib.rs");
    let lib_str = lib_file.to_str().unwrap().to_string();
    let mut builder = FixtureIndexBuilder::new("test_args", "0.1.0");
    let (cleaned, def_ids) = builder.add_defs_with_ids(code, &lib_str, "test_args", DefKind::Fn);
    fs::write(&lib_file, &cleaned).unwrap();
    builder.add_ref("test_args::greet", &lib_str, 3, 4, 3, 9, Some(def_ids[0]), Some(&lib_str), 3, 4, DefKind::Fn);
    builder.add_ref("test_args::greet", &lib_str, 7, 5, 7, 10, Some(def_ids[0]), Some(&lib_str), 3, 4, DefKind::Fn);
    let ridx_file = temp_dir.join("index.ridx");
    builder.write_to_file(&ridx_file);
    let mut ws = WorkspaceIndex::open(&ridx_file).unwrap();
    ws.set_workspace_root(Some(temp_dir.clone()));
    let mut ov = OverlayManager::new();
    let uri = format!("file://{lib_str}");
    ov.open_document(&uri, &lib_str, 1, cleaned.clone());

    let req = json!({ "params": {
        "textDocument": { "uri": uri },
        "range": { "start": { "line": 2, "character": 4 }, "end": { "line": 2, "character": 4 } },
        "context": { "diagnostics": [] }
    }});
    let list = handle_code_action_with_options(&req, Some(&ws), &ov, true).unwrap();
    let item = list.as_array().unwrap().iter().find(|it| it["title"] == TITLE).cloned();
    let updated = item.map(|item| {
        let resolved = handle_code_action_resolve(&json!({ "params": item }), Some(&ws), &ov).unwrap();
        let edits = resolved["edit"]["changes"][&uri].as_array().cloned().unwrap_or_default();
        apply(&cleaned, &edits)
    });
    let _ = fs::remove_dir_all(&temp_dir);
    (cleaned, updated)
}

fn apply(original: &str, edits: &[Value]) -> String {
    let starts: Vec<usize> = std::iter::once(0).chain(original.match_indices('\n').map(|(i, _)| i + 1)).collect();
    let at = |p: &Value| starts[p["line"].as_u64().unwrap() as usize] + p["character"].as_u64().unwrap() as usize;
    let mut spans: Vec<(usize, usize, &str)> = edits.iter().map(|e| (at(&e["range"]["start"]), at(&e["range"]["end"]), e["newText"].as_str().unwrap())).collect();
    spans.sort_by_key(|s| std::cmp::Reverse(s.0));
    let mut out = original.to_string();
    for (s, e, t) in spans {
        out.replace_range(s..e, t);
    }
    out
}

fn assert_compiles(source: &str) {
    let dir = std::env::temp_dir().join(format!("ridx_inline_args_c_{}_{}", std::process::id(), ID.fetch_add(1, Ordering::Relaxed)));
    let _ = fs::create_dir_all(&dir);
    let src = dir.join("lib.rs");
    fs::write(&src, source).unwrap();
    let out = Command::new("rustc").args(["--crate-type=lib", "--emit=metadata", "--edition=2021"]).arg(&src).arg("--out-dir").arg(&dir).output();
    let _ = fs::remove_dir_all(&dir);
    if let Ok(out) = out {
        assert!(out.status.success(), "rustc failed:\n{}\n\nSource:\n{source}", String::from_utf8_lossy(&out.stderr));
    }
}

#[test]
fn test_call_arguments_are_bound_with_their_declared_types() {
    let code = concat!(
        "fn make() -> String { String::from(\"a\") }\n",
        "\n",
        "fn «greet»(n: String, s: &str) -> usize {\n",
        "    n.len() + s.len()\n",
        "}\n",
        "pub fn caller(t: &String) -> usize {\n",
        "    greet(make(), t.as_str())\n",
        "}\n",
    );
    let (_, updated) = inline_greet(code);
    let updated = updated.expect("Inline into all callers offered for call arguments");
    assert!(updated.contains("let n: String = make();"), "{updated}");
    assert!(updated.contains("let s: &str = &t.as_str();"), "{updated}");
    assert!(!updated.contains("fn greet("), "{updated}");
    assert_compiles(&updated);
}

#[test]
fn test_reference_argument_through_a_temporary_chain_is_refused() {
    let code = concat!(
        "fn make() -> String { String::from(\"a\") }\n",
        "\n",
        "fn «greet»(n: String, s: &str) -> usize {\n",
        "    n.len() + s.len()\n",
        "}\n",
        "pub fn caller() -> usize {\n",
        "    greet(make(), make().as_str())\n",
        "}\n",
    );
    // Binding `make().as_str()` to `s` would drop the `String` at the `;` (E0716).
    assert_eq!(inline_greet(code).1, None);
}

#[test]
fn test_borrowed_result_is_inlined_only_over_places() {
    let place = concat!(
        "fn make() -> String { String::from(\"a\") }\n",
        "\n",
        "fn «greet»(s: &String) -> &str {\n",
        "    s.as_str()\n",
        "}\n",
        "pub fn caller(t: &String) -> usize {\n",
        "    greet(t).len()\n",
        "}\n",
    );
    let (_, updated) = inline_greet(place);
    let updated = updated.expect("a borrowed result over a place is inlined");
    assert!(!updated.contains("fn greet("), "{updated}");
    assert_compiles(&updated);

    // `&make()` is a temporary the inlined block would drop before `.len()` reads the borrow (E0716).
    let temporary = place.replace("greet(t).len()", "greet(&make()).len()");
    assert_eq!(inline_greet(&temporary).1, None);
}

#[test]
fn test_impl_trait_parameter_takes_only_a_variable_argument() {
    let code = concat!("pub struct W(u32);\n", "\n", "fn «greet»(n: impl Into<u64>) -> u64 {\n", "    n.into()\n", "}\n", "pub fn caller() -> u64 {\n", "    greet(1)\n", "}\n",);
    // `let n = 1;` leaves the integer type to infer and `n.into()` cannot pick it (E0283/E0689).
    assert_eq!(inline_greet(code).1, None);
}
