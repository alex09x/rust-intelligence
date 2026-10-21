# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Rules of the name-span A/B classifier on synthetic answers."""
import os
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from position_ab_name_span_rules import (  # noqa: E402
    classify_fallback, classify_invocation, classify_lens, classify_list, classify_self_calls, classify_single,
    contains_pos, is_test_attribute, load_contains, load_invocation_refs, locations_of, span_of,
)

FILE = "/opt/ws/src/lib.rs"
WHOLE_FN = (2, 1, 10, 2)
WHOLE_FILE = (1, 1, 30, 2)
CONTAINS = {FILE: {WHOLE_FN, WHOLE_FILE}}


def rng(sl, sc, el, ec):
    return {"start": {"line": sl - 1, "character": sc - 1}, "end": {"line": el - 1, "character": ec - 1}}


def loc(span):
    return {"uri": "file://" + FILE, "range": rng(*span)}


def test_load_contains_reads_only_the_contains_rows_and_resolves_relative_paths():
    with tempfile.TemporaryDirectory() as td:
        path = os.path.join(td, "c.ws.tsv")
        with open(path, "w", encoding="utf-8") as fh:
            fh.write("REF\ts.ridx\tsrc/lib.rs\t2:1-10:2\tFn\tplain\t9\tother\tdiffers\tother\tcontains\touter\n")
            fh.write("REF\ts.ridx\tsrc/lib.rs\t7:5-7:11\tFn\tplain\t1\tname\tdiffers\tother\tapart\thelper\n")
            fh.write("REF\ts.ridx\t/opt/registry/x/src/lib.rs\t1:1-9:2\tConst\tmacro\t9\tunread\tdiffers\tfile\tcontains\tt\n")
        got = load_contains([path], "/opt/ws")
        assert got == {"/opt/ws/src/lib.rs": {(2, 1, 10, 2)}, "/opt/registry/x/src/lib.rs": {(1, 1, 9, 2)}}


def test_span_of_converts_utf16_columns_with_the_file_text():
    lines = ["let s = \"\U0001F980\"; helper();"]
    assert span_of(rng(1, 14, 1, 20), lines) == (1, 13, 1, 19)
    assert span_of(rng(1, 14, 1, 20), None) == (1, 14, 1, 20)


def test_locations_of_walks_locations_highlights_calls_and_edits():
    res = [loc(WHOLE_FN), {"range": rng(7, 5, 7, 11), "kind": 2}]
    got = locations_of(res, lambda p: None)
    assert got == [(FILE, WHOLE_FN), (None, (7, 5, 7, 11))]
    calls = [{"from": {"uri": "file://" + FILE, "range": rng(20, 1, 22, 2), "name": "f"}, "fromRanges": [rng(21, 5, 21, 8)]}]
    got = locations_of(calls, lambda p: None)
    assert (FILE, (21, 5, 21, 8)) in got and (FILE, (20, 1, 22, 2)) in got
    edit = {"changes": {"file://" + FILE: [{"range": rng(2, 4, 2, 9), "newText": "x"}]}}
    assert locations_of(edit, lambda p: None) == [(FILE, (2, 4, 2, 9))]


def test_contains_pos_is_end_exclusive():
    assert contains_pos(WHOLE_FN, 2, 1) and contains_pos(WHOLE_FN, 10, 1)
    assert not contains_pos(WHOLE_FN, 10, 2) and not contains_pos(WHOLE_FN, 1, 30)


def test_classify_list_accepts_two_nested_self_containing_ranges_and_nothing_added():
    ok, ev = classify_list([(FILE, WHOLE_FN), (FILE, WHOLE_FILE)], [], CONTAINS, FILE)
    assert ok and ev["removed_count"] == 2 and ev["two_nested_ranges"] and ev["not_self_containing"] == []
    ok, ev = classify_list([(None, WHOLE_FN), (None, WHOLE_FILE)], [], CONTAINS, FILE)
    assert ok, "bare documentHighlight ranges belong to the cursor's file"


