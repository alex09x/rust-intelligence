# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Rules of `position_ab_enclosing.py`: checks over incomingCalls answers of two RIDX binaries that
differ by the enclosing-function lookup of call hierarchy (every shard of the call site's file is
tried, not only the first). Lines are 1-based; LSP ranges are 0-based."""

import json
import re
from pathlib import Path

from position_ab_body_end_rules import body_end, call_sites

CLASS_TEST_ONLY = "e1_callers_are_test_only_functions_held_by_the_test_shard_of_their_file"
CLASS_OWN_TARGET = "e2_callers_in_a_file_of_its_own_target"

CFG_TEST = re.compile(r"#\[cfg\((.*\b)?test\b|#\[test\]")
OWN_TARGET_DIRS = ("tests", "examples", "benches")


def module_opening(lines, name):
    """1-based line of `mod <name> {` (with any visibility) in `lines`, or None."""
    pat = re.compile(r"^\s*(pub(\([^)]*\))?\s+)?mod\s+" + re.escape(name) + r"\s*\{")
    for i, line in enumerate(lines):
        if pat.match(line):
            return i + 1
    return None


def cfg_test_attribute_above(lines, mod_line):
    """1-based line of a `#[cfg(... test ...)]` or `#[test]` attribute among the attribute and doc
    lines directly above `mod_line` (a `mod` or `fn` header), or None."""
    i = mod_line - 2
    while i >= 0 and (lines[i].strip().startswith("#[") or lines[i].strip().startswith("///") or not lines[i].strip()):
        if CFG_TEST.search(lines[i]):
            return i + 1
        i -= 1
    return None


def cfg_test_module_of(def_path, lines):
    """(module name, mod line, cfg line) for the first module segment of `def_path` (every segment
    but the item's own name; paths are crate-relative, so the first one may be a root module of the
    file) that opens in `lines` under a `#[cfg(test)]` attribute, else None."""
    for seg in def_path.split("::")[:-1]:
        mod_line = module_opening(lines, seg)
        if mod_line is None:
            continue
        cfg_line = cfg_test_attribute_above(lines, mod_line)
        if cfg_line is not None:
            return seg, mod_line, cfg_line
    return None


def own_target_of(abs_path, repo):
    """(target dir, package dir relative to repo) when the file lies under `tests/`, `examples/` or
    `benches/` of a package (a directory holding a Cargo.toml), else None."""
    p = Path(abs_path)
    for parent in p.parents:
        if parent.name in OWN_TARGET_DIRS and (parent.parent / "Cargo.toml").is_file():
            pkg = parent.parent
            try:
                rel = str(pkg.relative_to(repo))
            except ValueError:
                rel = str(pkg)
            return parent.name, rel
    return None


def duplicate_counts(res):
    """(caller items repeated, fromRanges repeated within one item) in an incomingCalls answer: a
    caller held by two shards must appear once, with each call site once."""
    seen, dup_items, dup_ranges = set(), 0, 0
    for item in res or []:
        node = item.get("from") or {}
        key = json.dumps([node.get("uri"), node.get("range"), node.get("name")], sort_keys=True)
        if key in seen:
            dup_items += 1
        seen.add(key)
        ranges = [json.dumps(r, sort_keys=True) for r in item.get("fromRanges") or []]
        dup_ranges += len(ranges) - len(set(ranges))
    return dup_items, dup_ranges


def unchanged_callers(before, after):
    """True when every caller of the before answer is in the after answer with the same call sites
    and the same item range end."""
    cb, ca = call_sites(before, "from"), call_sites(after, "from")
    return all(k in ca and ca[k] == v for k, v in cb.items())


def check_added_caller(key, sites_end, lines):
    """The added caller's call sites lie in its body as the corrected body-end scan gives it, and the
    caller item's range ends at that body end."""
    path, line, name = key
    sites, end = sites_end
    if not lines:
        return False, {"caller": [path, line, name], "reason": "file not readable"}
    body = body_end(lines, line, True)
    ok = bool(sites) and all(line <= s <= body for s in sites) and end == body
    return ok, {"caller": [path, line, name], "body_end": body, "range_end": end, "call_sites": sorted(sites)}


def classify_added_callers(before, after, lines_of, def_of, repo):
    """(class or None, evidence): the callers gained by incomingCalls, nothing lost or changed, no
    caller item or call site repeated, each gained caller inside its body-end span and found by the
    reader's definition at its name (`def_of(path, line, name)` gives `(definition path, line)`),
    and each either a test-only function of its file (e1: in a `#[cfg(test)]` module, or itself
    under a `#[cfg(test)]` or `#[test]` attribute) or a function of a file of its own target (e2).
    The class is e2 when every gained caller is in an own-target file, else e1 (one lookup over a
    second shard either way)."""
    dup_items, dup_ranges = duplicate_counts(after)
    counts = {"duplicate_caller_items": dup_items, "duplicate_from_ranges": dup_ranges}
    if dup_items or dup_ranges:
        return None, {"reason": "a caller item or a call site is repeated in the after answer", **counts}
    if not unchanged_callers(before, after):
        return None, {"reason": "a caller of the before answer is missing or changed in the after answer", **counts}
    cb, ca = call_sites(before, "from"), call_sites(after, "from")
    added = sorted((k for k in ca if k not in cb), key=str)
    if not added:
        return None, {"reason": "no caller added", **counts}
    callers, kinds = [], set()
    for key in added:
        path, line, name = key
        lines = lines_of(path) or []
        ok, ev = check_added_caller(key, ca[key], lines)
        def_path, def_line = def_of(path, line, name) if ok else (None, None)
        ev["def_path"], ev["def_line"] = def_path, def_line
        ok = ok and def_path is not None and def_line == line
        tm = cfg_test_module_of(def_path, lines) if ok else None
        attr = cfg_test_attribute_above(lines, line) if ok and not tm else None
        own = own_target_of(path, repo) if ok and not tm and not attr else None
        if tm:
            ev["test_module"], ev["mod_line"], ev["cfg_line"] = tm
            kinds.add("test_module")
        elif attr:
            ev["test_attribute_line"] = attr
            kinds.add("test_attribute")
        elif own:
            ev["target_dir"], ev["package_dir"] = own
            kinds.add("own_target")
        else:
            ok = False
        ev["ok"] = ok
        callers.append(ev)
    all_ok = all(c["ok"] for c in callers)
    cls = None
    if all_ok and kinds == {"own_target"}:
        cls = CLASS_OWN_TARGET
    elif all_ok:
        cls = CLASS_TEST_ONLY
    return cls, {"added_callers": callers, "added_count": len(callers), "added_sites": sum(len(c.get("call_sites", [])) for c in callers), **counts}
