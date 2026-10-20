# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""What stands under a cursor and around it: the token and its class, `::` qualification, macro
invocations and `macro_rules!` bodies, and whether a position is a binding site. Shared by the gap
rules and the gap driver."""

import re

EXIT_LOOP_KEYWORDS = {"fn", "return", "?", "await", "loop", "while", "for", "break", "continue"}
KEYWORDS = {
    "as", "async", "const", "crate", "dyn", "else", "enum", "extern", "false", "if", "impl", "in", "let",
    "match", "mod", "move", "mut", "pub", "ref", "self", "Self", "static", "struct", "super", "trait", "true",
    "type", "unsafe", "use", "where", "union", "macro_rules",
}
MACRO_CALL_RE = re.compile(r"[A-Za-z_][A-Za-z0-9_]*!\s*[\(\[\{]")
WORD_RE = re.compile(r"[A-Za-z_][A-Za-z0-9_]*")
NUMBER_RE = re.compile(r"[0-9][0-9A-Za-z_.]*")


def token_at(line_text, col_1based):
    """The token under the cursor: a word, `?`, a lifetime (`'a`), a string or char literal (as
    `\"...\"` / `'.'`), a number, or the single character there."""
    i = max(0, col_1based - 1)
    if i >= len(line_text):
        return ""
    if (line_text[:i].count('"') - line_text[:i].count('\\"')) % 2 == 1:
        return '"..."'
    for m in WORD_RE.finditer(line_text):
        if m.start() <= i < m.end():
            word = m.group(0)
            if m.start() > 0 and line_text[m.start() - 1] == "'":
                return "'.'" if line_text[m.end():m.end() + 1] == "'" else "'" + word
            return word
    for m in NUMBER_RE.finditer(line_text):
        if m.start() <= i < m.end():
            return m.group(0)
    ch = line_text[i]
    if ch == "'" and i + 2 < len(line_text) and line_text[i + 2] == "'":
        return "'.'"
    if ch == "'":
        m = WORD_RE.match(line_text, i + 1)
        return "'" + m.group(0) if m else "'"
    return ch


def token_class(token):
    """`exit_loop`, `keyword`, `literal`, `lifetime`, `punct` or `ident`."""
    if token in EXIT_LOOP_KEYWORDS:
        return "exit_loop"
    if token in KEYWORDS:
        return "keyword"
    if token in ('"..."', "'.'") or token[:1].isdigit():
        return "literal"
    if token.startswith("'"):
        return "lifetime"
    if not WORD_RE.fullmatch(token or " "):
        return "punct"
    return "ident"


def qualified(line_text, col_1based):
    """True when `::` stands right before the token under the cursor."""
    m = next((m for m in WORD_RE.finditer(line_text) if m.start() <= col_1based - 1 < m.end()), None)
    return bool(m) and line_text[:m.start()].rstrip().endswith("::")


def in_macro_rules(lines, idx):
    """True when line `idx` is inside the braces of a `macro_rules!` item above it."""
    depth = None
    for text in lines[:idx]:
        if depth is None:
            if "macro_rules!" in text:
                depth = text.count("{") - text.count("}")
                if depth <= 0 and "{" in text:
                    depth = None
        else:
            depth += text.count("{") - text.count("}")
            if depth <= 0:
                depth = None
    return depth is not None


def in_macro_call(line_text, col_1based):
    """True when a macro invocation opens on the line before the cursor."""
    return any(m.end() <= col_1based for m in MACRO_CALL_RE.finditer(line_text))


def block_open_at(lines, start, idx, opener, closer):
    """True when the bracket block opened on line `start` is still open at line `idx`: the depth
    counted from `start` never returns to zero before `idx`."""
    depth = 0
    for text in lines[start:idx]:
        depth += text.count(opener) - text.count(closer)
        if depth <= 0:
            return False
    return depth > 0


def in_macro_block(lines, idx, lookback=400):
    """True when line `idx` is inside a multi-line macro invocation opened on an earlier line: an
    earlier line ending in `name!(`, `name![` or `name!{` whose block is still open at `idx`."""
    for j in range(idx - 1, max(-1, idx - lookback), -1):
        m = MACRO_CALL_RE.search(lines[j])
        if not m or lines[j].rstrip()[-1:] not in "([{":
            continue
        opener = lines[j].rstrip()[-1]
        if block_open_at(lines, j, idx, opener, {"(": ")", "[": "]", "{": "}"}[opener]):
            return True
    return False


def is_binding_site(line_text, col_1based, token):
    """Whether the token at the column is bound there: a `let` pattern, a function or closure
    parameter, a match arm pattern or a `for` loop variable."""
    i = max(0, col_1based - 1)
    before, stripped = line_text[:i], line_text.lstrip()
    if stripped.startswith(("let ", "let mut ")) or " let " in before:
        return "=" not in before.split("let", 1)[-1]
    if stripped.startswith("for ") and " in " in line_text[i:]:
        return True
    if "=>" in line_text and "=>" not in before:
        return True
    if before.count("|") % 2 == 1:
        return True
    if "fn " in before and "(" in before:
        return bool(re.match(r"\s*:", line_text[i + len(token):]))
    if re.match(r"(mut\s+)?" + re.escape(token) + r"\s*:", stripped) and not stripped.startswith("pub"):
        return True
    return False


def in_attribute_block(lines, idx, lookback=40):
    """True when line `idx` is inside a multi-line `#[...]` attribute opened on an earlier line."""
    for j in range(idx - 1, max(-1, idx - lookback), -1):
        if lines[j].lstrip().startswith("#[") and block_open_at(lines, j, idx, "[", "]"):
            return True
    return False


def in_impl_header(lines, idx, lookback=40):
    """True when line `idx` continues an `impl<...>` header opened on an earlier line: the generics
    opened there are still open at the line (more `<` than `>`), and no `{` closed the header."""
    for j in range(idx - 1, max(-1, idx - lookback), -1):
        text = lines[j].lstrip()
        if text.startswith(("impl<", "impl <", "unsafe impl<")):
            between = "\n".join(lines[j:idx])
            return "{" not in between and between.count("<") > between.count(">")
        if text.startswith(("fn ", "pub ", "struct ", "enum ", "}", "impl ")):
            return False
    return False


def derive_above(lines, idx, lookback=40):
    """True when the attribute and doc lines directly above line `idx` contain a `derive`."""
    j = idx - 1
    while j >= max(0, idx - lookback):
        stripped = lines[j].strip()
        if not stripped or stripped.startswith(("///", "//")):
            j -= 1
            continue
        if stripped.startswith("#[") or stripped.endswith(")]") or in_attribute_block(lines, j):
            if "derive" in stripped:
                return True
            j -= 1
            continue
        return False
    return False
