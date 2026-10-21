# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Rules of `position_ab_name_span.py`: pure functions over the answers of the two servers and the
set of self-containing references (`ref_spans` rows whose `self` column is `contains`).

Spans are 1-based, end-exclusive `(start line, start col, end line, end col)` in characters, as the
shards store them; LSP ranges are converted with the file text when it is readable.
"""
from position_ab_content_rules import uri_path, utf16_to_char

CLASS_LIST = "s1_self_containing_reference_ranges_removed"
CLASS_SINGLE = "s2_definition_answered_through_self_containing_reference"
CLASS_LENS = "s3_code_lens_counts_without_self_containing_references"
CLASS_FALLBACK = "s4_position_on_no_name_answered_through_enclosing_reference"
CLASS_SELF_CALL = "s5_outgoing_call_to_the_enclosing_test_function_itself"
CLASS_INVOCATION = "s6_innermost_reference_is_now_a_macro_invocation_span"
CALL_KEYS = frozenset({"outgoing_calls"})
SINGLE_KEYS = frozenset({"definition", "declaration", "type_definition"})
LENS_KEYS = frozenset({"code_lens"})


def load_invocation_refs(paths, repo):
    """Kept multi-line macro references (`self` not `contains`), keyed by absolute file path:
    (span, last segment of the target path). Inside a `macro_rules!` invocation every reference of
    the expansion carries the invocation's span; once the harness references are skipped, one of
    them is the innermost reference at a cursor in the invocation."""
    out = {}
    for path in paths:
        with open(path, encoding="utf-8", errors="replace") as fh:
            for p in (line.rstrip("\n").split("\t") for line in fh):
                if len(p) < 12 or p[0] != "REF" or p[5] != "macro" or int(p[6]) <= 1 or p[10] == "contains":
                    continue
                stored = p[2]
                abs_path = stored if stored.startswith("/") else f"{repo.rstrip('/')}/{stored}"
                start, end = p[3].split("-")
                sl, sc = (int(x) for x in start.split(":"))
                el, ec = (int(x) for x in end.split(":"))
                out.setdefault(abs_path, []).append(((sl, sc, el, ec), p[11].rsplit("::", 1)[-1].split("<", 1)[0]))
    return out


def classify_invocation(cursor, def_before, def_after, contains_in_file, invocation_refs_in_file):
    """The before definition resolved through a skipped reference; the after definition is the
    target of a kept multi-line macro reference whose span contains the cursor (an invocation
    span), not the item at the cursor."""
    line, col = cursor
    spans = sorted(s for s in contains_in_file if contains_pos(s, line, col))
    before_inside = bool(def_before and def_before.get("found")) and any(
        contains_pos(s, def_before["start_line"], def_before["start_col"]) for s in spans
    )
    after_name = (def_after or {}).get("path", "").rsplit("::", 1)[-1].split("<", 1)[0] if def_after and def_after.get("found") else None
    hits = sorted({span for span, name in invocation_refs_in_file if name == after_name and contains_pos(span, line, col)})
    ok = bool(spans) and before_inside and after_name is not None and bool(hits)
    brief = lambda d: {k: d.get(k) for k in ("kind", "path", "start_line", "start_col")} if d and d.get("found") else None  # noqa: E731
    return ok, {
        "self_containing_spans_at_cursor": [list(s) for s in spans], "definition_before": brief(def_before),
        "definition_after": brief(def_after), "invocation_spans_of_the_after_target": [list(h) for h in hits],
    }


def load_contains(paths, repo):
    """Spans of the references the fix skips, keyed by absolute file path: the `REF` rows of
    `ref_spans` outputs whose `self` column is `contains`. Relative stored paths are workspace
    paths (the census is run with `--root <repo>`)."""
    out = {}
    for path in paths:
        with open(path, encoding="utf-8", errors="replace") as fh:
            for line in fh:
                p = line.rstrip("\n").split("\t")
                if len(p) < 12 or p[0] != "REF" or p[10] != "contains":
                    continue
                stored = p[2]
                abs_path = stored if stored.startswith("/") else f"{repo.rstrip('/')}/{stored}"
                start, end = p[3].split("-")
                sl, sc = (int(x) for x in start.split(":"))
                el, ec = (int(x) for x in end.split(":"))
                out.setdefault(abs_path, set()).add((sl, sc, el, ec))
    return out


def span_of(rng, lines):
    """1-based character span of an LSP range; without the file text the UTF-16 columns are used."""
    s, e = rng["start"], rng["end"]

    def col(line, character):
        if lines is not None and 0 <= line < len(lines):
            return utf16_to_char(lines[line], character) + 1
        return character + 1

    return (s["line"] + 1, col(s["line"], s["character"]), e["line"] + 1, col(e["line"], e["character"]))


def locations_of(res, lines_of):
    """(absolute path, span) of every location-like object in an answer: `uri`/`targetUri` with a
    `range`/`targetSelectionRange`/`targetRange`, call hierarchy items with their `fromRanges`,
    workspace edits (`changes`, `documentChanges`), and bare ranges (documentHighlight) which
    carry no uri and get `None` as path."""
    out = []

    def walk(x, uri=None):
        if isinstance(x, dict):
            u = x.get("uri") or x.get("targetUri") or uri
            if "newText" in x and "range" in x:
                out.append((uri_path(u) if u else None, span_of(x["range"], lines_of(uri_path(u)) if u else None)))
                return
            if "range" in x and isinstance(x["range"], dict) and "start" in x["range"]:
                path = uri_path(u) if u else None
                out.append((path, span_of(x["range"], lines_of(path) if path else None)))
            for key in ("targetRange", "targetSelectionRange"):
                if key in x and u:
                    out.append((uri_path(u), span_of(x[key], lines_of(uri_path(u)))))
            for key in ("fromRanges",):
                for r in x.get(key) or []:
                    frm = x.get("from", {}).get("uri") if isinstance(x.get("from"), dict) else None
                    path = uri_path(frm) if frm else None
                    out.append((path, span_of(r, lines_of(path) if path else None)))
            if isinstance(x.get("changes"), dict):
                for cu, edits in x["changes"].items():
                    for ed in edits:
                        walk(ed, cu)
            for k, v in x.items():
                if k not in ("range", "targetRange", "targetSelectionRange", "fromRanges", "changes"):
                    walk(v, x.get("textDocument", {}).get("uri") if isinstance(x.get("textDocument"), dict) else u)
        elif isinstance(x, list):
            for v in x:
                walk(v, uri)

    walk(res)
    return out


def contains_pos(span, line, col):
    sl, sc, el, ec = span
    if line < sl or line > el:
        return False
    if line == sl and col < sc:
        return False
    return not (line == el and col >= ec)


def is_test_attribute(lines, span):
    """True when the span covers exactly a `#[..test..]` attribute on one line: the definition of
    the constant the test harness generates, listed as a declaration while its reference was."""
    sl, sc, el, ec = span
    if lines is None or sl != el or not (1 <= sl <= len(lines)):
        return False
    text = lines[sl - 1]
    covered = text[sc - 1:ec - 1]
    return covered.startswith("#[") and covered.endswith("]") and "test" in covered and text.strip() == covered


def classify_list(removed, added, contains, cursor_path, lines_of=lambda path: None):
    """Every removed location is a self-containing reference (a bare range belongs to the cursor's
    file) or the test attribute that declares the generated constant, and nothing was added."""
    attrs = [loc for loc in removed if loc[1] not in contains.get(loc[0] or cursor_path, set())
             and is_test_attribute(lines_of(loc[0] or cursor_path), loc[1])]
    bad = [loc for loc in removed if loc[1] not in contains.get(loc[0] or cursor_path, set()) and loc not in attrs]
    ok = bool(removed) and not added and not bad
    nested = len(removed) == 2 and (
        contains_pos(removed[0][1], removed[1][1][0], removed[1][1][1]) or contains_pos(removed[1][1], removed[0][1][0], removed[0][1][1])
    )
    return ok, {
        "removed": [[p, list(s)] for p, s in removed], "added": [[p, list(s)] for p, s in added],
        "removed_count": len(removed), "added_count": len(added), "not_self_containing": [[p, list(s)] for p, s in bad],
        "test_attribute_declarations": [[p, list(s)] for p, s in attrs], "two_nested_ranges": nested,
    }


def classify_fallback(cursor, def_before, def_after, contains_in_file):
    """The cursor lies inside a self-containing reference of its file: the reader's `definition`
    there used to resolve through that reference (the before definition starts inside the span)
    and now resolves to the item at the cursor (a definition whose span contains the cursor), to
    the nameless crate-root module (the pre-existing fallback) or to nothing. `def_before` and
    `def_after` are `ridx def --json` answers: dicts with `found`, `kind`, `path`, `start_line`,
    `start_col`, `end_line`, `end_col`, or None."""
    line, col = cursor
    spans = sorted(s for s in contains_in_file if contains_pos(s, line, col))

    def start(d):
        return (d["start_line"], d["start_col"]) if d and d.get("found") else None

    before_inside = start(def_before) is not None and any(contains_pos(s, *start(def_before)) for s in spans)
    if not (def_after and def_after.get("found")):
        after_kind = "none"
    elif def_after.get("kind") == "Mod" and not def_after.get("path"):
        after_kind = "crate_root_module"
    elif contains_pos((def_after["start_line"], def_after["start_col"], def_after["end_line"], def_after["end_col"]), line, col):
        after_kind = "item_at_cursor"
    else:
        after_kind = "other"
    changed = start(def_after) != start(def_before) or (def_after or {}).get("path") != (def_before or {}).get("path")
    ok = bool(spans) and before_inside and changed and after_kind != "other"
    brief = lambda d: {k: d.get(k) for k in ("kind", "path", "start_line", "start_col")} if d and d.get("found") else None  # noqa: E731
    return ok, {
        "self_containing_spans_at_cursor": [list(s) for s in spans],
        "definition_before": brief(def_before), "definition_after": brief(def_after), "after_kind": after_kind,
    }


def classify_self_calls(removed, added, contains_in_file):
    """outgoingCalls on a test function: every removed callee is the function itself, reached from
    a call site inside its body through the enclosing reference (the `to` range and every
    `fromRange` lie inside a self-containing reference of the file), and nothing was added."""
    inside = lambda span: any(contains_pos(s, span[0], span[1]) for s in contains_in_file)  # noqa: E731
    bad = [loc for loc in removed if not inside(loc[1])]
    ok = bool(removed) and not added and not bad
    return ok, {
        "removed": [[p, list(s)] for p, s in removed], "added": [[p, list(s)] for p, s in added],
        "removed_count": len(removed), "added_count": len(added), "outside_self_containing": [[p, list(s)] for p, s in bad],
    }


def classify_single(cursor, before, after, contains_in_file):
    """The cursor and the before answer both lie inside one self-containing reference of the
    cursor's file, and the after answer is not that answer."""
    line, col = cursor
    spans = sorted(s for s in contains_in_file if contains_pos(s, line, col))
    hits = [s for s in spans if before is not None and contains_pos(s, before[1][0], before[1][1])]
    ok = bool(hits) and before is not None and after != before
    return ok, {
        "self_containing_spans_at_cursor": [list(s) for s in spans], "before": [before[0], list(before[1])] if before else None,
        "after": [after[0], list(after[1])] if after else None,
    }


