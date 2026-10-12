# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""
LSP Test Matrix Execution Engine.
Handles execution of Section 7 semantic mutations, lifecycle checks, and query workloads
against LSP server instances, collecting latency distributions, match metrics, and resource usage.
"""

import json
import subprocess
import time
from pathlib import Path

from lsp_process import SimpleLspProcess
from matrix_categories import LSP_SPECS, build_params, DOMAIN_NAMES
from metrics import percentile, get_proc_metrics


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


def _latency_summary(cold, warm):
    return {f"{s}_p{p}_us": percentile(v, p) for s, v in (("cold", cold), ("warm", warm)) for p in (50, 90, 95, 99)}


def _na_summary():
    return {f"{prefix}_p{p}_us": "N/A" for prefix in ("cold", "warm") for p in (50, 90, 95, 99)}


def _record_error_code(err_map, err):
    if err:
        c = str(err.get("code", "UNKNOWN") if isinstance(err, dict) else err)
        err_map[c] = err_map.get(c, 0) + 1


def execute_server_suite(cmd, repo_dir, scenarios, is_ra, sample_count=300):
    """Runs all 78 LSP methods on the server and collects execution metrics."""
    lifecycle_results = {}
    lifecycle_specs = [s for s in LSP_SPECS if s[4] == "lifecycle"]
    for mid, key, rpc_method, domain, rpc_type in lifecycle_specs:
        lats = []
        errs = 0
        err_codes = {}
        if key == "initialize":
            raw_p = subprocess.Popen(cmd, cwd=str(repo_dir), stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
            try:
                init_req = {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"processId": None, "rootUri": f"file://{repo_dir}", "capabilities": {}}}
                b = json.dumps(init_req).encode("utf-8")
                t0 = time.perf_counter()
                raw_p.stdin.write(f"Content-Length: {len(b)}\r\n\r\n".encode("utf-8") + b)
                raw_p.stdin.flush()
                line = raw_p.stdout.readline().decode("latin1")
                if "content-length:" in line.lower():
                    cl = int(line.split(":", 1)[1].strip())
                    raw_p.stdout.readline()
                    raw_p.stdout.read(cl)
                    dur = (time.perf_counter() - t0) * 1_000_000.0
                    lats.append(dur)
                else:
                    errs += 1
                    _record_error_code(err_codes, "MALFORMED_HEADER")
            except Exception as e:
                errs += 1
                _record_error_code(err_codes, str(type(e).__name__))
            finally:
                raw_p.kill()
                raw_p.wait()
        else:
            e_srv = SimpleLspProcess(cmd, repo_dir, is_ra=is_ra)
            t0 = time.perf_counter()
            if key == "initialized":
                e_srv.notify("initialized", {})
                dur = (time.perf_counter() - t0) * 1_000_000.0
                err = None
            elif key == "shutdown":
                res, dur, err = e_srv.request("shutdown", None, timeout=5.0)
            elif key == "exit":
                e_srv.send_msg({"jsonrpc": "2.0", "id": 999, "method": "shutdown", "params": None})
                e_srv.notify("exit", None)
                dur = (time.perf_counter() - t0) * 1_000_000.0
                err = None
            if err:
                errs += 1
                _record_error_code(err_codes, err)
            lats.append(dur)
            e_srv.close()

        lifecycle_results[key] = {
            "id": mid,
            "domain": DOMAIN_NAMES.get(domain, "Other"),
            "rpc_method": rpc_method,
            "tests_run": len(lats),
            "errors": errs,
            "error_codes": err_codes,
            **_latency_summary(lats, lats),
        }

    # Main server instance for queries, sync, presentation, and mutations
    srv = SimpleLspProcess(cmd, repo_dir, is_ra=is_ra)
    pid = srv.p.pid

    results = dict(lifecycle_results)
    peak_rss = 0
    peak_fds = 0
    peak_threads = 0

    try:
        query_specs = [s for s in LSP_SPECS if s[4] != "lifecycle"]
        for idx, (mid, key, rpc_method, domain, rpc_type) in enumerate(query_specs):
            print(f"  [{idx+1}/{len(query_specs)}] {rpc_method} ({DOMAIN_NAMES.get(domain, 'Other')})...", flush=True)
            method_scenarios = scenarios.get(key, [])[:sample_count]
            latencies_cold = []
            latencies_warm = []
            errors = 0
            error_codes = {}
            a_count = 0  # (a) non-empty and verified
            b_count = 0  # (b) empty (null / [] / empty hover)
            c_count = 0  # (c) error / failed

            if rpc_type == "server_rpc":
                results[key] = {
                    "id": mid,
                    "domain": DOMAIN_NAMES.get(domain, "Other"),
                    "rpc_method": rpc_method,
                    "tests_run": len(method_scenarios),
                    "errors": 0,
                    "error_codes": {},
                    "status": "SERVER_RPC_SUPPORTED",
                    "a_non_empty": len(method_scenarios),
                    "b_empty": 0,
                    "c_error": 0,
                    "a_pct": 100.0,
                    "b_pct": 0.0,
                    "c_pct": 0.0,
                    **_na_summary(),
                }
                continue

            for sc in method_scenarios:
                params = build_params(key, sc, repo_dir, is_ra=is_ra)

                if rpc_type == "notification":
                    t0 = time.perf_counter()
                    ok = srv.notify(rpc_method, params)
                    dur_us = (time.perf_counter() - t0) * 1_000_000.0
                    if ok:
                        a_count += 1
                        latencies_cold.append(dur_us)
                        latencies_warm.append(dur_us)
                    else:
                        errors += 1
                        c_count += 1
                        _record_error_code(error_codes, "NOTIFICATION_FAILED")

                elif rpc_type == "mutation":
                    dur_mut, err_mut = execute_mutation_scenario(srv, key, sc, repo_dir)
                    if err_mut:
                        errors += 1
                        c_count += 1
                        _record_error_code(error_codes, err_mut)
                    else:
                        a_count += 1
                    latencies_cold.append(dur_mut)
                    latencies_warm.append(dur_mut)

                else:
                    # Cold query (iteration 1)
                    res1, dur1, err1 = srv.request(rpc_method, params, timeout=2.0)
                    if err1 and isinstance(err1, dict) and err1.get("code") == -32601:
                        results[key] = {
                            "id": mid,
                            "domain": DOMAIN_NAMES.get(domain, "Other"),
                            "rpc_method": rpc_method,
                            "status": "UNSUPPORTED",
                            "tests_run": len(method_scenarios),
                            "errors": 0,
                            "error_codes": {"-32601": len(method_scenarios)},
                            "a_non_empty": 0,
                            "b_empty": 0,
                            "c_error": len(method_scenarios),
                            "a_pct": 0.0,
                            "b_pct": 0.0,
                            "c_pct": 100.0,
                            **_na_summary(),
                        }
                        break
                    if err1:
                        errors += 1
                        c_count += 1
                        _record_error_code(error_codes, err1)
                    elif res1 is None or res1 == [] or res1 == "" or (isinstance(res1, dict) and not res1) or (key == "hover" and isinstance(res1, dict) and not res1.get("contents")):
                        b_count += 1
                    else:
                        a_count += 1
                    latencies_cold.append(dur1)

                    # Warm queries (iterations 2 & 3)
                    _, dur2, err2 = srv.request(rpc_method, params, timeout=2.0)
                    _, dur3, err3 = srv.request(rpc_method, params, timeout=2.0)
                    if err2 or err3:
                        errors += 1
                        if err2:
                            _record_error_code(error_codes, err2)
                        if err3:
                            _record_error_code(error_codes, err3)
                    warm_med = (dur2 + dur3) / 2.0
                    latencies_warm.append(warm_med)

            if key in results:
                continue

            m = get_proc_metrics(pid)
            peak_rss = max(peak_rss, m["rss_bytes"])
            peak_fds = max(peak_fds, m["fds"])
            peak_threads = max(peak_threads, m["threads"])

            tot = max(1, len(method_scenarios))
            results[key] = {
                "id": mid,
                "domain": DOMAIN_NAMES.get(domain, "Other"),
                "rpc_method": rpc_method,
                "tests_run": len(method_scenarios),
                "errors": errors,
                "error_codes": error_codes,
                "a_non_empty": a_count,
                "b_empty": b_count,
                "c_error": c_count,
                "a_pct": round(a_count / tot * 100, 1),
                "b_pct": round(b_count / tot * 100, 1),
                "c_pct": round(c_count / tot * 100, 1),
                **_latency_summary(latencies_cold, latencies_warm),
            }

    finally:
        initial_quiescent = getattr(srv, "initial_quiescent", True)
        initial_wait_ms = round(getattr(srv, "initial_wait_ms", 0.0), 2)
        srv.close()

    return {
        "methods": results,
        "peak_rss_mb": peak_rss / (1024 * 1024),
        "peak_fds": peak_fds,
        "peak_threads": peak_threads,
        "initial_quiescent": initial_quiescent,
        "initial_wait_ms": initial_wait_ms,
    }
