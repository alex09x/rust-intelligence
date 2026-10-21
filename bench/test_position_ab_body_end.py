# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Rules of the body-end A/B classifier: the scanner model against known inputs and the two classes
on synthetic answers."""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from position_ab_body_end_rules import (  # noqa: E402
    body_end, call_sites, classify_incoming, classify_outgoing, moved_by_body_end,
)

FILE = "/opt/ws/src/lib.rs"
LINES = [
    "use std::task::Context;",                                                    # 1
    "pub fn poll_closed(&mut self, cx: &mut Context<'_>) -> Poll<()> {",         # 2
    "    let state = State::load(&self.inner.state);",                           # 3
    "    unset_tx_task(&self.inner.state);",                                     # 4
    "}",                                                                         # 5
    "fn plain() {",                                                              # 6
    "    helper(1);",                                                            # 7
    "    'outer: loop {",                                                        # 8
    "        break 'outer;",                                                     # 9
    "    }",                                                                     # 10
    "    helper(2);",                                                            # 11
    "}",                                                                         # 12
    "fn declared<'a>(x: &'a str) -> &'a str;",                                   # 13
    "fn after_declared() {",                                                     # 14
    "    helper(3);",                                                            # 15
    "}",                                                                         # 16
]


def item(side, line, name, sites, end=None):
    end_line = (end if end is not None else line) - 1
    return {side: {"uri": "file://" + FILE, "name": name, "range": {"start": {"line": line - 1, "character": 0}, "end": {"line": end_line, "character": 1}}},
            "fromRanges": [{"start": {"line": s - 1, "character": 4}, "end": {"line": s - 1, "character": 10}} for s in sites]}


def test_scanner_model_matches_the_rust_tests_in_both_versions():
    # the fixtures of crates/ridx/src/lsp/hierarchy/tests/scan_tests.rs with the values the Rust
    # scanner gave on the parent (old) and on eb9ba2e (new): 7/5, 2/4, 4/5, 2/1 and 6/6
    sig = ["pub fn poll_closed(&mut self, cx: &mut Context<'_>) -> Poll<()> {", "    let state = State::load(&self.inner.state);",
           "    unset_tx_task(&self.inner.state);", "    Poll::Pending", "}", "", "fn next_item() {}"]
    assert (body_end(sig, 1, False), body_end(sig, 1, True)) == (7, 5)
    ret = ["fn hidden() -> IgnoreMatch<'static> {", "    let m = Match::None;", "    m.invert()", "}"]
    assert (body_end(ret, 1, False), body_end(ret, 1, True)) == (2, 4)
    label = ["fn f() {", "    'outer: loop {", "        break 'outer;", "    }", "}"]
    assert (body_end(label, 1, False), body_end(label, 1, True)) == (4, 5)
    decl = ["fn declared<'a>(x: &'a str) -> &'a str;", "fn other() {}"]
    assert (body_end(decl, 1, False), body_end(decl, 1, True)) == (2, 1)
    chars = ["fn f() {", "    let open = '{';", "    let quote = '\\'';", "    let crab = '\\u{1F980}';", "    let byte = b'}';", "}"]
    assert (body_end(chars, 1, False), body_end(chars, 1, True)) == (6, 6)
    # the fixture of this file: old ends from the model itself, new ends by hand
    assert (body_end(LINES, 2, True), body_end(LINES, 6, True), body_end(LINES, 13, True)) == (5, 12, 13)


def test_call_sites_collects_from_ranges_and_the_item_end_per_item():
    assert call_sites([item("from", 2, "poll_closed", [3, 4], end=5)], "from") == {(FILE, 2, "poll_closed"): ({3, 4}, 5)}
    assert call_sites(None, "to") == {}


def test_moved_by_body_end_accepts_only_sites_inside_the_moved_span():
    assert moved_by_body_end([4, 5], [], 3, 5)
    assert moved_by_body_end([], [14, 15], 16, 13)
    assert not moved_by_body_end([2], [], 3, 5)
    assert not moved_by_body_end([], [12], 16, 13)


def test_incoming_call_sites_gained_in_the_newly_covered_span_are_the_class():
    # the label on line 8 made the old scan end at line 10: the call on line 11 was outside `plain`
    old6, new6 = body_end(LINES, 6, False), body_end(LINES, 6, True)
    assert old6 < 11 <= new6
    before = [item("from", 6, "plain", [7], end=old6)]
    after = [item("from", 6, "plain", [7, 11], end=new6)]
    ok, ev = classify_incoming(before, after, lambda p: LINES)
    assert ok and ev["added_count"] == 1 and ev["removed_count"] == 0
    assert ev["callers"][0]["caller"] == [FILE, 6, "plain"]


def test_incoming_call_sites_lost_from_the_wrongly_covered_span_are_the_class():
    old13, new13 = body_end(LINES, 13, False), body_end(LINES, 13, True)
    assert new13 < 15 <= old13
    ok, ev = classify_incoming([item("from", 13, "declared", [15], end=old13)], [item("from", 13, "declared", [], end=new13)], lambda p: LINES)
    assert ok and ev["removed_count"] == 1 and ev["added_count"] == 0


def test_only_the_item_range_end_moving_to_the_corrected_body_end_is_the_class():
    old6, new6 = body_end(LINES, 6, False), body_end(LINES, 6, True)
    ok, ev = classify_incoming([item("from", 6, "plain", [7], end=old6)], [item("from", 6, "plain", [7], end=new6)], lambda p: LINES)
    assert ok and ev["added_count"] == 0 and ev["callers"][0]["range_end_after"] == new6
    assert not classify_incoming([item("from", 6, "plain", [7], end=old6)], [item("from", 6, "plain", [7], end=new6 + 1)], lambda p: LINES)[0]


def test_a_call_site_outside_the_moved_span_is_not_the_class():
    old6, new6 = body_end(LINES, 6, False), body_end(LINES, 6, True)
    ok, _ = classify_incoming([item("from", 6, "plain", [7], end=old6)], [item("from", 6, "plain", [7, 3], end=new6)], lambda p: LINES)
    assert not ok
    assert not classify_incoming([], [], lambda p: LINES)[0]


def test_outgoing_callees_follow_the_item_body_end():
    old6, new6 = body_end(LINES, 6, False), body_end(LINES, 6, True)
    ok, ev = classify_outgoing([item("to", 99, "helper", [7])], [item("to", 99, "helper", [7, 11])], LINES, 6)
    assert ok and (ev["old_end"], ev["new_end"]) == (old6, new6) and ev["added_sites"] == [11]
    ok2, ev2 = classify_outgoing([item("to", 99, "helper", [15])], [], LINES, 13)
    assert ok2 and ev2["removed_sites"] == [15]
    assert not classify_outgoing([], [item("to", 99, "helper", [7])], LINES, 6)[0]
