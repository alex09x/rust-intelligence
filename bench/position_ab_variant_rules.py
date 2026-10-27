# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Classes of the rows that differ between two RIDX binaries that differ by what a use of an enum
variant resolves to: before, the struct or import of the same name in the file of the variant;
after, the variant the reference records.

  v1  definition: the answer is one location, the declaration of a variant named like the token
      at the cursor (a line of an enum body that begins with that name);
  v2  hover: the answer names a variant of the token's name, and the definition at the cursor
      (after) is the declaration of that variant;
  v3  references, documentHighlight, rename: the definition at the cursor (after) is a variant
      declaration; every location gained resolves to it (`ridx def`) or is written by a derive,
      attribute or macro; no location lost resolves to it (they belong to the item of the same
      name);
  v5  definition: the cursor token is not the name of either answer (a token inside a macro
      invocation whose span the index maps onto an item) and the two answers are items of one
      name, the one after a variant declaration: one wrong target swapped for another;
  v4  signatureHelp: nothing after; the callee in front of the open parenthesis around the cursor
      is a variant (its definition after is a declaration) and the label before is a path ending
      in its name.
"""

import re
from collections import Counter

from position_ab_item_position_rules import is_generated_span, same_definition

CLASS_DEFINITION = "v1_definition_of_a_variant_use_was_another_item"
CLASS_HOVER = "v2_hover_of_a_variant_use_named_another_item"
CLASS_LOCATIONS = "v3_locations_of_a_variant_were_those_of_another_item"
CLASS_SWAPPED = "v5_definition_inside_a_macro_swapped_between_items_of_one_name"
CLASS_SIGNATURE = "v4_signature_help_of_a_variant_was_a_signature_of_another_item"

ENUM_HEADER = re.compile(r"^\s*(pub(\([^)]*\))?\s+)?enum\s+\w+")
IDENT = re.compile(r"[A-Za-z_][A-Za-z0-9_]*")


def _indent(line):
    return len(line) - len(line.lstrip())


MACRO_OPEN = re.compile(r"\w+!\s*[({\[]")


def in_macro_invocation(lines, line1, limit=40):
    """Whether line `line1` (1-based) lies inside the brackets of a macro invocation that began
    on an earlier line (`name!(` with its bracket still open)."""
    if not lines or not 0 < line1 <= len(lines):
        return False
    for n in range(line1 - 1, max(0, line1 - 1 - limit), -1):
        m = MACRO_OPEN.search(lines[n - 1])
        if m:
            text = lines[n - 1][m.end() - 1:] + "\n" + "\n".join(lines[n:line1 - 1])
            if sum(text.count(c) for c in "([{") - sum(text.count(c) for c in ")]}") > 0:
                return True
    return False


def is_variant_declaration(lines, line1, name):
    """Whether line `line1` (1-based) of `lines` declares a variant `name`: it begins with the
    name, after attributes, and the first less indented line above it is an enum header."""
    if not lines or not 0 < line1 <= len(lines):
        return False
    text = lines[line1 - 1]
    if not re.match(rf"^\s*(#\[[^\]]*\]\s*)*{re.escape(name)}\b(?!::)\s*(\(|\{{|,|=|//|$)", text):
        return False
    here = _indent(text)
    if in_macro_invocation(lines, line1):
        return True  # a variant declared through the arguments of a macro (`declare_enum!(A => [..])`)
    for above in reversed(lines[: line1 - 1]):
        stripped = above.strip()
        if not stripped or stripped.startswith(("//", "#[", "#![")):
            continue
        if _indent(above) < here:
            return bool(ENUM_HEADER.match(above))
    return False


def cursor_token(lines, line1, char):
    """The identifier that contains the 0-based character `char` of line `line1`, or None."""
    if not lines or not 0 < line1 <= len(lines):
        return None
    for m in IDENT.finditer(lines[line1 - 1]):
        if m.start() <= char <= m.end():
            return m.group(0)
    return None


def callee_before_paren(lines, line1, char, limit=60):
    """(identifier, 1-based line, 0-based character of its start) of the call or pattern whose
    parentheses enclose the 0-based character `char` of line `line1`, looking back up to
    `limit` lines, or None."""
    if not lines or not 0 < line1 <= len(lines):
        return None
    depth = 0
    for n in range(line1, max(0, line1 - limit), -1):
        text = lines[n - 1]
        start = min(char, len(text)) if n == line1 else len(text)
        for i in range(start - 1, -1, -1):
            if text[i] == ")":
                depth += 1
            elif text[i] == "(":
                if depth == 0:
                    head = text[:i].rstrip()
                    found = IDENT.findall(head)
                    ends = [m for m in IDENT.finditer(head)]
                    return (ends[-1].group(0), n, ends[-1].start()) if found and ends[-1].end() == len(head) else None
                depth -= 1
    return None


def declares_at(lines_of, loc, name):
    """Whether the location `loc` = (path, (line, col, ..)) is the name of a variant declaration."""
    path, span = loc
    lines = lines_of(path) if path else None
    if not lines or not 0 < span[0] <= len(lines):
        return False
    text = lines[span[0] - 1]
    return text[span[1] - 1 : span[1] - 1 + len(name)] == name and is_variant_declaration(lines, span[0], name)


def classify_definition(before, after, lines_of, token, locations_of):
    """(class or None, evidence) for definition rows."""
    b, a = sorted(set(locations_of(before, lines_of)), key=str), sorted(set(locations_of(after, lines_of)), key=str)
    ev = {"token": token, "before": [[p, list(s)] for p, s in b], "after": [[p, list(s)] for p, s in a]}
    if bool(token) and len(a) == 1 and a != b and declares_at(lines_of, a[0], token):
        return CLASS_DEFINITION, ev
    name = name_at(lines_of, a[0]) if len(a) == 1 else None
    swapped = (name and token != name and len(b) == 1 and a != b and name_at(lines_of, b[0]) == name
               and declares_at(lines_of, a[0], name) and not declares_at(lines_of, b[0], name))
    ev["answer_name"] = name
    return (CLASS_SWAPPED if swapped else None), ev


def name_at(lines_of, loc):
    """The identifier that starts at the location `loc` = (path, (line, col, ..))."""
    path, span = loc
    lines = lines_of(path) if path else None
    if not lines or not 0 < span[0] <= len(lines):
        return None
    m = IDENT.match(lines[span[0] - 1][span[1] - 1:])
    return m.group(0) if m else None


def classify_hover(before_text, after_text, token, definition_is_variant):
    ev = {"token": token, "before": (before_text or "")[:80], "after": (after_text or "")[:80], "definition_is_variant": definition_is_variant}
    ok = bool(token) and definition_is_variant and before_text != after_text and (after_text or "").startswith(f"```rust\nvariant {token}\n")
    return (CLASS_HOVER if ok else None), ev


def classify_signature(before_label, after_empty, callee, callee_is_variant):
    ev = {"callee": callee, "before_label": before_label, "after_empty": after_empty, "callee_is_variant": callee_is_variant}
    ok = bool(callee) and callee_is_variant and after_empty and bool(before_label) and before_label.split("::")[-1] == callee
    return (CLASS_SIGNATURE if ok else None), ev


def classify_variant_locations(before, after, cursor_file, lines_of, locations_of, def_of_col, cursor_def, definition_is_variant):
    """(class or None, evidence) for references, documentHighlight and rename rows. `def_of_col(path,
    line, col1)` is the definition at a 1-based character position, `cursor_def` that at the cursor."""
    locs = lambda res: Counter((p or cursor_file, span) for p, span in locations_of(res, lines_of))
    cb, ca = locs(before), locs(after)
    removed, added = cb - ca, ca - cb
    ev = {"cursor_definition": list(cursor_def) if cursor_def else None, "definition_is_variant": definition_is_variant,
          "removed_count": sum(removed.values()), "added_count": sum(added.values()), "added_not_resolving": [], "removed_resolving": [], "added_generated": 0}
    if not (removed or added) or not cursor_def or not definition_is_variant:
        return None, ev
    generated = 0
    for (path, span), _ in sorted(added.items(), key=str):
        if not same_definition(def_of_col(path, span[0], span[1]), cursor_def):
            lines = (lines_of(path) if lines_of else None) or []
            if same_definition((path, span[0]), cursor_def) or in_macro_invocation(lines, span[0]) or is_generated_span(lines, span):
                generated += 1
            else:
                ev["added_not_resolving"].append([path, list(span)])
    ev["added_generated"] = generated
    for (path, span), _ in sorted(removed.items(), key=str):
        if same_definition(def_of_col(path, span[0], span[1]), cursor_def):
            ev["removed_resolving"].append([path, list(span)])
    ok = not ev["added_not_resolving"] and not ev["removed_resolving"]
    return (CLASS_LOCATIONS if ok else None), ev
