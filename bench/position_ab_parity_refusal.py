# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Justified refusal classification (`ridx_refusal_justified`) for `rename` and `prepare_rename`.

A refusal is classified as `ridx_refusal_justified` when ALL of the following hold:
1. Method is `rename` or `prepare_rename`.
2. `rust-analyzer` returned a non-empty answer (`outcome == "a"`).
3. `ridx` returned an explicit JSON-RPC error (`outcome == "c"`) carrying a non-empty
   human-readable refusal reason (`error_message`), never a transport failure (`TIMEOUT`,
   `PROCESS_EXITED`, `BROKEN_PIPE`) or empty answer (`outcome == "b"`).
4. `rust-analyzer`'s `WorkspaceEdit` for that rename (directly on `rename`, or via a
   co-located `textDocument/rename` probe on `prepare_rename`) touches at least one file
   outside `workspace_root` (e.g. sysroot `std`/`core` or Cargo registry crates).

In-workspace cases where `rust-analyzer`'s edit breaks `cargo check` are tracked separately
by `bench/rename_compile.py` and are intentionally NOT mixed into this parity rule.
"""

import os
import urllib.parse

TRANSPORT_ERRORS = frozenset({"TIMEOUT", "PROCESS_EXITED", "BROKEN_PIPE"})


def extract_refusal_reason(r_ans):
    """Returns a non-empty refusal message from `r_ans` if `ridx` returned an explicit JSON-RPC refusal."""
    if not isinstance(r_ans, dict) or r_ans.get("outcome") != "c":
        return None
    err = r_ans.get("error")
    if isinstance(err, str) and err in TRANSPORT_ERRORS:
        return None
    msg = r_ans.get("error_message")
    if not msg and isinstance(err, dict):
        msg = err.get("message")
    if not isinstance(msg, str):
        return None
    msg = msg.strip()
    if not msg or msg in TRANSPORT_ERRORS or msg.lstrip("-").isdigit():
        return None
    return msg


def workspace_edit_external_paths(res, repo_dir):
    """Returns `(has_edits, external_paths)` for an LSP `WorkspaceEdit` relative to `repo_dir`."""
    if not isinstance(res, dict) or not repo_dir:
        return False, []
    uris = []
    for uri, edits in (res.get("changes") or {}).items():
        if uri and edits:
            uris.append(uri)
    for dc in res.get("documentChanges") or []:
        if not isinstance(dc, dict):
            continue
        if "textDocument" in dc:
            uri = (dc.get("textDocument") or {}).get("uri")
            if uri and dc.get("edits"):
                uris.append(uri)
        else:
            for key in ("uri", "oldUri", "newUri"):
                if dc.get(key):
                    uris.append(dc[key])
    file_paths = []
    for uri in uris:
        if isinstance(uri, str) and uri.startswith("file://"):
            file_paths.append(os.path.realpath(urllib.parse.unquote(uri[7:])))
    if not file_paths:
        return False, []
    repo_real = os.path.realpath(str(repo_dir)).rstrip("/")
    prefix = repo_real + "/"
    external = sorted({p for p in file_paths if p != repo_real and not p.startswith(prefix)})
    return True, external


def check_ridx_refusal_justified(row, r_ans, a_ans, repo_dir):
    """Returns `('ridx_refusal_justified', evidence)` when `ridx` refused an external-edit rename, else `None`."""
    method = row.get("method")
    if method not in ("rename", "prepare_rename"):
        return None
    if not isinstance(a_ans, dict) or a_ans.get("outcome") != "a":
        return None
    reason = extract_refusal_reason(r_ans)
    if not reason:
        return None
    ra_edit = a_ans.get("res") if method == "rename" else a_ans.get("ra_rename_probe")
    has_edits, external_paths = workspace_edit_external_paths(ra_edit, repo_dir)
    if not has_edits or not external_paths:
        return None
    return "ridx_refusal_justified", {
        "method": method,
        "ridx_error": r_ans.get("error"),
        "ridx_reason": reason,
        "external_file_count": len(external_paths),
        "external_files": external_paths[:5],
    }


def classify_c_vs_a(row, r_ans, a_ans, repo_dir):
    """Classifies a row where RIDX returned error (`c`) and rust-analyzer answered (`a`)."""
    justified = check_ridx_refusal_justified(row, r_ans, a_ans, repo_dir)
    if justified is not None:
        return justified
    ev = {"ridx_error": r_ans.get("error"), "method": row["method"]}
    reason = extract_refusal_reason(r_ans)
    if reason:
        ev["ridx_reason"] = reason
    return "p_ridx_error_or_timeout", ev
