# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Class of a differing implementation row between two RIDX binaries that differ by what the
handler takes for an impl header: a reference line starting with the word `impl` or containing
`impl ` before, a line that begins, after attributes, `default` and `unsafe`, with the keyword
`impl` as a whole word after.

  i1  locations lost, none gained: no lost location is on an impl header line;
  i2  locations gained, none lost: every location gained is on an impl header line the old rule
      refused (`unsafe impl<T: ?Sized> Send for X`: a line with `<T:` or `where `);
  i3  both, each half by its own proof.
In every class each location kept is on an impl header line."""

import re

CLASS_LOST = "i1_locations_lost_that_are_no_impl_header"
CLASS_GAINED = "i2_locations_gained_that_are_impl_headers"
CLASS_BOTH = "i3_locations_lost_and_gained_each_by_its_own_proof"
IMPL_HEADER = re.compile(r"^\s*(#\[[^\]]*\]\s*)*(default\s+)?(unsafe\s+)?impl(?![A-Za-z0-9_])")


def is_impl_header(line):
    return bool(IMPL_HEADER.match(line or ""))


def classify_impl_header(before, after, lines_of, locations_of):
    """(class or None, evidence). `lines_of(path)` gives the 0-based list of the file's lines,
    `locations_of(res, lines_of)` the (path, span) pairs of an answer; span lines are 1-based."""
    locs = lambda res: sorted(set((p, tuple(s)) for p, s in locations_of(res, lines_of)), key=str)
    b, a = locs(before), locs(after)
    text = lambda loc: ((lines_of(loc[0]) or [""] * loc[1][0])[loc[1][0] - 1:] or [""])[0]
    lost = [loc for loc in b if loc not in a]
    gained = [loc for loc in a if loc not in b]
    pair = lambda loc: [loc[0], list(loc[1])]
    ev = {"lost_count": len(lost), "gained_count": len(gained), "kept_count": len(a)}
    ev["lost_headers"] = [pair(loc) for loc in lost if is_impl_header(text(loc))]
    ev["gained_non_headers"] = [pair(loc) for loc in gained if not is_impl_header(text(loc))]
    ev["kept_non_headers"] = [pair(loc) for loc in a if not is_impl_header(text(loc))]
    if not (lost or gained) or ev["lost_headers"] or ev["gained_non_headers"] or ev["kept_non_headers"]:
        return None, ev
    return (CLASS_BOTH if lost and gained else CLASS_LOST if lost else CLASS_GAINED), ev
