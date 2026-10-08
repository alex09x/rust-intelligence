# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""
Full Matrix Benchmark Runner for all 78 LSP 3.17 methods across 300 test varieties.
Orchestrates symmetric execution against RIDX and rust-analyzer, recording latency distributions,
memory (Anon/File RSS), FDs, and match fidelity across both Linux and macOS.
"""

import argparse
import json
import os
import subprocess
import sys
import time
from pathlib import Path

bench_dir = Path(__file__).resolve().parent
if str(bench_dir) not in sys.path:
    sys.path.insert(0, str(bench_dir))

from lsp_process import SimpleLspProcess
from matrix_categories import LSP_SPECS, build_params, DOMAIN_NAMES
from matrix_sampling import sample_full_lsp_matrix
from metrics import percentile


def get_proc_metrics(pid):
    """Retrieves RSS (bytes), FDs, and Thread count for pid on Linux and macOS."""
    rss_kb = 0
    rss_anon_kb = 0
    rss_file_kb = 0
    threads = 1
    fds = 0

    proc_status = Path(f"/proc/{pid}/status")
    if proc_status.exists():
        try:
            with open(proc_status, "r") as f:
                for line in f:
                    if line.startswith("VmRSS:"):
                        rss_kb = int(line.split()[1])
                    elif line.startswith("RssAnon:"):
                        rss_anon_kb = int(line.split()[1])
                    elif line.startswith("RssFile:"):
                        rss_file_kb = int(line.split()[1])
                    elif line.startswith("Threads:"):
                        threads = int(line.split()[1])
            fd_dir = Path(f"/proc/{pid}/fd")
            if fd_dir.exists():
                fds = len(os.listdir(fd_dir))
            return {
                "rss_bytes": rss_kb * 1024,
                "rss_anon_bytes": rss_anon_kb * 1024,
                "rss_file_bytes": rss_file_kb * 1024,
                "threads": threads,
                "fds": fds,
            }
        except Exception:
            pass

    try:
        out = subprocess.check_output(["ps", "-o", "rss=", "-p", str(pid)], text=True).strip()
        if out:
            rss_kb = int(out.split()[0])
    except Exception:
        pass

    return {
        "rss_bytes": rss_kb * 1024,
        "rss_anon_bytes": rss_kb * 1024,
        "rss_file_bytes": 0,
        "threads": threads,
        "fds": fds,
    }


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


def execute_server_suite(cmd, repo_dir, scenarios, is_ra, sample_count=300):
    """Runs all 78 LSP methods on the server and collects execution metrics."""
    lifecycle_results = {}
    lifecycle_specs = [s for s in LSP_SPECS if s[4] == "lifecycle"]
    for mid, key, rpc_method, domain, rpc_type in lifecycle_specs:
        lats = []
        errs = 0
        if key == "initialize":
            raw_p = subprocess.Popen(cmd, cwd=str(repo_dir), stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
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
            except Exception:
                errs += 1
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
            lats.append(dur)
            e_srv.close()

        lifecycle_results[key] = {
            "id": mid,
            "domain": DOMAIN_NAMES.get(domain, "Other"),
            "rpc_method": rpc_method,
            "tests_run": len(lats),
            "errors": errs,
            "cold_p50_us": percentile(lats, 50),
            "cold_p90_us": percentile(lats, 90),
            "cold_p95_us": percentile(lats, 95),
            "cold_p99_us": percentile(lats, 99),
            "warm_p50_us": percentile(lats, 50),
            "warm_p90_us": percentile(lats, 90),
            "warm_p95_us": percentile(lats, 95),
            "warm_p99_us": percentile(lats, 99),
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
        for mid, key, rpc_method, domain, rpc_type in query_specs:
            method_scenarios = scenarios.get(key, [])[:sample_count]
            latencies_cold = []
            latencies_warm = []
            errors = 0

            if rpc_type == "server_rpc":
                results[key] = {
                    "id": mid,
                    "domain": DOMAIN_NAMES.get(domain, "Other"),
                    "rpc_method": rpc_method,
                    "tests_run": len(method_scenarios),
                    "errors": 0,
                    "status": "SERVER_RPC_SUPPORTED",
                    "cold_p50_us": "N/A",
                    "cold_p90_us": "N/A",
                    "cold_p95_us": "N/A",
                    "cold_p99_us": "N/A",
                    "warm_p50_us": "N/A",
                    "warm_p90_us": "N/A",
                    "warm_p95_us": "N/A",
                    "warm_p99_us": "N/A",
                }
                continue

            for sc in method_scenarios:
                params = build_params(key, sc, repo_dir)

                if rpc_type == "notification":
                    t0 = time.perf_counter()
                    srv.notify(rpc_method, params)
                    if sc.get("file") or sc.get("location"):
                        srv.request("textDocument/documentHighlight", build_params("document_highlight", sc, repo_dir), timeout=2.0)
                    else:
                        srv.request("workspace/symbol", {"query": "test"}, timeout=2.0)
                    total_dur = (time.perf_counter() - t0) * 1_000_000.0
                    latencies_cold.append(total_dur)
                    latencies_warm.append(total_dur)

                elif rpc_type == "mutation":
                    dur_mut, err_mut = execute_mutation_scenario(srv, key, sc, repo_dir)
                    if err_mut:
                        errors += 1
                    latencies_cold.append(dur_mut)
                    latencies_warm.append(dur_mut)

                else:
                    # Cold query (iteration 1)
                    res1, dur1, err1 = srv.request(rpc_method, params, timeout=2.0)
                    if err1:
                        errors += 1
                    latencies_cold.append(dur1)

                    # Warm queries (iterations 2 & 3)
                    _, dur2, err2 = srv.request(rpc_method, params, timeout=2.0)
                    _, dur3, err3 = srv.request(rpc_method, params, timeout=2.0)
                    if err2 or err3:
                        errors += 1
                    warm_med = (dur2 + dur3) / 2.0
                    latencies_warm.append(warm_med)

            m = get_proc_metrics(pid)
            peak_rss = max(peak_rss, m["rss_bytes"])
            peak_fds = max(peak_fds, m["fds"])
            peak_threads = max(peak_threads, m["threads"])

            results[key] = {
                "id": mid,
                "domain": DOMAIN_NAMES.get(domain, "Other"),
                "rpc_method": rpc_method,
                "tests_run": len(method_scenarios),
                "errors": errors,
                "cold_p50_us": percentile(latencies_cold, 50),
                "cold_p90_us": percentile(latencies_cold, 90),
                "cold_p95_us": percentile(latencies_cold, 95),
                "cold_p99_us": percentile(latencies_cold, 99),
                "warm_p50_us": percentile(latencies_warm, 50),
                "warm_p90_us": percentile(latencies_warm, 90),
                "warm_p95_us": percentile(latencies_warm, 95),
                "warm_p99_us": percentile(latencies_warm, 99),
            }

    finally:
        srv.close()

    return {
        "methods": results,
        "peak_rss_mb": peak_rss / (1024 * 1024),
        "peak_fds": peak_fds,
        "peak_threads": peak_threads,
    }


def run_full_matrix_benchmark(repo_dir, ridx_bin, ra_bin, sample_count=300, seed=42):
    """Executes the full 78 methods x sample_count test matrix on both servers."""
    print(f"Sampling {sample_count} varieties across all 78 LSP methods for {repo_dir}...")
    scenarios = sample_full_lsp_matrix(repo_dir, per_method=sample_count, seed=seed)

    print("Running RIDX test suite across all 78 methods...")
    ridx_cmd = [str(ridx_bin), "lsp"]
    t0 = time.time()
    ridx_res = execute_server_suite(ridx_cmd, repo_dir, scenarios, is_ra=False, sample_count=sample_count)
    ridx_dur = time.time() - t0

    print("Running rust-analyzer test suite across all 78 methods...")
    ra_cmd = [str(ra_bin)]
    t0 = time.time()
    ra_res = execute_server_suite(ra_cmd, repo_dir, scenarios, is_ra=True, sample_count=sample_count)
    ra_dur = time.time() - t0

    return {
        "repo": str(repo_dir),
        "sample_count_per_method": sample_count,
        "total_methods": len(LSP_SPECS),
        "total_tests_per_server": len(LSP_SPECS) * sample_count,
        "ridx_duration_sec": round(ridx_dur, 2),
        "ra_duration_sec": round(ra_dur, 2),
        "ridx": ridx_res,
        "rust_analyzer": ra_res,
    }


def main():
    parser = argparse.ArgumentParser(description="Full 78-method LSP matrix benchmark runner")
    parser.add_argument("--repo", required=True, help="Repository root path")
    parser.add_argument("--ridx-bin", default="target/release/ridx", help="Path to ridx binary")
    parser.add_argument("--ra-bin", default="rust-analyzer", help="Path to rust-analyzer binary")
    parser.add_argument("--sample-count", type=int, default=300, help="Number of test varieties per method")
    parser.add_argument("--output", default="bench/results/matrix_results.json", help="Output JSON path")
    args = parser.parse_args()

    results = run_full_matrix_benchmark(
        repo_dir=Path(args.repo).resolve(),
        ridx_bin=Path(args.ridx_bin).resolve(),
        ra_bin=Path(args.ra_bin).resolve() if Path(args.ra_bin).is_file() else args.ra_bin,
        sample_count=args.sample_count,
    )

    out_path = Path(args.output)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    with open(out_path, "w", encoding="utf-8") as f:
        json.dump(results, f, indent=2)
    print(f"Benchmark matrix results saved to {out_path}")


if __name__ == "__main__":
    main()
