# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Positive and negative example per rule of position_ab_gap_rules and position_ab_gap_reverse, and
for the cursor helpers of position_ab_gap_cursor, on fixed lines."""

import sys
from pathlib import Path

bench_dir = Path(__file__).resolve().parent
if str(bench_dir) not in sys.path:
    sys.path.insert(0, str(bench_dir))

from position_ab_gap_reverse import (  # noqa: E402
    REVERSE_ORDER, cfg_context, classify_reverse, impl_for_target, impl_keyword_in_signature, impls_for_other_types,
)
from position_ab_gap import ordered, row_set  # noqa: E402
from position_ab_gap_cursor import (  # noqa: E402
    derive_above, in_attribute_block, in_impl_header, in_macro_block, in_macro_call, in_macro_rules, is_binding_site,
    qualified, token_at, token_class,
)
from position_ab_gap_rules import (  # noqa: E402
    RULE_ORDER, body_end_line, caller_category, classify_document_highlight, classify_implementation,
    classify_incoming_calls, is_call_site, location_kind, names_symbol, store_kind,
)

REPO = "/w/serde"
WS_TABLE = [("serde-abc.ridx", "serde/src/lib.rs"), ("serde-abc.ridx", "serde/src/de/mod.rs"),
            ("test_data-1.ridx", "crates/parser/src/../test_data/generated/runner.rs")]


def test_store_kind_distinguishes_stored_unstored_and_outside_files():
    assert store_kind("/w/serde/serde/src/lib.rs", REPO, WS_TABLE) == "ws"
    assert store_kind("/w/serde/crates/parser/test_data/generated/runner.rs", REPO, WS_TABLE) == "ws"
    assert store_kind("/w/serde/serde/src/ser/mod.rs", REPO, WS_TABLE) == "ws_unstored"
    assert store_kind("/opt/cargo/registry/src/index-1/serde_json-1.0.140/src/de.rs", REPO, WS_TABLE) == "registry"
    assert store_kind("/opt/cargo/git/checkouts/x-abc/def0/src/lib.rs", REPO, WS_TABLE) == "git"
    assert store_kind("/opt/toolchains/t/lib/rustlib/src/rust/library/core/src/fmt/mod.rs", REPO, WS_TABLE) == "sysroot"
    assert store_kind("/opt/other/src/lib.rs", REPO, WS_TABLE) == "external"


def _loc(store, text, decl=False, ref=False):
    return {"store": store, "line_text": text, "on_decl_line": decl, "ridx_ref_at_line": ref}


def test_location_kinds():
    assert location_kind(_loc("ws", "#[derive(Debug, Serialize)]")) == "attribute"
    assert location_kind(_loc("ws", "struct Foo {", decl=True)) == "decl"
    assert location_kind(_loc("ws", "struct Foo {")) == "other"
    assert location_kind(_loc("ws", "unsafe impl<T> Send for Inner<T> {}")) == "unsafe_impl"
    assert location_kind(_loc("ws", "impl Serialize for Foo {")) == "impl"
    assert location_kind(_loc("ws", "array_impls! {")) == "macro"
    assert location_kind({**_loc("ws", "    Ord,"), "in_attribute": True}) == "attribute"
    assert location_kind({**_loc("ws", "struct CounterSettings {"), "derive_above": True}) == "derived_item"
    assert location_kind(_loc("ws", "struct CounterSettings {")) == "other"
    assert location_kind({**_loc("ws", "> TarjanScc<'graph, N, S, A, Neighbors>"), "in_impl_header": True}) == "impl_header"
    assert location_kind({**_loc("ws", "Enum by Left -> ast::Enum,"), "in_macro": True}) == "macro"
    assert names_symbol("pub struct Enum {", "Enum") is True
    assert names_symbol("fn test_map_access_to_enum() {", "PotentialVisitor") is False
    assert names_symbol("use tokio::io::{copy, repeat};", "copy_ext") is False


def test_implementation_rules_in_order():
    impl_ref, impl_noref = _loc("ws", "impl Serialize for Foo {", ref=True), _loc("ws", "impl Serialize for Foo {")
    derive, decl = _loc("ws", "#[derive(Debug, Serialize)]"), _loc("ws", "struct Foo {", decl=True)
    dep, unstored = _loc("registry", "impl Serialize for Bar {"), _loc("ws_unstored", "impl Serialize for Baz {")
    macro, unsafe_impl = _loc("ws", "array_impls! {"), _loc("ws", "unsafe impl<T> Send for Inner<T> {}")
    assert classify_implementation("b", [impl_ref])[0] == "i2_cursor_unresolved"
    assert classify_implementation("a", [impl_ref], definition_names_symbol=False)[0] == "i7_definition_of_another_item"
    assert classify_implementation("a", [unstored, unstored])[0] == "i3_impls_in_unstored_files"
    assert classify_implementation("a", [dep, unstored])[0] == "i1_impls_outside_workspace"
    assert classify_implementation("a", [derive, decl, dep])[0] == "i5_generated_impls"
    assert classify_implementation("a", [derive, macro])[0] == "i6_macro_invocation_impls"
    assert classify_implementation("a", [decl, unsafe_impl, macro])[0] == "i4a_unsafe_impl_lines"
    assert classify_implementation("a", [derive, impl_noref, unsafe_impl])[0] == "i4b_impl_line_without_reference"
    derived = {**_loc("ws", "struct CounterSettings {"), "derive_above": True}
    header = {**_loc("ws", "> Iterator for TarjanScc<'graph, N, S, A, Neighbors>", ref=True), "in_impl_header": True}
    assert classify_implementation("a", [derived, derived])[0] == "i5_generated_impls"
    assert classify_implementation("a", [header, impl_ref])[0] == "i4d_multiline_impl_header"
    assert classify_implementation("a", [impl_noref, impl_ref])[0] == "i4c_impl_line_with_reference"
    assert classify_implementation("a", [_loc("ws", "fn helper() {}")])[0] == "unclassified"
    assert classify_implementation("a", [])[0] == "unclassified"
    assert RULE_ORDER["implementation"][0] == "i2_cursor_unresolved"


def test_call_site_test_matches_the_handler():
    assert is_call_site("    foo(1);", 8) is True
    assert is_call_site("    foo::<u8>(1);", 8) is True
    assert is_call_site("    map(foo);", 12) is False
    assert is_call_site("    foo :: <u8>(1);", 8) is False
    assert is_call_site("    let f = foo;", 16) is False


def test_body_end_line_matches_the_handler_scan():
    lines = ["pub fn from_ast(", "    cx: &Ctxt,", ") -> Option<Container> {", '    let s = "}";', "    call(x); // }",
             "    /* } */ inner();", "}", "fn next() {}", "struct S;", "fn open() {"]
    assert body_end_line(lines, 1) == 7
    assert body_end_line(lines, 8) == 8
    assert body_end_line(lines, 9) == 9
    assert body_end_line(lines, 10) == 10
    assert body_end_line(["fn a() -> [u8; 4] {", "    x", "}"], 1) == 3


def test_caller_categories_and_their_priority():
    call = {"is_macro_expansion": False, "call_site": True}
    value = {"is_macro_expansion": False, "call_site": False}
    macro = {"is_macro_expansion": True, "call_site": True}
    known = {"store": "ws", "kind": 12, "ridx_def_at_caller": True, "body_end": 80, "call_line": 72}
    assert caller_category({"store": "registry", "kind": 12, "refs": [call]}) == "outside"
    assert caller_category({"store": "ws_unstored", "kind": 12, "refs": []}) == "outside"
    assert caller_category({"store": "ws", "kind": 14, "refs": [call]}) == "non_fn_caller"
    assert caller_category({"store": "ws", "kind": 12, "refs": [], "from_in_macro": True}) == "macro"
    assert caller_category({"store": "ws", "kind": 6, "refs": [macro, macro]}) == "macro"
    assert caller_category({**known, "refs": [call], "from_in_macro": True}) == "enclosing_lookup_failed"
    assert caller_category({"store": "ws", "kind": 12, "refs": [], "refs_total": 0}) == "no_ridx_references"
    assert caller_category({"store": "ws", "kind": 12, "refs": [], "refs_total": 3}) == "missed"
    assert caller_category({"store": "ws", "kind": 12, "refs": [macro, value]}) == "not_call_site"
    assert caller_category({**known, "refs": [call], "ridx_def_at_caller": False}) == "enclosing_unknown"
    assert caller_category({**known, "refs": [call], "body_end": 70}) == "body_end_short"
    assert caller_category({**known, "refs": [macro, call]}) == "enclosing_lookup_failed"
    outside = {"store": "sysroot", "kind": 12, "refs": []}
    missed = {"store": "ws", "kind": 12, "refs": []}
    assert classify_incoming_calls("b", [missed])[0] == "c3_item_not_resolved"
    assert classify_incoming_calls("a", [outside, missed])[0] == "c4_workspace_callers_missed"
    assert classify_incoming_calls("a", [outside, {**known, "refs": [call]}])[0] == "c4_workspace_callers_missed"
    assert classify_incoming_calls("a", [outside, {"store": "ws", "kind": 12, "refs": [value]}])[0] == "c5_call_site_not_recognized"
    assert classify_incoming_calls("a", [outside, {"store": "ws", "kind": 12, "refs": [], "refs_total": 0}])[0] == "c7_item_without_references"
    assert classify_incoming_calls("a", [outside, {"store": "ws", "kind": 12, "refs": [macro]}])[0] == "c2_callers_in_macro_expansions"
    assert classify_incoming_calls("a", [outside, {"store": "ws", "kind": 14, "refs": []}])[0] == "c6_caller_not_a_function"
    assert classify_incoming_calls("a", [outside])[0] == "c1_callers_outside_workspace"
    cls, _note, cats = classify_incoming_calls("a", [])
    assert (cls, cats) == ("unclassified", [])


def test_token_at_token_class_and_qualifier_on_fixed_lines():
    line = '    let x: &\'a str = foo.await?; // "s" 42'
    assert token_at(line, 9) == "x" and token_class("x") == "ident"
    assert token_at(line, 14) == "'a" and token_class("'a") == "lifetime"
    assert token_at(line, 5) == "let" and token_class("let") == "keyword"
    assert token_at(line, 27) == "await" and token_class("await") == "exit_loop"
    assert token_at(line, 31) == "?" and token_class("?") == "exit_loop"
    assert token_at(line, 38) == '"..."' and token_class('"..."') == "literal"
    assert token_at(line, 42) == "42" and token_class("42") == "literal"
    assert token_at("    if c == 'x' {", 14) == "'.'" and token_class("'.'") == "literal"
    assert token_at("    a + b", 7) == "+" and token_class("+") == "punct"
    assert token_at("short", 20) == "" and token_class("") == "punct"
    assert qualified("fn f() -> Result<Self::Ok, Self::Error> {", 36) is True
    assert qualified("fn f() -> Result<Self::Ok, Self::Error> {", 11) is False
    assert qualified("    std::fmt::Result", 10) is True


def test_macro_detection_on_the_line_and_in_macro_rules_bodies():
    assert in_macro_call('    println!("{}", value);', 20) is True
    assert in_macro_call('    println!("{}", value);', 8) is False
    assert in_macro_call("    let value = compute();", 12) is False
    lines = ["macro_rules! forward {", "    ($t:ty) => {", "        impl Foo for $t {}", "    };", "}", "fn after() {}"]
    assert in_macro_rules(lines, 2) is True
    assert in_macro_rules(lines, 5) is False
    block = ["impl_deserialize_num! {", "    bool, deserialize_bool", "    u8, deserialize_u8", "}", "fn after() {", "    g();", "}", "array_impls!(1 2 3);"]
    assert in_macro_block(block, 2) is True
    assert in_macro_block(block, 4) is False
    assert in_macro_block(block, 5) is False
    assert in_macro_block(block, 7) is False
    assert in_macro_rules(["macro_rules! one { () => {} }", "fn after() {}"], 1) is False


def test_multiline_attribute_impl_header_and_derive_detection():
    lines = ["#[derive(", "    Debug,", "    Copy,", ")]", "pub struct Handle(pub u32);", "impl<", "    N: Hash,",
             "> TarjanScc<N> {", "    fn go() {}", "}", "fn f() -> impl Iterator<Item = u8> {", "    x", "}"]
    assert in_attribute_block(lines, 1) is True
    assert in_attribute_block(lines, 2) is True
    assert in_attribute_block(lines, 4) is False
    assert in_impl_header(lines, 7) is True
    assert in_impl_header(lines, 6) is True
    assert in_impl_header(lines, 8) is False
    assert in_impl_header(lines, 11) is False
    assert derive_above(lines, 4) is True
    assert derive_above(["/// doc", "#[derive(Debug)]", "struct S;"], 2) is True
    assert derive_above(["#[repr(C)]", "struct S;"], 1) is False
    assert derive_above(["#[derive(Debug)]", "struct A;", "struct B;"], 2) is False


def test_binding_site_detection():
    assert is_binding_site("    let mut buf = Vec::new();", 13, "buf") is True
    assert is_binding_site("    let total = count + 1;", 21, "count") is False
    assert is_binding_site("    for item in items {", 9, "item") is True
    assert is_binding_site("        Some(value) => value,", 14, "value") is True
    assert is_binding_site("        Some(value) => value,", 24, "value") is False
    assert is_binding_site("    items.iter().map(|item| item.len())", 27, "item") is True
    assert is_binding_site("pub fn parse(input: &str, limit: usize) -> Out {", 14, "input") is True
    assert is_binding_site("pub fn parse(input: &str, limit: usize) -> Out {", 46, "Out") is False
    assert is_binding_site("    limit: usize,", 5, "limit") is True
    assert is_binding_site("    pub limit: usize,", 9, "limit") is False
    assert is_binding_site("    call(limit);", 10, "limit") is False


def test_document_highlight_rules_in_order():
    ident = {"token": "value", "token_class": "ident", "in_macro": False, "qualified": False}
    local = {"same_file": True, "store": "ws", "line": 40, "line_text": "    let value = 1;", "col": 9, "token": "value"}
    item = {"same_file": False, "store": "ws", "line": 8, "line_text": "pub fn value() {}", "col": 8, "token": "value"}
    none_refs, elsewhere, here = {"total": 0, "same_file": 0}, {"total": 3, "same_file": 0}, {"total": 3, "same_file": 2}
    hl = classify_document_highlight
    assert hl({"token": "return", "token_class": "exit_loop"}, "b", None, none_refs)[0] == "h2a_exit_or_loop_keyword"
    assert hl({"token": "'a", "token_class": "lifetime"}, "a", None, none_refs)[0] == "h2b_keyword_literal_or_lifetime"
    assert hl({**ident, "in_macro": True}, "b", local, none_refs)[0] == "h3_macro_site"
    assert hl({**ident, "in_macro": True}, "a", local, none_refs)[0] == "h4a_resolved_no_references"
    assert hl(ident, "b", local, none_refs)[0] == "h1_local_binding"
    assert hl(ident, "a", local, none_refs)[0] == "h4a_resolved_no_references"
    assert hl(ident, "a", item, elsewhere, "registry")[0] == "h4b_external_item_references_elsewhere"
    assert hl(ident, "a", item, elsewhere, "ws")[0] == "h4d_workspace_item_references_elsewhere"
    assert hl(ident, "a", item, here, "registry")[0] == "h4c_resolved_references_in_file"
    assert hl(ident, "b", None, none_refs)[0] == "h5f_no_definition_either"
    assoc = {**item, "line_text": "    type Error: Error;"}
    assert hl({**ident, "qualified": True}, "b", assoc, none_refs)[0] == "h5a_associated_type_path"
    assert hl(ident, "b", assoc, none_refs)[0] == "h5e_workspace_item_unresolved"
    assert hl(ident, "b", {**item, "line": 1, "line_text": "//! Formatting."}, none_refs)[0] == "h5b_module_path_segment"
    assert hl(ident, "b", {**item, "line_text": "pub mod fmt;"}, none_refs)[0] == "h5b_module_path_segment"
    assert hl(ident, "b", {**item, "store": "sysroot"}, none_refs)[0] == "h5c_item_outside_workspace"
    assert hl(ident, "b", {**item, "line_text": "    TupleEnd,"}, none_refs)[0] == "h5d_enum_variant"
    assert hl(ident, "b", {**item, "line_text": "    Captured(Name),"}, none_refs)[0] == "h5d_enum_variant"
    assert hl(ident, "b", {**item, "line_text": "OsStringKind (Unix; 0, Windows; 1)", "in_macro": True}, none_refs)[0] == "h5g_definition_in_macro_input"
    assert hl(ident, "b", item, none_refs)[0] == "h5e_workspace_item_unresolved"
    assert RULE_ORDER["document_highlight"][-1] == "h5f_no_definition_either"


def test_reverse_rules_follow_the_re_asks_and_the_cursor():
    code = {"token_class": "ident", "line_text": "    x.lock();"}
    attr = {"token_class": "ident", "line_text": "#![allow(clippy::too_many_lines)]"}
    lit = {"token_class": "literal", "line_text": '#[serde(default = "MyDefault::my_default")]'}
    ra_def = {"same_file": True, "line_text": "fn lock(&self) {"}
    assert classify_reverse("document_highlight", {"ra_reask": "a", "ridx_reask": "a", "cursor": code})[0] == "r1_ra_answers_on_reask"
    assert classify_reverse("incoming_calls", {"ra_reask": "c", "ridx_reask": "a", "cursor": code})[0] == "unclassified"
    assert classify_reverse("incoming_calls", {"ra_reask": "b", "ridx_reask": "b", "cursor": code})[0] == "unclassified"
    assert classify_reverse("document_highlight", {"ra_reask": "b", "ridx_reask": "a", "cursor": attr, "ra_definition": None})[0] == "r4_cursor_in_attribute_or_string"
    assert classify_reverse("document_highlight", {"ra_reask": "b", "ridx_reask": "a", "cursor": lit, "ra_definition": ra_def})[0] == "r4_cursor_in_attribute_or_string"
    assert classify_reverse("document_highlight", {"ra_reask": "b", "ridx_reask": "a", "cursor": code, "ra_definition": None})[0] == "r3_ra_no_definition_at_cursor"
    gated = {**code, "cfg_context": "cfg_macro:cfg_rt"}
    assert classify_reverse("document_highlight", {"ra_reask": "b", "ridx_reask": "a", "cursor": gated, "ra_definition": None})[0] == "r3a_cursor_in_cfg_macro_block"
    assert classify_reverse("document_highlight", {"ra_reask": "b", "ridx_reask": "a", "cursor": gated, "ra_definition": ra_def})[0] == "r2_ra_empty_confirmed"
    attr_gated = {**code, "cfg_context": "cfg_attr:#[cfg(feature = \"rt\")]"}
    assert classify_reverse("implementation", {"ra_reask": "b", "ridx_reask": "a", "cursor": attr_gated, "ra_definition": None})[0] == "r3b_cursor_under_cfg_attribute"
    in_macro = {**code, "in_macro": True}
    assert classify_reverse("document_highlight", {"ra_reask": "b", "ridx_reask": "a", "cursor": in_macro, "ra_definition": None})[0] == "r3c_cursor_in_macro"
    other = [{"line_text": "impl<T: ?Sized + Send> AmbiguousIfSend<Invalid> for T {}"}, {"line_text": "unsafe impl Send for Wrapper<Invalid> {}"}]
    own = [{"line_text": "impl<'a, T: ?Sized> Deref for MutexGuard<'a, T> {"}]
    ev = {"ra_reask": "b", "ridx_reask": "a", "cursor": code, "ra_definition": ra_def, "symbol": "Invalid"}
    assert classify_reverse("implementation", {**ev, "ridx_answer": other})[0] == "r5_ridx_impls_for_other_types"
    assert classify_reverse("implementation", {**ev, "symbol": "MutexGuard", "ridx_answer": own})[0] == "r2_ra_empty_confirmed"
    sig = [{"line_text": "pub fn iter(&self) -> impl Iterator<Item = &SceneDependency> {"}]
    assert classify_reverse("implementation", {**ev, "symbol": "SceneDependency", "ridx_answer": sig + own})[0] == "r6_ridx_impl_keyword_in_signature"
    assert impl_keyword_in_signature("impl Foo for Bar {") is False
    assert impl_keyword_in_signature("unsafe impl Send for Bar {}") is False
    assert classify_reverse("incoming_calls", {**ev, "ridx_answer": other})[0] == "r2_ra_empty_confirmed"
    assert REVERSE_ORDER[-1] == "r2_ra_empty_confirmed"


def test_cfg_context_finds_the_gate_of_a_line():
    lines = ["cfg_rt! {", "    pub(crate) fn spawn() {", "        inner();", "    }", "}", "fn plain() {", "    x();", "}",
             "#[cfg(feature = \"io-util\")]", "mod util {", "    fn read() {", "        y();", "    }", "}"]
    assert cfg_context(lines, 2) == "cfg_macro:cfg_rt"
    assert cfg_context(lines, 6) is None
    assert cfg_context(lines, 11) == "cfg_attr:#[cfg(feature = \"io-util\")]"
    assert cfg_context(["#![cfg_attr(loom, allow(dead_code))]", "fn f() {", "    g();", "}"], 2) is None
    assert cfg_context(["#![cfg(feature = \"full\")]", "fn f() {", "    g();", "}"], 2) == "cfg_attr:#![cfg(feature = \"full\")]"
    assert cfg_context(["#[cfg(test)]", "fn a() {}", "fn b() {", "    c();", "}"], 3) is None
    assert cfg_context(["impl Foo {", "    #[cfg(unix)]", "    /// doc", "    fn a(&self) {", "        b();", "    }", "}"], 4) == "cfg_attr:#[cfg(unix)]"


def test_impl_for_target_parsing():
    assert impl_for_target("impl<T: ?Sized + Send> AmbiguousIfSend<Invalid> for T {}") == "T"
    assert impl_for_target("impl<'a, T: ?Sized> Deref for MutexGuard<'a, T> {") == "MutexGuard"
    assert impl_for_target("impl fmt::Display for crate::Error {") == "Error"
    assert impl_for_target("impl Foo {") is None
    assert impl_for_target("    let x = 1;") is None
    assert impls_for_other_types("Invalid", [{"line_text": "impl Trait<Invalid> for T {}"}, {"line_text": "impl Foo {"}]) is False
    assert impls_for_other_types("Invalid", []) is False


def test_rows_of_the_bevy_run_that_took_no_part_in_the_rule_design():
    """Evidence copied from the bevy sidecar of the step A run; the rules were written on serde,
    tokio, rust-analyzer and ripgrep rows. Positions: examples/ui/scroll_and_overflow/
    overflow_transform.rs:141:4, crates/bevy_gizmos/src/lib.rs:130:8, examples/stress_tests/
    many_cubes.rs:547:37, crates/bevy_ecs/src/query/state.rs:1307:35, crates/bevy_pbr/src/prepass/
    mod.rs:902:12, crates/bevy_scene/src/scene.rs:149:12."""
    value_ref = {"is_macro_expansion": False, "call_site": False}
    main_caller = {"store": "ws", "kind": 12, "refs": [value_ref], "from_in_macro": False, "ridx_def_at_caller": True,
                   "body_end": 14, "call_line": 12, "refs_total": 1}
    assert classify_incoming_calls("a", [main_caller])[0] == "c5_call_site_not_recognized"
    build = {"store": "ws", "kind": 12, "refs": [], "from_in_macro": False, "ridx_def_at_caller": True, "body_end": 144,
             "call_line": 134, "refs_total": 0}
    assert classify_incoming_calls("a", [build, {**build, "body_end": 122, "call_line": 112}])[0] == "c7_item_without_references"
    cursor = {"token": "looking_at", "token_class": "ident", "in_macro": False, "qualified": False}
    ra_def = {"same_file": False, "store": "ws", "line": 187, "line_text": "pub fn looking_at(mut self, target: Vec3, up: impl TryInto<Dir3>) -> Self {", "col": 12, "token": "looking_at"}
    assert classify_document_highlight(cursor, "a", ra_def, {"total": 3, "same_file": 0}, "ws")[0] == "h4d_workspace_item_references_elsewhere"
    cursor = {"token": "ReadOnly", "token_class": "ident", "in_macro": False, "qualified": True}
    ra_def = {"same_file": False, "store": "ws", "line": 338, "line_text": "type ReadOnly: ReadOnlyQueryData<State = <Self as WorldQuery>::State>;", "col": 10, "token": "ReadOnly"}
    assert classify_document_highlight(cursor, "b", ra_def, {"total": 0, "same_file": 0}, None)[0] == "h5a_associated_type_path"
    decl = {"store": "ws", "line_text": "pub struct SpecializedPrepassMaterialPipelineCache {", "on_decl_line": True, "ridx_ref_at_line": True}
    attr = {"store": "ws", "line_text": "#[derive(Resource, Deref, DerefMut, Default)]", "on_decl_line": False, "ridx_ref_at_line": True}
    assert classify_implementation("a", [decl, attr], True)[0] == "i5_generated_impls"
    ev = {"ra_reask": "b", "ridx_reask": "a", "symbol": "SceneDependency",
          "cursor": {"token_class": "ident", "line_text": "pub struct SceneDependency {"},
          "ra_definition": {"same_file": True, "line_text": "pub struct SceneDependency {"},
          "ridx_answer": [{"line_text": "pub fn iter(&self) -> impl Iterator<Item = &SceneDependency> {"}]}
    assert classify_reverse("implementation", ev)[0] == "r6_ridx_impl_keyword_in_signature"


def test_row_set_from_first_answers_and_retry():
    def ra(outcome, retry=None):
        r = {"outcome": outcome}
        if retry is not None:
            r["retry"] = {"outcome": retry}
        return r

    b, a = {"outcome": "b"}, {"outcome": "a"}
    assert row_set(b, ra("a")) == "target"
    assert row_set(b, ra("c", "a")) == "target"
    assert row_set(a, ra("a")) == "other"
    assert row_set(b, ra("b", "a")) == "ra_noise"
    assert row_set(a, ra("b", "a")) == "ra_noise"
    assert row_set(a, ra("b", "b")) == "reverse"
    assert row_set(a, ra("b")) == "reverse"
    assert row_set(b, ra("c", "b")) == "ra_timeout_then_empty"
    assert row_set(a, ra("c", "b")) == "ra_timeout_then_empty"
    assert row_set(b, ra("c", "c")) == "ra_timeout"
    assert row_set(a, ra("c")) == "ra_timeout"
    assert row_set(a, ra("c", "a")) == "other"
    assert row_set(b, ra("b", "b")) == "other"


def test_evidence_lists_have_a_fixed_order():
    a = {"path": "ws:b.rs", "line": 2}
    b = {"path": "ws:a.rs", "line": 9}
    c = {"path": "ws:a.rs", "line": 3}
    assert ordered([a, b, c]) == [a, c, b]  # canonical JSON: "line" sorts before "path"
    assert ordered([c, b, a]) == [a, c, b]
    assert ordered([]) == []
