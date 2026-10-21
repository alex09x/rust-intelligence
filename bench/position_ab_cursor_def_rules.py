# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Rules of `position_ab_cursor_def.py`: pure functions over the reader's definition at the cursor
of the two binaries (`ridx def --json` answers) and the references of the cursor's file that are
wide enough to enclose a definition (`ref_spans` rows).

Spans are 1-based, end-exclusive `(start line, start col, end line, end col)` in characters.
"""
from position_ab_name_span_rules import contains_pos

CLASS_ITEM = "d1_item_at_cursor_answers_instead_of_the_enclosing_reference"
TEST_DIRS = frozenset({"tests", "test", "benches", "examples"})


def load_enclosing_refs(paths, repo):
    """Kept references (`self` not `contains`) that may enclose a definition, keyed by absolute
    file path: (span, last segment of the target path, macro flag). A reference may enclose a
    definition when its span covers more than one line or is wider than its target's name, the
    same test `IndexReader::may_enclose_definition` applies."""
    out = {}
    for path in paths:
        with open(path, encoding="utf-8", errors="replace") as fh:
            for p in (line.rstrip("\n").split("\t") for line in fh):
                if len(p) < 12 or p[0] != "REF" or p[10] == "contains":
                    continue
                start, end = p[3].split("-")
                sl, sc = (int(x) for x in start.split(":"))
                el, ec = (int(x) for x in end.split(":"))
                name = p[11].rsplit("::", 1)[-1].split("<", 1)[0]
                if (el, ec) < (sl, sc) or (el == sl and ec - sc <= len(name)):
                    continue
                stored = p[2]
                abs_path = stored if stored.startswith("/") else f"{repo.rstrip('/')}/{stored}"
                out.setdefault(abs_path, []).append(((sl, sc, el, ec), name, p[5] == "macro"))
    return out


def one_edit_form(res):
    """A WorkspaceEdit carrying both `changes` and `documentChanges` (RIDX sends both) keeps only
    `documentChanges`, so every edit is counted once."""
    if isinstance(res, dict) and "documentChanges" in res and "changes" in res:
        return {k: v for k, v in res.items() if k != "changes"}
    return res


def def_span(d):
    """Span of a found `ridx def --json` answer, else None."""
    if not (d and d.get("found")):
        return None
    return (d["start_line"], d["start_col"], d["end_line"], d["end_col"])


def strictly_inside(inner, outer):
    (isl, isc, iel, iec), (osl, osc, oel, oec) = inner, outer
    return (osl, osc) <= (isl, isc) and (iel, iec) <= (oel, oec) and inner != outer


def brief(d):
    return {k: d.get(k) for k in ("kind", "path", "start_line", "start_col")} if d and d.get("found") else None


def classify_cursor_definition(cursor, def_before, def_after, enclosing_in_file):
    """The cursor lies inside a reference wide enough to enclose a definition; the reader's
    definition there used to be an item not at the cursor (the reference's target, reached through
    the enclosing reference: an item elsewhere, or the nameless crate-root module whose span
    encloses the reference itself) and is now an item whose span contains the cursor and lies
    inside that reference's span."""
    line, col = cursor
    spans = sorted({s for s, _, _ in enclosing_in_file if contains_pos(s, line, col)})
    sb, sa = def_span(def_before), def_span(def_after)
    before_at_cursor = sb is not None and contains_pos(sb, line, col) and not any(strictly_inside(s, sb) for s in spans)
    after_at_cursor = sa is not None and contains_pos(sa, line, col)
    after_inside = sa is not None and any(strictly_inside(sa, s) for s in spans)
    targets = sorted({name for s, name, _ in enclosing_in_file if contains_pos(s, line, col)})
    before_name = (def_before or {}).get("path", "").rsplit("::", 1)[-1].split("<", 1)[0] if sb else None
    if sb is None:
        before_kind = "none"
    elif def_before.get("kind") == "Mod" and not def_before.get("path"):
        before_kind = "crate_root_module"
    elif before_name in targets and not contains_pos(sb, line, col):
        before_kind = "enclosing_target"
    else:
        before_kind = "other"
    ok = bool(spans) and after_at_cursor and after_inside and not before_at_cursor and sb != sa
    return ok, {
        "enclosing_spans_at_cursor": [list(s) for s in spans], "enclosing_targets_at_cursor": targets,
        "definition_before": brief(def_before), "definition_after": brief(def_after), "before_kind": before_kind,
    }


def in_test_file(rel):
    """True when the path marks a test, bench or example target: a `tests/`, `test/`, `benches/`
    or `examples/` directory, or a file named `tests.rs`, `test.rs`, `*_test.rs`, `*_tests.rs` or
    `test_*.rs`. A `#[cfg(test)] mod tests` inside another file is not visible here; see
    `in_test_module`."""
    parts = rel.replace("\\", "/").split("/")
    name = parts[-1]
    return (
        any(p in TEST_DIRS for p in parts[:-1])
        or name in ("tests.rs", "test.rs")
        or name.endswith(("_test.rs", "_tests.rs"))
        or name.startswith("test_")
    )


def in_test_module(path):
    """True when a definition path has a `tests` or `test` module segment."""
    return any(seg in ("tests", "test") for seg in (path or "").split("::")[:-1])
