# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Rules of `position_ab_item_position.py`: checks over incomingCalls answers of two RIDX binaries
that differ by how the item is resolved (by its definition path before, by its position after).
Lines are 1-based; LSP ranges are 0-based."""

import re
from collections import Counter

from position_ab_body_end_rules import body_end, call_sites

CLASS_GAINED = "g1_callers_gained_whose_call_sites_resolve_to_the_item"
CLASS_LOST = "g2_callers_lost_whose_call_sites_resolve_to_another_item"
CLASS_LOCATIONS = "t1_locations_gained_from_units_reaching_the_item_through_another_unit"
MACRO_CALL = re.compile(r"\w!\s*[({\[]|\w!\s*$")
CLASS_FIXED = "t2_error_before_edits_after_each_resolving_to_the_definition"
CLASS_BOTH = "g3_callers_gained_and_lost_each_by_its_own_proof"


def range_counts(res):
    """{(path, start line, name): number of fromRanges} of an incomingCalls answer: call sites as
    ranges, not as distinct lines."""
    out = {}
    for item in res or []:
        node = item.get("from") or {}
        uri = node.get("uri", "")
        key = (uri[7:] if uri.startswith("file://") else uri, ((node.get("range") or {}).get("start") or {}).get("line", -1) + 1, node.get("name", ""))
        out[key] = out.get(key, 0) + len(item.get("fromRanges") or [])
    return out


def call_ranges(res):
    """{(path, start line, name): sorted [(line 1-based, character 0-based UTF-16)]} of the
    fromRanges of an incomingCalls answer."""
    out = {}
    for item in res or []:
        node = item.get("from") or {}
        uri = node.get("uri", "")
        key = (uri[7:] if uri.startswith("file://") else uri, ((node.get("range") or {}).get("start") or {}).get("line", -1) + 1, node.get("name", ""))
        out.setdefault(key, set()).update(
            ((r.get("start") or {}).get("line", -1) + 1, (r.get("start") or {}).get("character", 0)) for r in item.get("fromRanges") or []
        )
    return {k: sorted(v) for k, v in out.items()}


def ref_lines(refs):
    """{(path, 1-based line)} of a `ridx refs --json` reference list."""
    return {(r.get("file"), r.get("start_line")) for r in refs or []}


def caller_changes(before, after):
    """(added, removed, changed) over caller items keyed by (path, start line, name); `changed`
    are callers of both answers whose call sites or range end differ."""
    cb, ca = call_sites(before, "from"), call_sites(after, "from")
    added = sorted((k for k in ca if k not in cb), key=str)
    removed = sorted((k for k in cb if k not in ca), key=str)
    changed = sorted((k for k in ca if k in cb and ca[k] != cb[k]), key=str)
    return added, removed, changed, cb, ca


def inside_body(key, sites, lines):
    """Every call site lies in the caller's body by the lifetime-aware body-end model."""
    path, line, _ = key
    return bool(lines) and bool(sites) and all(line <= s <= body_end(lines, line, True) for s in sites)


def classify(before, after, lines_of, def_of, item_def):
    """(class or None, evidence). `def_of(path, line, utf16_character)` is the reader's definition at
    a call site, `(file, line)` or None; `item_def` is the definition of the item the request names.
    Callers of both answers must be unchanged (same call sites, same item range end). Then:
      gained callers: every call site resolves to the item (`def_of` == `item_def`) and lies in the
          caller's body by the body-end model;
      lost callers: no call site resolves to the item: each belongs to another item that carries the
          same path.
    g1 when only callers were gained, g2 when only lost, g3 when both; a lost call site that
    resolves to the item is a real loss and the row stays unclassified."""
    added, removed, changed, cb, ca = caller_changes(before, after)
    ranges_b, ranges_a = call_ranges(before), call_ranges(after)
    ev = {"item_definition": list(item_def) if item_def else None, "added_callers": [], "removed_callers": [], "changed_callers": [list(k) for k in changed]}
    if changed or not (added or removed) or not item_def:
        ev["reason"] = "a caller of both answers changed" if changed else "no caller added or removed" if item_def else "the item has no definition"
        return None, ev
    ok = True
    for key in added:
        sites = ranges_a.get(key, [])
        resolved = [tuple(def_of(key[0], l, c) or ()) == tuple(item_def) for l, c in sites]
        good = bool(sites) and all(resolved) and inside_body(key, sorted({l for l, _ in sites}), lines_of(key[0]) or [])
        ev["added_callers"].append({"caller": list(key), "call_sites": len(sites), "call_site_lines": sorted({l for l, _ in sites}),
                                    "sites_resolving_to_the_item": sum(resolved), "ok": good})
        ok = ok and good
    for key in removed:
        sites = ranges_b.get(key, [])
        resolved = [tuple(def_of(key[0], l, c) or ()) == tuple(item_def) for l, c in sites]
        good = bool(sites) and not any(resolved)
        ev["removed_callers"].append({"caller": list(key), "call_sites": len(sites), "call_site_lines": sorted({l for l, _ in sites}),
                                      "sites_resolving_to_the_item": sum(resolved), "ok": good})
        ok = ok and good
    ev["added_count"], ev["removed_count"] = len(added), len(removed)
    ev["added_sites"] = sum(c["call_sites"] for c in ev["added_callers"])
    ev["removed_sites"] = sum(c["call_sites"] for c in ev["removed_callers"])
    ev["added_site_lines"] = sum(len(c["call_site_lines"]) for c in ev["added_callers"])
    ev["removed_site_lines"] = sum(len(c["call_site_lines"]) for c in ev["removed_callers"])
    if not ok:
        return None, ev
    return (CLASS_BOTH if added and removed else CLASS_GAINED if added else CLASS_LOST), ev


