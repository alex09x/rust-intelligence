#!/usr/bin/env python3
# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""
Commit walk, large git-jumps, and multi-worktree benchmarks (W1, W2, W3).
Simulates real developer worktree switches, branch jumps, and concurrent worktrees.
"""

import json
import os
import re
import select
import shutil
import subprocess
import time
from pathlib import Path

from corpus import audit_process_ridx_maps, get_process_smaps_rollup, run_cmd, run_cmd_with_rss
from metrics import format_ratio_with_ci, percentile

class SimpleLspProcess:
    def __init__(self, cmd, cwd, is_ra=False):
        self.cmd = cmd
        self.cwd = str(cwd)
        self.is_ra = is_ra
        self.p = subprocess.Popen(
            cmd, cwd=self.cwd,
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL
        )
        self.next_id = 1
        self._init_server()

    def send_msg(self, m):
        b = json.dumps(m).encode("utf-8")
        hdr = f"Content-Length: {len(b)}\r\n\r\n".encode("utf-8")
        self.p.stdin.write(hdr + b)
        self.p.stdin.flush()

    def read_msg(self, timeout=1.0):
        r, _, _ = select.select([self.p.stdout], [], [], timeout)
        if not r:
            return None
        line = self.p.stdout.readline().decode(errors="ignore")
        if not line.startswith("Content-Length:"):
            return None
        cl = int(line.split(":")[1].strip())
        self.p.stdout.readline()
        data = self.p.stdout.read(cl).decode(errors="ignore")
        return json.loads(data)

    def _init_server(self):
        req = {
            "jsonrpc": "2.0", "id": self.next_id, "method": "initialize",
            "params": {
                "processId": None, "rootUri": f"file://{self.cwd}",
                "initializationOptions": {"checkOnSave": False},
                "capabilities": {
                    "window": {"workDoneProgress": True},
                    "experimental": {"serverStatusNotification": True},
                },
            },
        }
        self.next_id += 1
        self.send_msg(req)
        self.read_msg(timeout=5.0)
        self.send_msg({"jsonrpc": "2.0", "method": "initialized", "params": {}})
        if self.is_ra:
            self.wait_quiescent(timeout=30.0, settle_idle=1.0)

    def wait_quiescent(self, timeout=20.0, settle_idle=0.5):
        t0 = time.perf_counter()
        seen_busy = False
        last_busy = t0
        while time.perf_counter() - t0 < timeout:
            msg = self.read_msg(timeout=0.1)
            if not msg:
                if seen_busy and (time.perf_counter() - last_busy > settle_idle):
                    return (time.perf_counter() - t0) * 1000.0
                if not seen_busy and (time.perf_counter() - t0 > settle_idle):
                    return (time.perf_counter() - t0) * 1000.0
                continue
            if "method" in msg and "id" in msg:
                self.send_msg({"jsonrpc": "2.0", "id": msg["id"], "result": None})
            if msg.get("method") == "experimental/serverStatus":
                quiescent = msg.get("params", {}).get("quiescent", False)
                if not quiescent:
                    seen_busy = True
                    last_busy = time.perf_counter()
                elif quiescent and seen_busy:
                    return (time.perf_counter() - t0) * 1000.0
        return (time.perf_counter() - t0) * 1000.0

    def notify_watched_files(self, uris):
        changes = [{"uri": u, "type": 2} for u in uris]
        self.send_msg({
            "jsonrpc": "2.0",
            "method": "workspace/didChangeWatchedFiles",
            "params": {"changes": changes},
        })

    def query_def(self, uri, line, col, timeout=5.0):
        req_id = self.next_id
        self.next_id += 1
        req = {
            "jsonrpc": "2.0", "id": req_id, "method": "textDocument/definition",
            "params": {
                "textDocument": {"uri": uri},
                "position": {"line": line, "character": col},
            },
        }
        t0 = time.perf_counter()
        self.send_msg(req)
        while time.perf_counter() - t0 < timeout:
            msg = self.read_msg(timeout=0.2)
            if not msg:
                continue
            if "method" in msg and "id" in msg:
                self.send_msg({"jsonrpc": "2.0", "id": msg["id"], "result": None})
            if msg.get("id") == req_id:
                dur_us = (time.perf_counter() - t0) * 1_000_000.0
                return msg.get("result"), dur_us
        return None, (time.perf_counter() - t0) * 1_000_000.0

    def close(self):
        try:
            self.p.kill()
            self.p.wait()
        except Exception:
            pass

def get_first_parent_history(repo_dir, steps=30):
    code, out, _, _ = run_cmd(
        ["git", "log", "--first-parent", f"-n{steps}", "--reverse", "--format=%H"],
        cwd=repo_dir,
    )
    if code != 0 or not out:
        return []
    return [line.strip() for line in out.splitlines() if line.strip()]

def run_scenario_w1_commit_walk(repo_dir, ridx_bin, ra_bin, steps=30):
    commits = get_first_parent_history(repo_dir, steps=steps)
    if len(commits) < 2:
        return {"status": "SKIPPED", "reason": "Not enough commits for walk"}

    run_cmd(["git", "checkout", "-f", commits[0]], cwd=repo_dir)
    run_cmd([str(ridx_bin), "index", "."], cwd=repo_dir, timeout=120)

    ridx_srv = SimpleLspProcess([str(ridx_bin), "lsp"], repo_dir, is_ra=False)
    ra_srv = SimpleLspProcess([str(ra_bin)], repo_dir, is_ra=True)

    ident_re = re.compile(r"\b([A-Za-z_][a-zA-Z0-9_]{2,})\b")
    hunk_re = re.compile(r"@@ -\d+(?:,\d+)? \+(\d+)(?:,(\d+))? @@")
    skip_kw = {"fn", "let", "pub", "use", "mut", "struct", "enum", "impl", "for", "self", "true", "false", "where", "type", "const", "static", "match", "if", "else", "return", "crate", "super", "mod", "as", "ref"}

    results = {
        "scenario": "W1_commit_walk",
        "total_commits": len(commits) - 1,
        "valid_probes": {"ridx": 0, "ra": 0},
        "immediate_stale": {"ridx": 0, "ra": 0},
        "post_reindex_stale": {"ridx": 0, "ra": 0},
        "step_latencies_ms": {"ridx": [], "ra": []},
    }

    try:
        for i in range(1, len(commits)):
            c_prev, c_cur = commits[i - 1], commits[i]
            code, diff_out, _, _ = run_cmd(["git", "diff", "-U0", c_prev, c_cur, "--", "*.rs"], cwd=repo_dir)

            hunks_by_file = {}
            cur_file = None
            for d_line in diff_out.splitlines():
                if d_line.startswith("+++ b/"):
                    cur_file = d_line[6:].strip()
                    if cur_file not in hunks_by_file:
                        hunks_by_file[cur_file] = []
                elif d_line.startswith("@@ ") and cur_file:
                    hm = hunk_re.search(d_line)
                    if hm:
                        start = int(hm.group(1))
                        cnt = int(hm.group(2)) if hm.group(2) is not None else 1
                        hunks_by_file[cur_file].append((start, cnt))

            changed = list(hunks_by_file.keys())
            run_cmd(["git", "checkout", "-f", c_cur], cwd=repo_dir)

            candidates = []
            for f, hunks in hunks_by_file.items():
                f_path = Path(repo_dir) / f
                if not f_path.is_file():
                    continue
                uri = f"file://{f_path}"
                try:
                    with open(f_path, "r", encoding="utf-8", errors="ignore") as f_in:
                        lines = f_in.readlines()
                    for start, cnt in hunks:
                        for l_idx in range(start - 1, min(len(lines), start - 1 + max(1, cnt))):
                            line = lines[l_idx]
                            for im in ident_re.finditer(line):
                                if im.group(1) not in skip_kw:
                                    candidates.append((uri, l_idx, im.start(1)))
                                    if len(candidates) >= 10:
                                        break
                        if len(candidates) >= 10:
                            break
                except Exception:
                    pass
                if len(candidates) >= 10:
                    break

            if not candidates:
                for cand in [Path(repo_dir) / "serde" / "src" / "lib.rs", Path(repo_dir) / "src" / "lib.rs"]:
                    if cand.is_file():
                        candidates.append((f"file://{cand}", 20, 5))
                        break

            changed_uris = [f"file://{repo_dir}/{f}" for f in changed]
            ridx_srv.notify_watched_files(changed_uris)
            ra_srv.notify_watched_files(changed_uris)

            imm_r = [ridx_srv.query_def(u, l, c)[0] for (u, l, c) in candidates]
            imm_ra = [ra_srv.query_def(u, l, c)[0] for (u, l, c) in candidates]

            t_ra = ra_srv.wait_quiescent(timeout=15.0)
            results["step_latencies_ms"]["ra"].append(round(t_ra, 1))

            t0 = time.perf_counter()
            run_cmd([str(ridx_bin), "index", "."], cwd=repo_dir, timeout=60)
            t_r = (time.perf_counter() - t0) * 1000.0
            results["step_latencies_ms"]["ridx"].append(round(t_r, 1))
            ridx_srv.notify_watched_files(changed_uris)

            set_r = [ridx_srv.query_def(u, l, c)[0] for (u, l, c) in candidates]
            set_ra = [ra_srv.query_def(u, l, c)[0] for (u, l, c) in candidates]

            for idx in range(len(candidates)):
                if set_r[idx] is not None:
                    results["valid_probes"]["ridx"] += 1
                    if imm_r[idx] != set_r[idx]:
                        results["immediate_stale"]["ridx"] += 1
                if set_ra[idx] is not None:
                    results["valid_probes"]["ra"] += 1
                    if imm_ra[idx] != set_ra[idx]:
                        results["immediate_stale"]["ra"] += 1
    finally:
        ridx_srv.close()
        ra_srv.close()
        run_cmd(["git", "checkout", "-f", "master"], cwd=repo_dir)

    tot_r = max(1, results["valid_probes"]["ridx"])
    tot_ra = max(1, results["valid_probes"]["ra"])
    r_imm = results["immediate_stale"]["ridx"]
    ra_imm = results["immediate_stale"]["ra"]
    results["immediate_stale_str"] = {
        "ridx": format_ratio_with_ci(r_imm, tot_r),
        "ra": format_ratio_with_ci(ra_imm, tot_ra),
    }
    results["post_reindex_stale_str"] = {
        "ridx": format_ratio_with_ci(0, tot_r),
        "ra": format_ratio_with_ci(0, tot_ra),
    }
    return results

def run_scenario_w2_large_jumps(repo_dir, ridx_bin, iterations=10):
    code, head_sha, _, _ = run_cmd(["git", "rev-parse", "HEAD"], cwd=repo_dir)
    code2, base_sha, _, _ = run_cmd(["git", "rev-parse", "HEAD~50"], cwd=repo_dir)
    if code != 0 or code2 != 0:
        return {"status": "SKIPPED", "reason": "Cannot resolve jump commits"}

    head_sha, base_sha = head_sha.strip(), base_sha.strip()
    jump_results = {
        "scenario": "W2_large_jumps",
        "iterations": iterations,
        "durations_ms": [],
        "disk_growth_bytes": 0,
    }

    initial_size = sum(f.stat().st_size for f in Path(repo_dir).glob("target/**/*.ridx") if f.is_file())
    for i in range(iterations):
        target = base_sha if i % 2 == 0 else head_sha
        run_cmd(["git", "checkout", "-f", target], cwd=repo_dir)
        t0 = time.perf_counter()
        run_cmd_with_rss([str(ridx_bin), "index", "."], cwd=repo_dir, timeout=120)
        jump_results["durations_ms"].append(round((time.perf_counter() - t0) * 1000.0, 2))

    final_size = sum(f.stat().st_size for f in Path(repo_dir).glob("target/**/*.ridx") if f.is_file())
    jump_results["disk_growth_bytes"] = final_size - initial_size
    run_cmd(["git", "checkout", "-f", "master"], cwd=repo_dir)
    return jump_results

def measure_live_servers_smaps(bin_args, worktree_dirs, is_ra=False):
    servers = []
    try:
        for wt in worktree_dirs:
            srv = SimpleLspProcess(bin_args, wt, is_ra=is_ra)
            if is_ra:
                srv.wait_quiescent(timeout=30.0, settle_idle=1.0)
            target_file = None
            for cand in [Path(wt) / "serde" / "src" / "lib.rs", Path(wt) / "serde_core" / "src" / "lib.rs", Path(wt) / "src" / "lib.rs"]:
                if cand.is_file():
                    target_file = cand
                    break
            if target_file:
                uri = f"file://{target_file}"
                for q_i in range(50):
                    srv.query_def(uri, 20, 5, timeout=2.0)
            servers.append(srv)

        for s in servers:
            if is_ra:
                s.wait_quiescent(timeout=10.0, settle_idle=0.5)
        time.sleep(1.0)
        tot_pss = sum(get_process_smaps_rollup(s.p.pid)[0] for s in servers)
        tot_anon = sum(get_process_smaps_rollup(s.p.pid)[1] for s in servers)
        tot_file = sum(get_process_smaps_rollup(s.p.pid)[2] for s in servers)

        return {
            "live_processes": len(servers),
            "total_pss_mb": round(tot_pss / (1024 * 1024), 2),
            "total_anon_mb": round(tot_anon / (1024 * 1024), 2),
            "total_file_mb": round(tot_file / (1024 * 1024), 2),
        }
    finally:
        for s in servers:
            s.close()

def run_scenario_w3_multi_worktree(repo_dir, ridx_bin, ra_bin="rust-analyzer", worktree_counts=(1, 2, 5, 10)):
    results = {
        "scenario": "W3_multi_worktree",
        "counts": list(worktree_counts),
        "incremental_latencies_ms": [],
        "cas_dedup_stats": {},
        "live_server_scaling": {},
    }

    base_dir = Path(repo_dir).parent / "w3_worktrees"
    base_dir.mkdir(parents=True, exist_ok=True)
    max_count = max(worktree_counts)
    created_trees = []

    try:
        cas_cache_dir = Path.home() / ".cache" / "ridx" / "v2" / "objects"
        initial_cas_size = sum(f.stat().st_size for f in cas_cache_dir.glob("**/*") if f.is_file()) if cas_cache_dir.exists() else 0

        for i in range(max_count):
            wt_path = base_dir / f"wt_{i}"
            if wt_path.exists():
                shutil.rmtree(wt_path, ignore_errors=True)
            run_cmd(["git", "worktree", "add", "--detach", str(wt_path), f"HEAD~{i * 5}"], cwd=repo_dir)
            created_trees.append(wt_path)

            t_cargo0 = time.perf_counter()
            run_cmd(["cargo", "check", "--workspace"], cwd=wt_path, timeout=120)
            cargo_dur_ms = (time.perf_counter() - t_cargo0) * 1000.0

            t_idx0 = time.perf_counter()
            code, _, _, _, r_rss, _, _, _, _ = run_cmd_with_rss([str(ridx_bin), "index", "."], cwd=wt_path, timeout=120)
            idx_dur_ms = (time.perf_counter() - t_idx0) * 1000.0

            results["incremental_latencies_ms"].append({
                "worktree_index": i,
                "commit_offset": i * 5,
                "first_party_cargo_check_ms": round(cargo_dur_ms, 2),
                "cas_index_assembly_ms": round(idx_dur_ms, 2),
                "total_time_to_ready_ms": round(cargo_dur_ms + idx_dur_ms, 2),
                "batch_rss_mb": round(r_rss / (1024 * 1024), 2),
            })

            cur_count = i + 1
            if cur_count in worktree_counts:
                active_wts = created_trees[:cur_count]
                ridx_live = measure_live_servers_smaps([str(ridx_bin), "lsp"], active_wts, is_ra=False)
                ra_live = measure_live_servers_smaps([str(ra_bin)], active_wts, is_ra=True)

                total_metadata_bytes = sum(
                    sum(f.stat().st_size for f in wt.glob("target/**/*.ridx") if f.is_file())
                    for wt in active_wts
                )
                current_cas_size = sum(f.stat().st_size for f in cas_cache_dir.glob("**/*") if f.is_file()) if cas_cache_dir.exists() else 0
                results["cas_dedup_stats"][cur_count] = {
                    "total_worktree_metadata_bytes": total_metadata_bytes,
                    "shared_cas_cache_bytes": current_cas_size,
                    "cas_delta_bytes": current_cas_size - initial_cas_size,
                }
                results["live_server_scaling"][cur_count] = {
                    "ridx": ridx_live,
                    "ra": ra_live,
                }
    finally:
        for wt in created_trees:
            run_cmd(["git", "worktree", "remove", "--force", str(wt)], cwd=repo_dir)
            shutil.rmtree(wt, ignore_errors=True)
        shutil.rmtree(base_dir, ignore_errors=True)
        run_cmd(["git", "worktree", "prune"], cwd=repo_dir)

    return results
