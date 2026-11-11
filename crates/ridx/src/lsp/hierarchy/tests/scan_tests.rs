/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! The body-end scanner behind call hierarchy must not mistake a lifetime or a label for the
//! start of a char literal: one tick on a line used to swallow the rest of that line, so an
//! opening brace after `Context<'_>` or `IgnoreMatch<'static>` was never seen and the body ended
//! at the first `;`, which made callers inside such functions invisible to incomingCalls.

use crate::lsp::hierarchy::scan::find_function_body_end_line;

#[test]
fn a_lifetime_in_the_signature_does_not_hide_the_opening_brace() {
    let lines = [
        "pub fn poll_closed(&mut self, cx: &mut Context<'_>) -> Poll<()> {",
        "    let state = State::load(&self.inner.state);",
        "    unset_tx_task(&self.inner.state);",
        "    Poll::Pending",
        "}",
        "",
        "fn next_item() {}",
    ];
    assert_eq!(find_function_body_end_line(&lines, 1), 5);
}

#[test]
fn a_static_lifetime_in_the_return_type_does_not_hide_the_opening_brace() {
    let lines = ["fn hidden() -> IgnoreMatch<'static> {", "    let m = Match::None;", "    m.invert()", "}"];
    assert_eq!(find_function_body_end_line(&lines, 1), 4);
}

#[test]
fn a_loop_label_does_not_hide_its_brace() {
    let lines = ["fn f() {", "    'outer: loop {", "        break 'outer;", "    }", "}"];
    assert_eq!(find_function_body_end_line(&lines, 1), 5);
}

#[test]
fn char_literals_with_braces_and_escapes_still_do_not_count() {
    let lines = ["fn f() {", "    let open = '{';", "    let quote = '\\'';", "    let crab = '\\u{1F980}';", "    let byte = b'}';", "}"];
    assert_eq!(find_function_body_end_line(&lines, 1), 6);
}

#[test]
fn a_declaration_without_a_body_still_ends_at_its_semicolon() {
    let lines = ["fn declared<'a>(x: &'a str) -> &'a str;", "fn other() {}"];
    assert_eq!(find_function_body_end_line(&lines, 1), 1);
}

#[test]
fn a_declared_name_is_no_call_site() {
    use crate::lsp::hierarchy::scan::is_fn_declaration_name;
    let lines = ["    fn get_aabb(&self) -> Option<Aabb> {", "    pub(crate) fn get(&self) {}", "        let b = self.get_aabb();", "    defn(x)"];
    assert!(is_fn_declaration_name(&lines, 1, 8));
    assert!(is_fn_declaration_name(&lines, 2, 19));
    assert!(!is_fn_declaration_name(&lines, 3, 22));
    assert!(!is_fn_declaration_name(&lines, 4, 5));
}
