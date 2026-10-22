# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Rules of the enclosing-function A/B classifier on synthetic files and answers."""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from position_ab_enclosing_rules import (  # noqa: E402
    CLASS_OWN_TARGET, CLASS_TEST_ONLY, cfg_test_attribute_above, cfg_test_module_of, classify_added_callers,
    duplicate_counts, module_opening, own_target_of, unchanged_callers,
)

FILE = "/opt/ws/pkg/src/lib.rs"
LINES = [
    "pub fn helper() -> u32 { 1 }",          # 1
    "pub fn lib_caller() -> u32 {",           # 2
    "    helper()",                           # 3
    "}",                                      # 4
    "#[cfg(test)]",                           # 5
    "#[allow(unused)]",                       # 6
    "mod tests {",                            # 7
    "    #[test]",                            # 8
    "    fn calls_helper() {",                # 9
    "        assert_eq!(super::helper(), 1);",  # 10
    "    }",                                  # 11
    "}",                                      # 12
    "mod plain {",                            # 13
    "    pub fn other() { super::helper(); }",  # 14
    "}",                                      # 15
    "#[test]",                                # 16
    "fn root_test() {",                       # 17
    "    helper();",                          # 18
    "}",                                      # 19
    "#[cfg(test)]",                           # 20
    "#[allow(dead_code)]",                    # 21
    "pub(crate) fn probe() {",                # 22
    "    helper();",                          # 23
    "}",                                      # 24
]


def item(line, name, sites, end, path=FILE):
    return {"from": {"uri": "file://" + path, "name": name, "range": {"start": {"line": line - 1, "character": 4}, "end": {"line": end - 1, "character": 1}}},
            "fromRanges": [{"start": {"line": s - 1, "character": 8}, "end": {"line": s - 1, "character": 14}} for s in sites]}


def test_module_opening_and_cfg_attribute_are_found_through_other_attributes():
    assert module_opening(LINES, "tests") == 7 and module_opening(LINES, "plain") == 13
    assert cfg_test_attribute_above(LINES, 7) == 5
    assert cfg_test_attribute_above(LINES, 13) is None
    assert cfg_test_module_of("pkg::tests::calls_helper", LINES) == ("tests", 7, 5)
    assert cfg_test_module_of("tests::calls_helper", LINES) == ("tests", 7, 5)  # crate-root module
    assert cfg_test_module_of("pkg::plain::other", LINES) is None
    assert cfg_test_module_of("pkg::lib_caller", LINES) is None
    assert cfg_test_attribute_above(LINES, 17) == 16 and cfg_test_attribute_above(LINES, 22) == 20
    assert cfg_test_attribute_above(LINES, 14) is None and cfg_test_attribute_above(LINES, 2) is None


def test_own_target_needs_a_package_manifest(tmp_path):
    pkg = tmp_path / "crates" / "pkg"
    (pkg / "tests").mkdir(parents=True)
    (pkg / "Cargo.toml").write_text("[package]\n")
    assert own_target_of(str(pkg / "tests" / "it.rs"), str(tmp_path)) == ("tests", "crates/pkg")
    assert own_target_of(str(pkg / "src" / "lib.rs"), str(tmp_path)) is None
    (tmp_path / "examples").mkdir()
    assert own_target_of(str(tmp_path / "examples" / "demo.rs"), str(tmp_path)) is None


def test_unchanged_callers_requires_same_sites_and_range_end():
    before = [item(2, "lib_caller", [3], 4)]
    assert unchanged_callers(before, before + [item(9, "calls_helper", [10], 11)])
    assert not unchanged_callers(before, [item(2, "lib_caller", [3], 5)])
    assert not unchanged_callers(before, [item(9, "calls_helper", [10], 11)])