def test_classify_list_rejects_other_losses_and_additions():
    ok, ev = classify_list([(FILE, (7, 5, 7, 11))], [], CONTAINS, FILE)
    assert not ok and ev["not_self_containing"] == [[FILE, [7, 5, 7, 11]]]
    ok, ev = classify_list([(FILE, WHOLE_FN)], [(FILE, (7, 5, 7, 11))], CONTAINS, FILE)
    assert not ok and ev["added_count"] == 1
    ok, _ = classify_list([], [], CONTAINS, FILE)
    assert not ok


def test_classify_single_needs_cursor_and_before_answer_inside_one_skipped_reference():
    before, after = (FILE, (2, 4, 2, 9)), (FILE, (5, 12, 5, 17))
    ok, ev = classify_single((5, 14), before, after, CONTAINS[FILE])
    assert ok and ev["self_containing_spans_at_cursor"] == [[1, 1, 30, 2], [2, 1, 10, 2]]
    ok, _ = classify_single((5, 14), before, None, CONTAINS[FILE])
    assert ok
    ok, _ = classify_single((5, 14), (FILE, (40, 1, 40, 5)), after, CONTAINS[FILE])
    assert not ok, "the before answer lies outside every skipped reference"
    ok, _ = classify_single((35, 1), before, after, CONTAINS[FILE])
    assert not ok, "the cursor lies outside every skipped reference"
    ok, _ = classify_single((5, 14), before, before, CONTAINS[FILE])
    assert not ok, "the answer did not change"


def lens(line, title):
    return {"range": rng(line, 4, line, 9), "command": {"title": title, "command": "ridx.showReferences"}}


def test_classify_lens_requires_the_drop_to_match_the_skipped_references_at_the_lens():
    lines = [""] * 40
    before = [lens(2, "5 references"), lens(20, "3 references")]
    after = [lens(2, "3 references"), lens(20, "3 references")]
    ok, ev = classify_lens(before, after, CONTAINS[FILE], lines)
    assert ok and ev["changed"] == [{"line": 2, "before": "5 references", "after": "3 references", "self_containing_at_lens": 2, "ok": True}]
    ok, _ = classify_lens(before, [lens(2, "4 references"), lens(20, "3 references")], CONTAINS[FILE], lines)
    assert not ok, "the count fell by one, two references were skipped"
    ok, _ = classify_lens(before, [lens(2, "3 references")], CONTAINS[FILE], lines)
    assert not ok, "a lens disappeared"
    ok, _ = classify_lens(before, before, CONTAINS[FILE], lines)
    assert not ok, "nothing changed"


def test_classify_list_accepts_the_test_attribute_declaration_of_the_generated_constant():
    lines = [""] * 30
    lines[0] = "#[test]"
    lines[20] = "    #[tokio::test]"
    lines_of = lambda path: lines  # noqa: E731
    assert is_test_attribute(lines, (1, 1, 1, 8)) and is_test_attribute(lines, (21, 5, 21, 19))
    assert not is_test_attribute(lines, (1, 1, 1, 4)), "part of the attribute is not the declaration"
    ok, ev = classify_list([(FILE, (1, 1, 1, 8)), (FILE, WHOLE_FN), (FILE, WHOLE_FILE)], [], CONTAINS, FILE, lines_of)
    assert ok and ev["removed_count"] == 3 and ev["test_attribute_declarations"] == [[FILE, [1, 1, 1, 8]]]
    ok, _ = classify_list([(FILE, (7, 5, 7, 11))], [], CONTAINS, FILE, lines_of)
    assert not ok


