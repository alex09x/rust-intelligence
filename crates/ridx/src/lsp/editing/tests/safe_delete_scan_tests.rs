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

/// Review of 6914b9cf: a raw literal's CRLF is read as LF; an import inside the item deleted goes with it; a `cfg` on a
/// definition makes it one of several; an inline module is a word of the item's path.
#[test]
fn test_raw_crlf_own_imports_gated_definitions_and_inline_modules() {
    assert_eq!(string_value("r#\"body\r\n.inc\"#").as_deref(), Some("body\n.inc"));
    let text = "pub fn unused() {\n    #[cfg(feature = \"x\")]\n    use crate::unused as alias;\n}\n";
    let own = 0..text.len();
    assert!(!scan(text, &["unused"], &["unused"], true, Some(&own)).gated_mention, "its own import");
    assert!(scan(text, &["unused"], &["unused"], true, None).gated_mention);
    for indexed in [true, false] {
        let s = scan("#[cfg(feature = \"a\")]\nmacro_rules! gen {\n    ($f:ident) => {};\n}\npub fn g() {}\n", &["unused"], &["unused"], indexed, None);
        assert!(s.defines.iter().any(|d| d.0 == "gen" && d.2), "a cfg on the definition, indexed {indexed}");
    }
    let s = scan("mod m {\n    #[cfg(any())]\n    macro_rules! gen {\n        ($f:ident) => {};\n    }\n}\n", &["unused"], &["unused"], false, None);
    assert!(s.defines.iter().any(|d| d.0 == "gen" && d.2), "a cfg on what holds it");
    let doc = "pub mod b {\n    pub fn chosen() {}\n}\n";
    let words = crate::lsp::editing::safe_delete::module_words(std::path::Path::new("/w/src/lib.rs"), doc, doc.find("pub fn").unwrap());
    assert!(words.iter().any(|w| w == "b"), "{words:?}");
}