def test_a_gained_test_module_caller_inside_its_body_is_e1():
    before = [item(2, "lib_caller", [3], 4)]
    after = before + [item(9, "calls_helper", [10], 11)]
    cls, ev = classify_added_callers(before, after, lambda p: LINES, lambda p, l, n: ("pkg::tests::calls_helper", 9), "/opt/ws")
    assert cls == CLASS_TEST_ONLY and ev["added_count"] == 1 and ev["added_sites"] == 1
    assert ev["added_callers"][0]["test_module"] == "tests" and ev["added_callers"][0]["cfg_line"] == 5


def test_a_gained_function_under_a_test_attribute_is_e1():
    before = [item(2, "lib_caller", [3], 4)]
    after = before + [item(17, "root_test", [18], 19), item(22, "probe", [23], 24)]
    cls, ev = classify_added_callers(before, after, lambda p: LINES, lambda p, l, n: (f"pkg::{n}", l), "/opt/ws")
    assert cls == CLASS_TEST_ONLY and [c["test_attribute_line"] for c in ev["added_callers"]] == [16, 20]


def test_a_gained_caller_of_a_plain_module_or_outside_its_body_is_not_classified():
    before = [item(2, "lib_caller", [3], 4)]
    cls, ev = classify_added_callers(before, before + [item(14, "other", [14], 14)], lambda p: LINES, lambda p, l, n: ("pkg::plain::other", 14), "/opt/ws")
    assert cls is None and not ev["added_callers"][0]["ok"]
    cls2, _ = classify_added_callers(before, before + [item(9, "calls_helper", [14], 11)], lambda p: LINES, lambda p, l, n: ("pkg::tests::calls_helper", 9), "/opt/ws")
    assert cls2 is None
    cls3, _ = classify_added_callers(before, before + [item(9, "calls_helper", [10], 12)], lambda p: LINES, lambda p, l, n: ("pkg::tests::calls_helper", 9), "/opt/ws")
    assert cls3 is None
    assert classify_added_callers(before, before, lambda p: LINES, lambda p, l, n: (None, None), "/opt/ws")[0] is None
    cls4, ev4 = classify_added_callers(before, before + [item(9, "calls_helper", [10], 11)], lambda p: LINES, lambda p, l, n: ("pkg::tests::calls_helper", 2), "/opt/ws")
    assert cls4 is None and ev4["added_callers"][0]["def_line"] == 2


def test_repeated_caller_items_or_call_sites_are_counted_and_refused():
    once = [item(2, "lib_caller", [3], 4)]
    assert duplicate_counts(once + once) == (1, 0)
    assert duplicate_counts([item(2, "lib_caller", [3, 3], 4)]) == (0, 1)
    assert duplicate_counts(None) == (0, 0)
    cls, ev = classify_added_callers(once, once + once + [item(9, "calls_helper", [10], 11)], lambda p: LINES, lambda p, l, n: ("pkg::tests::calls_helper", 9), "/opt/ws")
    assert cls is None and ev["duplicate_caller_items"] == 1
    cls2, ev2 = classify_added_callers(once, once + [item(9, "calls_helper", [10], 11)], lambda p: LINES, lambda p, l, n: ("pkg::tests::calls_helper", 9), "/opt/ws")
    assert cls2 == CLASS_TEST_ONLY and (ev2["duplicate_caller_items"], ev2["duplicate_from_ranges"]) == (0, 0)


def test_a_gained_caller_in_an_own_target_file_is_e2(tmp_path):
    pkg = tmp_path / "pkg"
    (pkg / "tests").mkdir(parents=True)
    (pkg / "Cargo.toml").write_text("[package]\n")
    path = str(pkg / "tests" / "it.rs")
    lines = ["fn it_calls() {", "    helper();", "}"]
    after = [item(1, "it_calls", [2], 3, path=path)]
    cls, ev = classify_added_callers([], after, lambda p: lines, lambda p, l, n: ("it::it_calls", 1), str(tmp_path))
    assert cls == CLASS_OWN_TARGET and ev["added_callers"][0]["target_dir"] == "tests"
