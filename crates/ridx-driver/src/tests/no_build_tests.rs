/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Tests of `no_build`: which units are stubbed and what a procedural macro crate's stub declares.

use super::*;

fn args(a: &[&str]) -> Vec<String> {
    a.iter().map(|s| s.to_string()).collect()
}

#[test]
fn test_build_scripts_and_proc_macro_crates_are_told_by_their_arguments() {
    assert_eq!(unit_of(&args(&["rustc", "--crate-name", "build_script_build", "build.rs"])), Unit::BuildScript);
    assert_eq!(unit_of(&args(&["rustc", "--crate-name=build_script_main", "src/build.rs"])), Unit::BuildScript);
    assert_eq!(unit_of(&args(&["rustc", "--crate-name", "serde_derive", "--crate-type", "proc-macro", "src/lib.rs"])), Unit::ProcMacro);
    assert_eq!(unit_of(&args(&["rustc", "--crate-name", "a", "--crate-type=lib,proc-macro", "src/lib.rs"])), Unit::ProcMacro);
    assert_eq!(unit_of(&args(&["rustc", "--crate-name", "a", "--crate-type", "lib", "src/lib.rs"])), Unit::Other);
}

#[test]
fn test_a_stub_declares_each_macro_and_runs_none_of_the_crate() {
    let src = r##"
//! #[proc_macro] fn in_doc() {}
use proc_macro::TokenStream;
/* #[proc_macro] fn in_block_comment /* nested */ () {} */
const S: &str = "#[proc_macro] fn in_string()";
const R: &str = r#"#[proc_macro] fn in_raw() "#;
const C: char = '[';
fn helper<'a>(x: &'a str) -> &'a str { x }

#[proc_macro_derive(Builder, attributes(builder, r#default))]
pub fn derive_builder(input: TokenStream) -> TokenStream {
    std::process::Command::new("touch").arg("/tmp/ran").status().unwrap();
    input
}

#[proc_macro_attribute]
#[doc(hidden)]
pub fn route(attr: TokenStream, item: TokenStream) -> TokenStream { item }

#[proc_macro]
pub fn r#sql(input: TokenStream) -> TokenStream { input }
"##;
    let stub = proc_macro_stub(src);
    assert_eq!(
        stub,
        "extern crate proc_macro;\n\
         #[proc_macro_derive(Builder, attributes(builder, r#default))]\n\
         pub fn derive_builder(_: proc_macro::TokenStream) -> proc_macro::TokenStream { proc_macro::TokenStream::new() }\n\
         #[proc_macro_attribute]\n\
         pub fn route(_: proc_macro::TokenStream, item: proc_macro::TokenStream) -> proc_macro::TokenStream { item }\n\
         #[proc_macro]\n\
         pub fn sql(_: proc_macro::TokenStream) -> proc_macro::TokenStream { proc_macro::TokenStream::new() }\n"
    );
    assert!(!stub.contains("Command"), "nothing of the crate's code");
}
