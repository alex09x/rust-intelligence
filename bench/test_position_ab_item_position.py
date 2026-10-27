# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Rules of the item-position A/B classifier on synthetic answers and definition proofs."""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from position_ab_item_position_rules import (  # noqa: E402
    CLASS_BOTH, CLASS_FIXED, CLASS_SITES, CLASS_GAINED, CLASS_LOCATIONS, CLASS_LOST, call_ranges, caller_changes, classify, classify_error_fixed, classify_locations,
    is_generated_span, same_definition,
)

FILE = "/opt/ws/tests/test_de.rs"
ITEM = ("tests/test_de.rs", 1)
OTHER = ("other/src/lib.rs", 1)
LINES = ["fn test() {}", "fn test_u8() {", "    test();", "}", "fn test_u16() {", "    test();", "}"]


def item(line, name, sites, end):
    return {"from": {"uri": "file://" + FILE, "name": name, "range": {"start": {"line": line - 1, "character": 3}, "end": {"line": end - 1, "character": 1}}},
            "fromRanges": [{"start": {"line": s - 1, "character": 4}, "end": {"line": s - 1, "character": 8}} for s in sites]}


def resolves_to(target):
    return lambda path, line, col: target


def test_call_ranges_keep_every_range_and_caller_changes_see_new_callers():
    assert call_ranges([item(2, "test_u8", [3, 3], 4)]) == {(FILE, 2, "test_u8"): [(3, 4)]}
    assert call_ranges(None) == {}
    added, removed, changed, _, _ = caller_changes([item(2, "test_u8", [3], 4)], [item(2, "test_u8", [3], 4), item(5, "test_u16", [6], 7)])
    assert (added, removed, changed) == ([(FILE, 5, "test_u16")], [], [])


def test_callers_gained_whose_call_sites_resolve_to_the_item_are_g1():
    cls, ev = classify([], [item(2, "test_u8", [3], 4), item(5, "test_u16", [6], 7)], lambda p: LINES, resolves_to(ITEM), ITEM)
    assert cls == CLASS_GAINED and ev["added_count"] == 2 and ev["added_sites"] == 2 and ev["added_callers"][0]["sites_resolving_to_the_item"] == 1


def test_a_gained_call_site_resolving_elsewhere_or_outside_the_body_is_refused():
    assert classify([], [item(2, "test_u8", [3], 4)], lambda p: LINES, resolves_to(OTHER), ITEM)[0] is None
    assert classify([], [item(2, "test_u8", [6], 4)], lambda p: LINES, resolves_to(ITEM), ITEM)[0] is None
    assert classify([], [item(2, "test_u8", [3], 4)], lambda p: LINES, resolves_to(None), ITEM)[0] is None


def test_callers_lost_whose_call_sites_resolve_to_another_item_are_g2():
    cls, ev = classify([item(5, "test_u16", [6], 7)], [], lambda p: LINES, resolves_to(OTHER), ITEM)
    assert cls == CLASS_LOST and ev["removed_count"] == 1 and ev["removed_sites"] == 1


def test_a_lost_call_site_resolving_to_the_item_is_a_real_loss():
    cls, ev = classify([item(5, "test_u16", [6], 7)], [], lambda p: LINES, resolves_to(ITEM), ITEM)
    assert cls is None and ev["removed_callers"][0]["ok"] is False and ev["removed_callers"][0]["sites_resolving_to_the_item"] == 1


def test_gained_and_lost_in_one_row_need_both_proofs():
    def def_of(path, line, col):
        return ITEM if line == 3 else OTHER
    cls, _ = classify([item(5, "test_u16", [6], 7)], [item(2, "test_u8", [3], 4)], lambda p: LINES, def_of, ITEM)
    assert cls == CLASS_BOTH
    cls2, _ = classify([item(5, "test_u16", [6], 7)], [item(2, "test_u8", [3], 4)], lambda p: LINES, resolves_to(ITEM), ITEM)
    assert cls2 is None


def test_a_changed_or_unchanged_answer_or_an_item_without_definition_is_not_classified():
    same = [item(2, "test_u8", [3], 4)]
    assert classify(same, same, lambda p: LINES, resolves_to(ITEM), ITEM)[0] is None
    assert classify(same, [item(2, "test_u8", [3], 5)], lambda p: LINES, resolves_to(ITEM), ITEM)[0] is None
    assert classify([], same, lambda p: LINES, resolves_to(ITEM), None)[0] is None


