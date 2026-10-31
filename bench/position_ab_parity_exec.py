# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Parity-specific row preparation and request execution for `position_ab_parity.py`.

Handles:
1. `signature_help`: shifts symbol cursor positions into the call argument list right after `(` and `,`.
2. `inlay_hint`: sends `didOpen` and queries the full file line range, then `didClose`.
3. `code_lens_resolve`: chains `textDocument/codeLens` -> `codeLens/resolve` on the same server after `didOpen`.
4. `semantic_tokens_*`: attaches the server's `initialize` legend (`semantic_token_types`) and uses a 100-line range for `semantic_tokens_range`.
"""

from pathlib import Path
from matrix_categories import LSP_SPECS, build_params
from position_ab_classify import classify_outcome, target_kind_of

RPC_OF = {key: rpc for _, key, rpc, _, _ in LSP_SPECS}


def call_arg_positions(lines, line_1based, col_1based):
    """Finds 1-based `(line, col)` positions immediately after `(` and after each top-level `,`
    of the call starting at or after `(line_1based, col_1based)`."""
    idx = int(line_1based or 1) - 1
    if idx < 0 or idx >= len(lines):
        return [(line_1based, col_1based)]
    start_c = max(0, int(col_1based or 1) - 1)
    open_l, open_c, angle = None, None, 0
    for l_off in range(3):
        li = idx + l_off
        if li >= len(lines):
            break
        s = lines[li]
        c0 = start_c if l_off == 0 else 0
        for ci in range(c0, len(s)):
            ch = s[ci]
            if ch == "<":
                angle += 1
            elif ch == ">" and angle > 0:
                angle -= 1
            elif ch == "(" and angle == 0:
                open_l, open_c = li, ci
                break
        if open_l is not None:
            break
    if open_l is None:
        return [(line_1based, col_1based)]

    positions = [(open_l + 1, open_c + 2)]
    depth_p, depth_b, depth_c = 1, 0, 0
    in_str, esc = False, False
    for l_off in range(6):
        li = open_l + l_off
        if li >= len(lines):
            break
        s = lines[li]
        c0 = (open_c + 1) if l_off == 0 else 0
        for ci in range(c0, len(s)):
            ch = s[ci]
            if in_str:
                if esc:
                    esc = False
                elif ch == "\\":
                    esc = True
                elif ch == '"':
                    in_str = False
                continue
            if ch == '"':
                in_str = True
            elif ch == "(":
                depth_p += 1
            elif ch == ")":
                depth_p -= 1
                if depth_p == 0:
                    return positions
            elif ch == "[":
                depth_b += 1
            elif ch == "]":
                depth_b = max(0, depth_b - 1)
            elif ch == "{":
                depth_c += 1
            elif ch == "}":
                depth_c = max(0, depth_c - 1)
            elif ch == "," and depth_p == 1 and depth_b == 0 and depth_c == 0:
                nc = ci + 1
                while nc < len(s) and s[nc] == " ":
                    nc += 1
                if nc < len(s) and s[nc] != ")":
                    positions.append((li + 1, nc + 1))
    return positions


def prepare_parity_rows(rows, positions, repo_dir, entries, read_lines):
    """Enriches `rows` in-place for parity measurement before querying servers."""
    lens_recs = positions.get("code_lens") or []
    lens_idx = 0
    sig_multi_idx = 0
    for row in rows:
        key = row["method"]
        if key == "signature_help" and row.get("file"):
            lines = read_lines(row["file"])
            cands = call_arg_positions(lines, row.get("line") or 1, row.get("character") or 1)
            if len(cands) > 1:
                chosen = cands[sig_multi_idx % len(cands)]
                sig_multi_idx += 1
            else:
                chosen = cands[0]
            row["line"], row["character"] = chosen[0], chosen[1]
            if row["scenario"].get("location"):
                row["scenario"]["location"][1] = chosen[0]
                row["scenario"]["location"][2] = chosen[1]
            row["scenario"]["arg_index_expected"] = cands.index(chosen)
        elif key in ("inlay_hint", "semantic_tokens_range") and row.get("file"):
            lines = read_lines(row["file"])
            last_line0 = max(0, len(lines) - 1)
            row["scenario"]["last_line0"] = last_line0
            row["scenario"]["last_char0"] = len(lines[last_line0]) if lines else 0
        elif key == "code_lens_resolve":
            rel = row.get("file") or ""
            if not rel and lens_idx < len(lens_recs):
                rel = lens_recs[lens_idx].get("file") or ""
            lens_idx += 1
            if rel:
                abs_f = str(Path(repo_dir) / rel)
                row["file"] = rel
                row["target_kind"] = target_kind_of(rel, entries)
                row["scenario"]["file"] = abs_f
                row["scenario"]["location"] = [abs_f, 1, 1]


def execute_parity_request(srv, key, row, repo_dir, is_ra, timeout, read_lines):
    """Executes one parity request against `srv`, returning `(outcome, res, code, us, extra)`."""
    sc = row["scenario"]
    rel = row.get("file") or ""
    abs_f = sc.get("file") or ( str(Path(repo_dir) / rel) if rel else "" )
    uri = f"file://{abs_f}" if abs_f and not str(abs_f).startswith("file://") else f"file://{repo_dir}"

    if key == "inlay_hint" and rel:
        lines = read_lines(rel)
        text = "\n".join(lines)
        end_line = int(sc.get("last_line0") if "last_line0" in sc else max(0, len(lines) - 1))
        end_char = int(sc.get("last_char0") if "last_char0" in sc else (len(lines[end_line]) if lines else 0))
        srv.notify("textDocument/didOpen", {"textDocument": {"uri": uri, "languageId": "rust", "version": 1, "text": text}})
        params = {"textDocument": {"uri": uri}, "range": {"start": {"line": 0, "character": 0}, "end": {"line": end_line, "character": end_char}}}
        res, us, err = srv.request(RPC_OF[key], params, timeout=timeout)
        srv.notify("textDocument/didClose", {"textDocument": {"uri": uri}})
        outcome, code = classify_outcome(key, res, err)
        return outcome, res, code, us, {}

    if key == "code_lens_resolve" and rel:
        text = "\n".join(read_lines(rel))
        srv.notify("textDocument/didOpen", {"textDocument": {"uri": uri, "languageId": "rust", "version": 1, "text": text}})
        lenses, l_us, l_err = srv.request("textDocument/codeLens", {"textDocument": {"uri": uri}}, timeout=timeout)
        if isinstance(lenses, list) and lenses:
            chosen = next((l for l in lenses if isinstance(l, dict) and not l.get("command") and l.get("data")), lenses[0])
            res, us, err = srv.request("codeLens/resolve", chosen, timeout=timeout)
            if isinstance(res, dict) and not (res.get("command") or {}).get("title"):
                res = None
        else:
            res, us, err = None, l_us, l_err
        srv.notify("textDocument/didClose", {"textDocument": {"uri": uri}})
        outcome, code = classify_outcome(key, res, err)
        return outcome, res, code, us, {}

    if key == "semantic_tokens_range" and rel:
        lines = read_lines(rel)
        last_line0 = int(sc.get("last_line0") if "last_line0" in sc else max(0, len(lines) - 1))
        end_line = min(100, last_line0)
        end_char = int(sc.get("last_char0") or 0) if end_line == last_line0 else 0
        params = {"textDocument": {"uri": uri}, "range": {"start": {"line": 0, "character": 0}, "end": {"line": end_line, "character": end_char}}}
    else:
        params = build_params(key, sc, str(repo_dir), is_ra=is_ra)

    res, us, err = srv.request(RPC_OF[key], params, timeout=timeout)
    outcome, code = classify_outcome(key, res, err)
    extra = {"legend": srv.semantic_token_types} if key.startswith("semantic_tokens_") else {}
    return outcome, res, code, us, extra
