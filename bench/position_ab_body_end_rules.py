# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Rules of `position_ab_body_end.py`: a model of the call-hierarchy body-end scanner
(`find_function_body_end_line`) in both versions, and checks over call hierarchy answers of two
servers. Lines are 1-based; LSP ranges are 0-based."""

CLASS_INCOMING = "f1_call_sites_follow_the_corrected_body_end_of_their_callers"
CLASS_OUTGOING = "f2_callees_follow_the_corrected_body_end_of_the_function"


def opens_char_literal(line, i):
    """The quote at `line[i]` opens a char literal: an escape follows, or the character after the
    next one is the closing quote. Otherwise it is a lifetime or a label."""
    nxt = line[i + 1] if i + 1 < len(line) else None
    nxt2 = line[i + 2] if i + 2 < len(line) else None
    return nxt == "\\" or (nxt is not None and nxt2 == "'")


def body_end(lines, start_line, lifetime_aware):
    """Model of `find_function_body_end_line`. With `lifetime_aware` False every quote outside a
    string toggles the char-literal state (the scanner before eb9ba2e); with True a quote opens a
    char literal only when `opens_char_literal` says so."""
    depth = 0; paren = 0; seen_open = False; in_block = False
    for idx in range(max(start_line - 1, 0), len(lines)):
        line = lines[idx]; i = 0; in_s = False; in_c = False; esc = False
        while i < len(line):
            ch = line[i]; i += 1
            if in_block:
                if ch == "*" and i < len(line) and line[i] == "/":
                    i += 1; in_block = False
                continue
            if esc:
                esc = False; continue
            if ch == "\\" and (in_s or in_c):
                esc = True; continue
            if ch == '"' and not in_c:
                in_s = not in_s; continue
            if ch == "'" and not in_s:
                if lifetime_aware:
                    if in_c:
                        in_c = False
                    elif opens_char_literal(line, i - 1):
                        in_c = True
                else:
                    in_c = not in_c
                continue
            if in_s or in_c:
                continue
            if ch == "/" and i < len(line) and line[i] == "/":
                break
            if ch == "/" and i < len(line) and line[i] == "*":
                i += 1; in_block = True; continue
            if not seen_open:
                if ch in "([":
                    paren += 1
                elif ch in ")]" and paren > 0:
                    paren -= 1
                elif ch == ";" and paren == 0:
                    return idx + 1
            if ch == "{":
                depth += 1; seen_open = True
            elif ch == "}":
                depth -= 1
                if seen_open and depth <= 0:
                    return idx + 1
    return len(lines)


def call_sites(res, side):
    """{(path, item start line 1-based, name): (sorted 1-based lines of its fromRanges, item range
    end line 1-based)} for the call hierarchy items under `side` (`from` for incomingCalls, `to`
    for outgoingCalls). A caller item's range ends at the body end the scanner found."""
    out = {}
    for item in res or []:
        node = item.get(side) or {}
        uri = node.get("uri", "")
        path = uri[7:] if uri.startswith("file://") else uri
        rng = node.get("range") or {}
        key = (path, (rng.get("start") or {}).get("line", -1) + 1, node.get("name", ""))
        sites, _ = out.get(key, (set(), None))
        sites.update(((r.get("start") or {}).get("line", -1) + 1) for r in item.get("fromRanges") or [])
        out[key] = (sites, (rng.get("end") or {}).get("line", -1) + 1)
    return out


def moved_by_body_end(added, removed, old_end, new_end):
    """Every added call-site line lies in the span the corrected scan newly covers (old_end,
    new_end], every removed one in the span the old scan wrongly covered (new_end, old_end]."""
    return all(old_end < l <= new_end for l in added) and all(new_end < l <= old_end for l in removed)


def classify_incoming(before, after, lines_of):
    """incomingCalls: for every caller that changed, the call sites gained or lost follow the move
    of that caller's body end from the old scan to the corrected one, and the caller item's range
    ends at the old body end before and at the corrected one after."""
    cb, ca = call_sites(before, "from"), call_sites(after, "from")
    checked = []
    for key in sorted(set(cb) | set(ca)):
        path, line, name = key
        sites_b, end_b = cb.get(key, (set(), None))
        sites_a, end_a = ca.get(key, (set(), None))
        if sites_b == sites_a and end_b == end_a:
            continue
        lines = lines_of(path) or []
        old_end = body_end(lines, line, False) if lines else None
        new_end = body_end(lines, line, True) if lines else None
        added, removed = sorted(sites_a - sites_b), sorted(sites_b - sites_a)
        ok = (bool(lines) and moved_by_body_end(added, removed, old_end, new_end)
              and (end_b is None or end_b == old_end) and (end_a is None or end_a == new_end))
        checked.append({"caller": [path, line, name], "old_end": old_end, "new_end": new_end, "range_end_before": end_b,
                        "range_end_after": end_a, "added_sites": added, "removed_sites": removed, "ok": ok})
    ok = bool(checked) and all(c["ok"] for c in checked)
    return ok, {"callers": checked, "removed_count": sum(len(c["removed_sites"]) for c in checked),
                "added_count": sum(len(c["added_sites"]) for c in checked)}


def classify_outgoing(before, after, lines, item_line):
    """outgoingCalls on the function at `item_line`: callees gained have their call sites in the
    span the corrected scan newly covers, callees lost in the span the old scan wrongly covered."""
    cb = {k: v[0] for k, v in call_sites(before, "to").items()}
    ca = {k: v[0] for k, v in call_sites(after, "to").items()}
    old_end = body_end(lines, item_line, False) if lines else None
    new_end = body_end(lines, item_line, True) if lines else None
    added = sorted(l for k in ca for l in ca[k] - cb.get(k, set()))
    removed = sorted(l for k in cb for l in cb[k] - ca.get(k, set()))
    ok = bool(lines) and bool(added or removed) and moved_by_body_end(added, removed, old_end, new_end)
    return ok, {"item_line": item_line, "old_end": old_end, "new_end": new_end, "added_sites": added, "removed_sites": removed,
                "removed_count": len(removed), "added_count": len(added)}
