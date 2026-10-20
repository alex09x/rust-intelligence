# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Pure helpers of `position_ab.py`: scenarios rebuilt from a published positions file, the
matrix outcome rule, order-insensitive digests, the cargo target kind of a file and the kind of
the source token under the cursor."""

import hashlib
import json
import re
from pathlib import Path

from matrix_categories import LSP_SPECS
from scan import DECL_RE, METHOD_CALL_RE, PATH_IDENT_RE, TYPE_USAGE_RE

# Request methods of the matrix whose params `build_params` derives from a positions record
# (file, line, character, symbol) or from nothing at all; document synchronisation (domain 2)
# needs the harness's own edit protocol and stays out.
REQUEST_KEYS = tuple(key for _, key, _, domain, kind in LSP_SPECS if kind == "request" and domain != 2)
# Methods whose answers come back in an order that varies between processes or even between
# requests: rust-analyzer's completion items, RIDX's incoming calls and rename edits (observed
# on serde at 8cb4341; the sorted answers are equal). Their digests sort every list first.
SET_DIGEST_KEYS = frozenset({"completion", "incoming_calls", "rename"})
CFG_TEST_RE = re.compile(r"^\s*#\[cfg\(test\)\]")
MOD_RE = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+[A-Za-z_][A-Za-z0-9_]*\s*\{")
IMPL_RE = re.compile(r"^\s*(?:unsafe\s+)?impl\b")
MACRO_RE = re.compile(r"\b([A-Za-z_][A-Za-z0-9_]*)!\s*[\(\[\{]")


def scenario_from_record(key, rec, repo_dir):
    """Rebuilds the scenario dict the sampler produced for `rec`, so `build_params` yields the
    request of the matrix: absolute `location`, `target` (the symbol), `query` for
    workspace/symbol, `new_name` for rename and `prev_id` for semantic tokens deltas."""
    name = rec.get("name", "")
    sc = {"name": name}
    rel = rec.get("file") or ""
    if rel:
        f = str(Path(repo_dir) / rel)
        sc["file"] = f
        sc["location"] = [f, int(rec.get("line") or 0), int(rec.get("character") or 0)]
    if rec.get("symbol") is not None:
        sc["target"] = rec["symbol"]
    suffix = name[len(key) + 1:] if name.startswith(key + "_") else ""
    index, _, rest = suffix.partition("_")
    if key == "workspace_symbol":
        sc["query"] = name.partition("ws_sym_")[2].partition("_")[2] if name.startswith("ws_sym_") else rest
    elif key in ("prepare_rename", "rename", "linked_editing_range", "code_action") and rest:
        sc["new_name"] = f"{rest}_renamed_{index}"
    elif key == "semantic_tokens_delta" and index:
        sc["prev_id"] = f"id_{index}"
    return sc


def classify_outcome(key, res, err):
    """The matrix rule (bench/matrix_execute.py): an error is (c), an absent or empty answer is
    (b), anything else is (a). Returns the letter and the error code or text, if any."""
    if err is not None:
        code = err.get("code") if isinstance(err, dict) else err
        return "c", str(code)
    empty = (res is None or res == [] or res == "" or (isinstance(res, dict) and not res)
             or (key == "hover" and isinstance(res, dict) and not res.get("contents")))
    return ("b" if empty else "a"), None


def sorted_form(value):
    """`value` with every list sorted by the canonical JSON of its items, recursively."""
    if isinstance(value, list):
        return sorted((sorted_form(v) for v in value), key=lambda v: json.dumps(v, sort_keys=True, separators=(",", ":")))
    if isinstance(value, dict):
        return {k: sorted_form(v) for k, v in value.items()}
    return value


def canonical(key, res):
    """The answer as compared between servers: every list sorted for the methods in
    SET_DIGEST_KEYS, everything else as returned."""
    return sorted_form(res) if key in SET_DIGEST_KEYS else res


def digest(key, res):
    """First 16 hex digits of the sha256 of the canonical answer."""
    text = json.dumps(canonical(key, res), sort_keys=True, separators=(",", ":"))
    return hashlib.sha256(text.encode("utf-8")).hexdigest()[:16]


def load_target_entries(repo_dir):
    """(relative entry path, kind) of every cargo target recorded by `ridx index` in the active
    variant's target_kinds.json, or an empty list when the index has none."""
    repo = Path(repo_dir).resolve()
    manifest = repo / "target" / "ridx" / "ridx_manifest.json"
    try:
        data = json.loads(manifest.read_text(encoding="utf-8"))
        kinds_path = manifest.parent / data.get("variant_dir", "") / "target_kinds.json"
        kinds = json.loads(kinds_path.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return []
    entries = []
    for entry, kind in kinds.items():
        try:
            rel = str(Path(entry).resolve().relative_to(repo))
        except ValueError:
            continue
        entries.append((rel, str(kind)))
    return sorted(set(entries))


def target_kind_of(rel, entries):
    """Kind of the cargo target whose entry file is `rel`, else of the target whose source tree
    most plausibly contains it: the longest entry directory that prefixes the path, a lib, bin or
    proc-macro entry winning ties over tests, examples and benches. A file under `<entry stem>/`
    of a test, example or bench belongs to that target. Returns "none" outside every target. This
    is a path heuristic: the shard file tables are the authority."""
    rel = rel.replace("\\", "/")
    best = None
    for entry, kind in entries:
        if entry == rel:
            return kind
        directory = entry.rsplit("/", 1)[0] + "/" if "/" in entry else ""
        stem = entry[:-3] + "/" if entry.endswith(".rs") else None
        if stem and rel.startswith(stem):
            candidate = (len(stem) + 1, 1, kind)
        elif rel.startswith(directory):
            weight = 1 if kind in ("lib", "bin", "proc-macro", "dylib") else 0
            candidate = (len(directory), weight, kind)
        else:
            continue
        if best is None or candidate[:2] > best[:2]:
            best = candidate
    return best[2] if best else "none"


def cursor_kind(lines, line_1based, character_1based, symbol):
    """What the sampler's token is on its line: a declaration (`decl:fn`, `decl:struct`, ...), an
    `impl` header, a method call, a path segment, a type annotation, a macro name, a `use` or
    `let` line, else `ident`; plus whether the line sits in a `#[cfg(test)]` module or carries
    the attribute itself."""
    idx = line_1based - 1
    if idx < 0 or idx >= len(lines):
        return {"cursor_kind": "out_of_file", "cfg_test": False}
    line = lines[idx]
    col = max(0, character_1based - 1)
    kind = "ident"
    m = DECL_RE.match(line)
    if m and (symbol is None or m.group(2) == symbol):
        kind = f"decl:{m.group(1)}"
    elif IMPL_RE.match(line):
        kind = "impl"
    elif line.lstrip().startswith("use "):
        kind = "use"
    elif line.lstrip().startswith("let "):
        kind = "let"
    else:
        for regex, label in ((METHOD_CALL_RE, "method_call"), (PATH_IDENT_RE, "path"),
                             (TYPE_USAGE_RE, "type_usage"), (MACRO_RE, "macro")):
            hit = next((mm for mm in regex.finditer(line) if mm.start(1) <= col <= mm.end(1)
                        and (symbol is None or mm.group(1) == symbol)), None)
            if hit:
                kind = label
                break
    return {"cursor_kind": kind, "cfg_test": in_cfg_test(lines, idx)}


def in_cfg_test(lines, idx):
    """True when line `idx` is inside a `#[cfg(test)] mod ... { ... }` block (brace counting
    from the module line) or when the attribute stands directly above it."""
    if idx > 0 and CFG_TEST_RE.match(lines[idx - 1]):
        return True
    depth = None
    for i, text in enumerate(lines[: idx + 1]):
        if depth is None:
            if i > 0 and CFG_TEST_RE.match(lines[i - 1]) and MOD_RE.match(text):
                depth = text.count("{") - text.count("}")
                if depth <= 0:
                    depth = None
        else:
            depth += text.count("{") - text.count("}")
            if depth <= 0:
                depth = None
    return depth is not None and depth > 0
