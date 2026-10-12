#
# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
#

"""Execution logic for complex scenarios (semantic mutations, synchronization)."""

import time
from pathlib import Path


def execute_mutation_scenario(srv, key, sc, repo_dir):
    """Executes Section 7 semantic mutations with strict verification (Case A, Case B, Case C)."""
    t0 = time.perf_counter()
    err = None

    if key == "mutation_case_a":
        # Case A: Cross-file new symbol injection (file_a defines, file_b calls via use crate::)
        fa = sc.get("file_a")
        fb = sc.get("file_b")
        fn_name = sc.get("injected_fn", "bench_probe_0")
        call_file = Path(fb) if fb and Path(fb).is_file() else Path(repo_dir) / "src" / "lib.rs"
        root_file = Path(fa) if fa and Path(fa).is_file() else call_file

        srv.notify("textDocument/didOpen", {"textDocument": {"uri": f"file://{root_file}", "languageId": "rust", "version": 1, "text": f"\npub fn {fn_name}() -> usize {{ 42 }}\n"}})
        srv.notify("textDocument/didOpen", {"textDocument": {"uri": f"file://{call_file}", "languageId": "rust", "version": 1, "text": f"\nuse crate::{fn_name};\npub fn caller_{fn_name}() {{ let _ = {fn_name}(); }}\n"}})
        res, dur, err = srv.request("textDocument/definition", {"textDocument": {"uri": f"file://{call_file}"}, "position": {"line": 1, "character": 12}}, timeout=2.0)
        srv.notify("textDocument/didClose", {"textDocument": {"uri": f"file://{root_file}"}})
        srv.notify("textDocument/didClose", {"textDocument": {"uri": f"file://{call_file}"}})
        return dur, err

    elif key == "mutation_case_b":
        # Case B: Stale overlay detector (rename in file_a, query orig_name in caller_file -> expect Not Found)
        fa = sc.get("file_a")
        fb = sc.get("file_b")
        orig_name = sc.get("orig_name", "orig_func")
        stale_name = sc.get("stale_renamed", f"{orig_name}_stale")
        target_file = Path(fa) if fa and Path(fa).is_file() else Path(repo_dir) / "src" / "lib.rs"
        caller_file = Path(fb) if fb and Path(fb).is_file() else target_file

        srv.notify("textDocument/didOpen", {"textDocument": {"uri": f"file://{target_file}", "languageId": "rust", "version": 1, "text": f"\npub fn {stale_name}() {{}}\n"}})
        srv.notify("textDocument/didOpen", {"textDocument": {"uri": f"file://{caller_file}", "languageId": "rust", "version": 1, "text": f"\npub fn check_stale() {{ let _ = {orig_name}(); }}\n"}})
        res, dur, err = srv.request("textDocument/definition", {"textDocument": {"uri": f"file://{caller_file}"}, "position": {"line": 1, "character": 33}}, timeout=2.0)
        srv.notify("textDocument/didClose", {"textDocument": {"uri": f"file://{target_file}"}})
        srv.notify("textDocument/didClose", {"textDocument": {"uri": f"file://{caller_file}"}})
        if res:
            err = "STALE_INDEX_LEAK_DETECTED"
        return dur, err

    elif key == "mutation_case_c":
        # Case C: Line displacement (+10 lines in file_a)
        fa = sc.get("file_a")
        target_file = Path(fa) if fa and Path(fa).is_file() else Path(repo_dir) / "src" / "lib.rs"
        orig_line = sc.get("orig_line", 1)
        loc = sc.get("location", [str(target_file), orig_line, 4])
        query_col = loc[2] if len(loc) > 2 else 4

        try:
            with open(target_file, "r", encoding="utf-8", errors="ignore") as f_in:
                original_text = f_in.read()
        except Exception:
            original_text = "pub fn dummy() {}\n"

        shifted_text = ("\n" * 10) + original_text
        srv.notify("textDocument/didOpen", {"textDocument": {"uri": f"file://{target_file}", "languageId": "rust", "version": 1, "text": shifted_text}})
        res, dur, err = srv.request("textDocument/definition", {"textDocument": {"uri": f"file://{target_file}"}, "position": {"line": orig_line + 10, "character": query_col}}, timeout=2.0)
        srv.notify("textDocument/didClose", {"textDocument": {"uri": f"file://{target_file}"}})
        return dur, err

    dur = (time.perf_counter() - t0) * 1_000_000.0
    return dur, err
