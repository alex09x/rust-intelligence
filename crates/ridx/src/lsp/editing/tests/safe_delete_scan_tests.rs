/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! What the scan of one source tells `ridx/safeDelete`.

use super::*;

/// Review of d8dcd7b1: a continuation skips ASCII whitespace only, after CRLF as after LF.
#[test]
fn test_a_string_is_decoded_as_rust_decodes_it() {
    assert_eq!(string_value("\"a\\\n  \u{a0}b\"").as_deref(), Some("a\u{a0}b"));
    assert_eq!(string_value("\"a\\\r\n  b\"").as_deref(), Some("ab"));
    assert_eq!(string_value("\"body\\u{2e}inc\"").as_deref(), Some("body.inc"));
    assert_eq!(string_value("r#\"x\"#").as_deref(), Some("x"));
}

/// Review of d8dcd7b1: a trailing comma after an include's literal; an include a macro expands is from where the macro
/// is called; arguments naming the item; an import under `cfg` naming its module.
#[test]
fn test_includes_arguments_and_gated_imports() {
    let s = scan("pub fn c() -> u8 { include!(\"body.inc\",) }\n", &["unused"], &["unused"], false, None);
    assert_eq!((s.includes, s.unresolved_include), (vec!["body.inc".to_string()], false));
    let s = scan("macro_rules! load {\n    () => { include!(\"body.inc\") };\n}\n", &["unused"], &["unused"], false, None);
    assert!(s.unresolved_include);
    let s = scan("gen!(unused);\n", &["unused"], &["unused"], true, None);
    assert!(s.calls.iter().any(|c| c.0 == "gen" && c.2), "{:?}", s.calls);
    let s = scan("#[cfg(feature = \"x\")]\nuse b as pick;\npub fn caller() { pick::chosen() }\n", &["chosen"], &["chosen", "b"], true, None);
    assert!(s.gated_mention);
    let s = scan("#[cfg(test)]\nmod tests {\n    use super::*;\n}\n", &["chosen"], &["chosen", "b"], true, None);
    assert!(!s.gated_mention, "a test module's glob names neither");
}