def locations(spans):
    return lambda res, lines_of: [(FILE if p else None, s) for p, s in (res or [])]


def test_locations_gained_that_resolve_to_the_cursor_definition_are_t1():
    before = [("p", (3, 5, 3, 9))]
    after = before + [("p", (6, 5, 6, 9)), (None, (9, 1, 9, 4))]
    cls, ev = classify_locations(before, after, FILE, None, locations(None), lambda path, line, col: ITEM, ITEM)
    assert cls == CLASS_LOCATIONS and ev["added_count"] == 2 and ev["removed_count"] == 0


def test_a_lost_location_or_a_gained_one_resolving_elsewhere_is_refused():
    before = [("p", (3, 5, 3, 9))]
    assert classify_locations(before, [], FILE, None, locations(None), lambda *a: ITEM, ITEM)[0] is None
    cls, ev = classify_locations(before, before + [("p", (6, 5, 6, 9))], FILE, None, locations(None), lambda *a: OTHER, ITEM)
    assert cls is None and ev["added_not_resolving"] == [[FILE, [6, 5, 6, 9]]]
    assert classify_locations(before, before, FILE, None, locations(None), lambda *a: ITEM, ITEM)[0] is None


def test_one_file_under_two_unit_prefixes_is_the_same_definition():
    assert same_definition(("crates/bevy_feathers/crates/bevy_ui/src/ui_node.rs", 7), ("crates/bevy_ui/src/ui_node.rs", 7))
    assert not same_definition(("crates/a/src/x.rs", 7), ("crates/a/src/x.rs", 8))
    assert not same_definition(("crates/a/src/x.rs", 7), ("crates/b/src/x.rs", 7))
    assert not same_definition(None, ("crates/a/src/x.rs", 7))


def test_derive_attribute_and_macro_invocation_spans_are_generated_but_plain_code_is_not():
    lines = ["#[derive(Deserialize)]", "", "let _p = tracing::info_span!(\"x\").entered();", "", "let a = b;"]
    assert is_generated_span(lines, (1, 10, 1, 21))
    assert is_generated_span(lines, (3, 9, 3, 30))
    assert not is_generated_span(lines, (5, 9, 5, 10))


def test_fields_of_a_derived_item_and_a_macro_with_a_block_are_generated():
    lines = ["#[derive(Serialize)]", "enum E {", "    Var(u32),", "}", "", "fn f() {}", "forward_to_deserialize_any! {", "    bool", "}"]
    assert is_generated_span(lines, (3, 9, 3, 12))
    assert not is_generated_span(lines, (6, 4, 6, 5))
    assert is_generated_span(lines, (7, 1, 9, 2))


def test_an_error_before_and_edits_after_that_resolve_to_the_cursor_definition_are_t2():
    after = [("p", (3, 5, 3, 9)), ("p", (6, 5, 6, 9))]
    cls, ev = classify_error_fixed(after, FILE, None, locations(None), lambda *a: ITEM, ITEM)
    assert cls == CLASS_FIXED and ev["added_count"] == 2
    cls, ev = classify_error_fixed(after, FILE, None, locations(None), lambda *a: OTHER, ITEM)
    assert cls is None and len(ev["added_not_resolving"]) == 2
    assert classify_error_fixed(None, FILE, None, locations(None), lambda *a: ITEM, ITEM)[0] is None


def test_a_caller_of_both_answers_that_gained_call_sites_is_g4_only_if_each_new_site_resolves():
    before = [item(2, "test_u8", [3], 4)]
    after = [item(2, "test_u8", [3, 5], 7)]  # one caller, a second call site on line 5
    lines = ["fn test() {}", "fn test_u8() {", "    test();", "    other();", "    test();", "    done();", "}"]
    cls, ev = classify(before, after, lambda p: lines, resolves_to(ITEM), ITEM)
    assert cls == CLASS_SITES and ev["changed_callers_detail"][0]["call_sites_gained"] == 1
    assert classify(before, after, lambda p: lines, resolves_to(OTHER), ITEM)[0] is None
    assert classify(after, before, lambda p: lines, resolves_to(ITEM), ITEM)[0] is None
