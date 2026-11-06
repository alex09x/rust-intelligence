/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use crate::lsp::editing::action_inline_callers_cross::is_safe_for_cross_file_inline;
use crate::lsp::editing::action_parse::mask_comments_and_strings;

#[test]
fn test_cross_file_path_qualified_call() {
    let body = "make::path_segment_crate()";
    let masked = mask_comments_and_strings(
        r#"
mod make {
    pub fn path_segment_crate() -> PathSegment { PathSegment }
}
"#,
    );
    assert!(is_safe_for_cross_file_inline(body, &masked));
}

#[test]
fn test_cross_file_match_with_public_enum() {
    let body = r#"
match id {
    ast::ExprOrPatId::ExprId(id) => this.expr_syntax(id),
    ast::ExprOrPatId::PatId(id) => this.pat_syntax(id),
}
"#;
    let doc = r#"
pub enum ExprOrPatId {
    ExprId(u32),
    PatId(u32),
}

impl ExpressionStoreSourceMap {
    pub fn expr_syntax(&self, id: u32) -> u32 { id }
    pub fn pat_syntax(&self, id: u32) -> u32 { id }
}
"#;
    let masked = mask_comments_and_strings(doc);
    assert!(is_safe_for_cross_file_inline(body, &masked));
}

#[test]
fn test_cross_file_rejects_private_enum() {
    let body = "NotifyOneStrategy::Lifo";
    let doc = r#"
enum NotifyOneStrategy {
    Fifo,
    Lifo,
}
"#;
    let masked = mask_comments_and_strings(doc);
    assert!(!is_safe_for_cross_file_inline(body, &masked));
}

#[test]
fn test_cross_file_rejects_private_method() {
    let body = "this.private_helper()";
    let doc = r#"
impl Foo {
    fn private_helper(&self) {}
}
"#;
    let masked = mask_comments_and_strings(doc);
    assert!(!is_safe_for_cross_file_inline(body, &masked));
}

#[test]
fn test_cross_file_allows_public_method_chain() {
    let body = "this.vals_flatten().next()";
    let doc = r#"
impl MatchedArg {
    pub(crate) fn vals_flatten(&self) -> Iter { self.vals.iter() }
}
"#;
    let masked = mask_comments_and_strings(doc);
    assert!(is_safe_for_cross_file_inline(body, &masked));
}