def lens_pairs(res):
    """(range span key, title) of every lens, in order."""
    out = []
    for lens in res or []:
        rng = lens.get("range", {})
        key = (rng.get("start", {}).get("line"), rng.get("start", {}).get("character"))
        title = (lens.get("command") or {}).get("title", "")
        out.append((key, title))
    return out


def count_of(title):
    head = title.split(" ", 1)[0]
    return int(head) if head.isdigit() else None


def classify_lens(before, after, contains_in_file, lines):
    """Same lenses in the same order; every changed title is a reference count that fell by exactly
    the number of self-containing references of the file whose span contains the lens position."""
    pb, pa = lens_pairs(before), lens_pairs(after)
    if len(pb) != len(pa) or [k for k, _ in pb] != [k for k, _ in pa]:
        return False, {"before_lenses": len(pb), "after_lenses": len(pa), "same_positions": False}
    changes = []
    ok = True
    for (key, tb), (_, ta) in zip(pb, pa):
        if tb == ta:
            continue
        line, char = key
        col = utf16_to_char(lines[line], char) + 1 if lines is not None and 0 <= line < len(lines) else char + 1
        expected = sum(1 for s in contains_in_file if contains_pos(s, line + 1, col))
        nb, na = count_of(tb), count_of(ta)
        drop = nb - na if nb is not None and na is not None else None
        good = drop is not None and drop > 0 and drop == expected and tb.split(" ", 1)[1:] == ta.split(" ", 1)[1:]
        ok = ok and good
        changes.append({"line": line + 1, "before": tb, "after": ta, "self_containing_at_lens": expected, "ok": good})
    return ok and bool(changes), {"lenses": len(pb), "changed": changes}