def same_definition(a, b):
    """Two `(file, line)` definitions are the same when the lines agree and one file path is a
    suffix of the other: the driver records one file under the path prefix of each unit reaching it."""
    if not a or not b or a[1] != b[1]:
        return False
    x, y = str(a[0]).split("/"), str(b[0]).split("/")
    n = min(len(x), len(y), 3)
    return x[-n:] == y[-n:]


def is_generated_span(lines, span):
    """The location lies in text that expands to code: a derive list, an attribute, a macro
    invocation, or the body of an item that carries a derive (its fields are what the derived
    impls are written over). A reference the expansion makes is recorded there, where `ridx def`
    answers the macro or the field type, not the item the reference names."""
    if not lines or span[0] > len(lines):
        return False
    line = lines[span[0] - 1]
    text = line[span[1] - 1:span[3] - 1] if span[0] == span[2] else line[span[1] - 1:]
    if "derive(" in line or text.lstrip().startswith("#[") or line.lstrip().startswith("#[") or MACRO_CALL.search(text):
        return True
    for above in range(span[0] - 2, max(span[0] - 80, 0) - 1, -1):
        if not lines[above].strip():
            return False
        if "derive(" in lines[above]:
            return True
    return False


def classify_locations(before, after, cursor_file, lines_of, locations_of, def_of_col, cursor_def):
    """(class or None, evidence) for references, documentHighlight and rename. `def_of_col(path,
    line, col1)` is the reader's definition at a 1-based character position, `(file, line)` or
    None. The answers must lose no location; every location gained (a bare range of
    documentHighlight lies in `cursor_file`) must resolve to the definition under the cursor:
    consumers that reach the item through another unit, no longer dropped by the unit scoping."""
    locs = lambda res: Counter((p or cursor_file, span) for p, span in locations_of(res, lines_of))
    cb, ca = locs(before), locs(after)
    removed, added = cb - ca, ca - cb
    ev = {"cursor_definition": list(cursor_def) if cursor_def else None, "removed_count": sum(removed.values()), "added_count": sum(added.values()),
          "added_not_resolving": []}
    if removed or not added or not cursor_def:
        ev["reason"] = "a location was lost" if removed else "no location gained" if not added else "the cursor has no definition"
        return None, ev
    files = set()
    for (path, span), _ in sorted(added.items(), key=str):
        files.add(path)
        if not same_definition(def_of_col(path, span[0], span[1]), cursor_def):
            ev["added_not_resolving"].append([path, list(span)])
    ev["added_files"] = len(files)
    generated = [loc for loc in ev["added_not_resolving"] if is_generated_span((lines_of(loc[0]) if lines_of else None) or [], loc[1])]
    ev["added_generated"] = len(generated)
    ev["added_not_resolving"] = [loc for loc in ev["added_not_resolving"] if loc not in generated]
    return (CLASS_LOCATIONS if not ev["added_not_resolving"] else None), ev


def classify_error_fixed(after, cursor_file, lines_of, locations_of, def_of_col, cursor_def):
    """(class or None, evidence) for a row whose answer before was an error and after is a
    result: every location of the answer must resolve to the definition under the cursor (or be
    generated), as in `classify_locations` with nothing before."""
    cls, ev = classify_locations(None, after, cursor_file, lines_of, lambda res, lines: locations_of(res, lines) if res else [], def_of_col, cursor_def)
    return (CLASS_FIXED if cls == CLASS_LOCATIONS else None), ev
