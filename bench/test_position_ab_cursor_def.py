# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Rules of the cursor-precedence A/B classifier on synthetic answers."""
import os
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from position_ab_cursor_def_rules import (  # noqa: E402
    classify_cursor_definition, in_test_file, in_test_module, load_enclosing_refs, one_edit_form,
)

FILE = "/opt/ws/tests/rt_common.rs"
INVOCATION = (66, 1, 1445, 2)
ENCLOSING = [(INVOCATION, "Arc", True), ((8, 5, 8, 30), "Arc", True)]


def found(kind, path, span):
    sl, sc, el, ec = span
    return {"found": True, "kind": kind, "path": path, "start_line": sl, "start_col": sc, "end_line": el, "end_col": ec}


def test_load_enclosing_refs_keeps_only_spans_wide_enough_to_enclose_a_definition():
    with tempfile.TemporaryDirectory() as td:
        path = os.path.join(td, "c.ws.tsv")
        rows = [
            ("src/lib.rs", "66:1-1445:2", "macro", "apart", "alloc::sync::Arc"),       # multi-line: kept
            ("src/lib.rs", "7:5-7:8", "plain", "apart", "alloc::sync::Arc"),           # as wide as the name: dropped
            ("src/lib.rs", "8:5-8:30", "macro", "apart", "alloc::sync::Arc"),          # wider: kept
            ("src/lib.rs", "2:1-10:2", "plain", "contains", "fx::outer"),              # self-containing: dropped
            ("src/lib.rs", "9:1-9:12", "plain", "none", "core::option::Option<T>"),    # name without generics: kept
            ("src/lib.rs", "50:4-0:0", "plain", "apart", "fx::zero_end"),              # end before start: dropped
        ]
        with open(path, "w", encoding="utf-8") as fh:
            for stored, span, macro, self_col, target in rows:
                fh.write(f"REF\ts.ridx\t{stored}\t{span}\tFn\t{macro}\t1\tother\tdiffers\tother\t{self_col}\t{target}\n")
        got = load_enclosing_refs([path], "/opt/ws")
        assert got == {"/opt/ws/src/lib.rs": [((66, 1, 1445, 2), "Arc", True), ((8, 5, 8, 30), "Arc", True), ((9, 1, 9, 12), "Option", False)]}


def test_item_at_cursor_inside_the_invocation_is_the_class():
    before = found("Struct", "alloc::sync::Arc", (1, 1, 1, 1))
    after = found("Fn", "current_thread_scheduler::socket_from_blocking", (586, 8, 586, 28))
    ok, ev = classify_cursor_definition((586, 10), before, after, ENCLOSING)
    assert ok
    assert ev["enclosing_spans_at_cursor"] == [list(INVOCATION)]
    assert ev["enclosing_targets_at_cursor"] == ["Arc"]
    assert ev["before_kind"] == "enclosing_target"
    assert ev["definition_after"]["start_line"] == 586


def test_a_before_definition_already_at_the_cursor_is_not_the_class():
    at_cursor = found("Fn", "x::f", (586, 8, 586, 28))
    ok, _ = classify_cursor_definition((586, 10), at_cursor, at_cursor, ENCLOSING)
    assert not ok


def test_an_after_definition_outside_every_enclosing_span_is_not_the_class():
    before = found("Struct", "alloc::sync::Arc", (1, 1, 1, 1))
    after = found("Fn", "x::f", (2000, 4, 2000, 9))
    ok, _ = classify_cursor_definition((2000, 5), before, after, ENCLOSING)
    assert not ok


def test_a_cursor_outside_every_enclosing_reference_is_not_the_class():
    before = found("Struct", "alloc::sync::Arc", (1, 1, 1, 1))
    after = found("Fn", "x::f", (1500, 4, 1500, 9))
    ok, ev = classify_cursor_definition((1500, 5), before, after, ENCLOSING)
    assert not ok and ev["enclosing_spans_at_cursor"] == []


def test_a_missing_before_definition_with_an_item_after_is_the_class():
    after = found("Const", "fx::WIDE", (8, 12, 8, 16))
    ok, ev = classify_cursor_definition((8, 14), {"found": False}, after, ENCLOSING)
    assert ok and ev["definition_before"] is None and ev["before_kind"] == "none"


def test_a_before_definition_that_is_the_crate_root_module_around_the_invocation_is_the_class():
    # The invocation's references include one to the crate root (path ""), whose nameless Mod
    # definition spans the whole file: it contains the cursor but encloses the invocation itself.
    before = found("Mod", "", (1, 1, 547, 2))
    after = found("Fn", "current_thread_scheduler::basic_fs", (140, 8, 140, 16))
    ok, ev = classify_cursor_definition((140, 10), before, after, ENCLOSING + [((135, 1, 431, 2), "", True)])
    assert ok and ev["before_kind"] == "crate_root_module"


def test_test_file_and_test_module_detection():
    assert in_test_file("tokio/tests/rt_common.rs")
    assert in_test_file("crates/x/src/foo_tests.rs")
    assert in_test_file("crates/x/benches/bench.rs")
    assert not in_test_file("serde_core/src/ser/impls.rs")
    assert not in_test_file("crates/x/src/testing.rs")
    assert in_test_module("glob::tests::literal1")
    assert not in_test_module("serde_core::ser::Error")
    assert not in_test_module("x::tests")


def test_a_workspace_edit_with_both_forms_keeps_only_document_changes():
    edit = {"changes": {"file:///a.rs": [{"range": {}, "newText": "x"}]}, "documentChanges": [{"textDocument": {"uri": "file:///a.rs"}, "edits": []}]}
    assert list(one_edit_form(edit)) == ["documentChanges"]
    assert one_edit_form({"changes": {}}) == {"changes": {}}
    assert one_edit_form([{"uri": "file:///a.rs"}]) == [{"uri": "file:///a.rs"}]