def test_classify_fallback_needs_the_before_definition_inside_the_enclosing_reference():
    def d(kind, path, sl, sc, el, ec):
        return {"found": True, "kind": kind, "path": path, "start_line": sl, "start_col": sc, "end_line": el, "end_col": ec}

    before = d("Fn", "fx::outer", 2, 4, 2, 9)
    ok, ev = classify_fallback((5, 14), before, None, CONTAINS[FILE])
    assert ok and ev["after_kind"] == "none" and ev["definition_before"] == {"kind": "Fn", "path": "fx::outer", "start_line": 2, "start_col": 4}
    ok, ev = classify_fallback((5, 14), before, d("Mod", "", 1, 1, 30, 2), CONTAINS[FILE])
    assert ok and ev["after_kind"] == "crate_root_module"
    ok, ev = classify_fallback((5, 14), before, d("Struct", "fx::Inner", 5, 12, 5, 17), CONTAINS[FILE])
    assert ok and ev["after_kind"] == "item_at_cursor"
    ok, ev = classify_fallback((5, 14), before, d("Fn", "fx::helper", 40, 1, 40, 5), CONTAINS[FILE])
    assert not ok and ev["after_kind"] == "other"
    ok, _ = classify_fallback((5, 14), d("Fn", "fx::far", 40, 1, 40, 5), None, CONTAINS[FILE])
    assert not ok, "the before definition lies outside every skipped reference"
    ok, _ = classify_fallback((35, 1), before, None, CONTAINS[FILE])
    assert not ok, "the cursor lies outside every skipped reference"
    ok, _ = classify_fallback((5, 14), before, before, CONTAINS[FILE])
    assert not ok, "the definition did not change"


def test_classify_self_calls_accepts_callees_inside_the_enclosing_reference_only():
    ok, ev = classify_self_calls([(None, (6, 17, 6, 20)), (FILE, (2, 4, 2, 9))], [], CONTAINS[FILE])
    assert ok and ev["removed_count"] == 2 and ev["outside_self_containing"] == []
    ok, ev = classify_self_calls([(FILE, (40, 1, 40, 5))], [], CONTAINS[FILE])
    assert not ok and ev["outside_self_containing"] == [[FILE, [40, 1, 40, 5]]]
    ok, _ = classify_self_calls([(None, (6, 17, 6, 20))], [(FILE, (12, 1, 12, 5))], CONTAINS[FILE])
    assert not ok, "a callee was added"


def test_classify_invocation_matches_the_after_target_to_a_kept_macro_reference_over_the_cursor():
    def d(kind, path, sl, sc):
        return {"found": True, "kind": kind, "path": path, "start_line": sl, "start_col": sc, "end_line": sl, "end_col": sc + 3}

    invocation = [((1, 1, 30, 2), "Arc"), ((1, 1, 30, 2), "Runtime"), ((40, 1, 45, 2), "Vec")]
    before, after = d("Fn", "fx::outer", 2, 4), d("Struct", "alloc::sync::Arc", 269, 12)
    ok, ev = classify_invocation((5, 14), before, after, CONTAINS[FILE], invocation)
    assert ok and ev["invocation_spans_of_the_after_target"] == [[1, 1, 30, 2]]
    ok, _ = classify_invocation((5, 14), before, d("Struct", "alloc::vec::Vec", 1, 1), CONTAINS[FILE], invocation)
    assert not ok, "the Vec reference does not contain the cursor"
    ok, _ = classify_invocation((5, 14), before, d("Struct", "fx::Inner", 5, 12), CONTAINS[FILE], invocation)
    assert not ok, "the after definition is the item at the cursor, not an invocation target"
    ok, _ = classify_invocation((5, 14), d("Fn", "fx::far", 40, 1), after, CONTAINS[FILE], invocation)
    assert not ok, "the before definition lies outside every skipped reference"


def test_load_invocation_refs_keeps_multi_line_macro_references_that_are_not_skipped():
    with tempfile.TemporaryDirectory() as td:
        path = os.path.join(td, "c.ws.tsv")
        with open(path, "w", encoding="utf-8") as fh:
            fh.write("REF\ts.ridx\tsrc/lib.rs\t66:1-1445:2\tStruct\tmacro\t1380\tother\tnone\titem\tnone\talloc::sync::Arc\n")
            fh.write("REF\ts.ridx\tsrc/lib.rs\t66:1-1445:2\tConst\tmacro\t1380\tother\tdiffers\titem\tcontains\tmod::test_a\n")
            fh.write("REF\ts.ridx\tsrc/lib.rs\t7:5-7:11\tFn\tmacro\t1\tname\tnone\tother\tnone\thelper\n")
            fh.write("REF\ts.ridx\tsrc/lib.rs\t2:1-10:2\tFn\tplain\t9\tother\tdiffers\tother\tcontains\touter\n")
        got = load_invocation_refs([path], "/opt/ws")
        assert got == {"/opt/ws/src/lib.rs": [((66, 1, 1445, 2), "Arc")]}
