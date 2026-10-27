# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from position_ab_impl_header_rules import CLASS_BOTH, CLASS_GAINED, CLASS_LOST, CLASS_USED, classify_impl_header, is_head_position, is_impl_header  # noqa: E402

FILE = "/opt/ws/src/lib.rs"
LINES = ["impl Tr for S {}", "fn take(x: impl Tr) {}", "impl_tr!(S);", "unsafe impl Tr for U {}"]


def locs(res, lines_of):
    return [(FILE, (line, 1, line, 3)) for line in res or []]


def lines_of(path):
    return LINES


def test_header_line_model_matches_the_handler_rule():
    for line in ["impl Tr for S {}", "  impl<T> Tr for Vec<T> {", "unsafe impl Send for S {}", "#[cfg(test)] impl Tr for S {}", "default unsafe impl<T> Tr for T {}"]:
        assert is_impl_header(line), line
    for line in ["fn take(x: impl Tr) {}", "impl_tr!(S);", "implement(Tr);", "// impl Tr for S", ""]:
        assert not is_impl_header(line), line


def test_lost_non_header_lines_with_the_headers_kept_are_i1():
    cls, ev = classify_impl_header([1, 2, 3, 4], [1, 4], lines_of, locs)
    assert cls == CLASS_LOST and ev["lost_count"] == 2 and ev["kept_count"] == 2


def test_a_lost_header_a_gained_location_or_a_kept_non_header_is_refused():
    assert classify_impl_header([1, 4], [1], lines_of, locs)[0] is None
    assert classify_impl_header([1], [1, 2], lines_of, locs)[0] is None
    assert classify_impl_header([1, 2], [1, 2], lines_of, locs)[0] is None
    cls, ev = classify_impl_header([1, 2], [2], lines_of, locs)
    assert cls is None and ev["lost_headers"] == [[FILE, [1, 1, 1, 3]]] and ev["kept_non_headers"] == [[FILE, [2, 1, 2, 3]]]


def test_gained_impl_headers_are_i2_and_gained_with_lost_non_headers_are_i3():
    assert classify_impl_header([1], [1, 4], lines_of, locs)[0] == CLASS_GAINED
    cls, ev = classify_impl_header([1, 2], [1, 4], lines_of, locs)
    assert cls == CLASS_BOTH and ev["lost_count"] == 1 and ev["gained_count"] == 1


def test_a_header_that_only_uses_the_item_is_lost_as_i4_but_a_lost_head_is_refused():
    lines = ["impl Tr for S {}", "impl Wrapper<Tr> for S {}", "fn take(x: impl Tr) {}", "impl<T: Tr> Other for T {}"]
    ls = lambda res, lf: [(FILE, (line, col, line, col + 2)) for line, col in res or []]
    lf = lambda path: lines
    cls, ev = classify_impl_header([(1, 6), (2, 14), (3, 17), (4, 9)], [(1, 6)], lf, ls)
    assert cls == CLASS_USED and len(ev["lost_used_by_header"]) == 2 and ev["lost_count"] == 3
    assert classify_impl_header([(1, 6), (2, 14)], [(2, 14)], lf, ls)[0] is None
    assert classify_impl_header([(1, 6), (2, 14)], [(1, 6)], lf, ls)[0] == CLASS_USED
    assert is_head_position("impl S where S: Tr {}", 0) and not is_head_position("impl S where S: Tr {}", 17)
