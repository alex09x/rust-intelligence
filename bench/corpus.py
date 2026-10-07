#!/usr/bin/env python3
"""
rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
Copyright (c) 2026 Alexander Panasenko

Contact: alex@prod.codes
Author: https://prod.codes/about/
Project: https://github.com/alex09x/rust-intelligence
SPDX-License-Identifier: MIT OR Apache-2.0

Corpus loading and repository setup utilities for benchmarking.
"""

import json
import os
import resource
import subprocess
import sys
import time
from pathlib import Path

DEFAULT_PILOT_REPOS = [
    {"repo": "tokio-rs/tokio", "commit": "b2636752450484955e7ad334bac678424d51bc4a"},
    {"repo": "BurntSushi/ripgrep", "commit": "3fce3b5bb0236da2df6d99672afb8a719642eca7"},
    {"repo": "serde-rs/serde", "commit": "6693a89cca77e0151437da1c7f890090b9ebf04c"},
    {"repo": "bevyengine/bevy", "commit": "68c80f3b19ddf9e4ae1ce7dd8b5a68273204c2e8"},
    {"repo": "rust-lang/rust-analyzer", "commit": "021bfc15e24909389bd208d1622b7f4f3942b91a"},
]


def run_cmd(cmd, cwd=None, timeout=600):
    t0 = time.perf_counter()
    try:
        proc = subprocess.run(
            cmd,
            cwd=cwd,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            timeout=timeout,
        )
        elapsed = time.perf_counter() - t0
        return proc.returncode, proc.stdout, proc.stderr, elapsed
    except subprocess.TimeoutExpired:
        return -1, "", "Timeout expired", time.perf_counter() - t0
    except Exception as e:
        return -2, "", str(e), time.perf_counter() - t0



def get_process_tree_pids(root_pid):
    """Recursively collects all child PIDs belonging to root_pid."""
    pids = [root_pid]
    try:
        task_dir = Path(f"/proc/{root_pid}/task")
        if task_dir.exists():
            for tid_dir in task_dir.iterdir():
                children_file = tid_dir / "children"
                if children_file.exists():
                    try:
                        with open(children_file, "r") as f:
                            for child_str in f.read().split():
                                child_pid = int(child_str)
                                pids.extend(get_process_tree_pids(child_pid))
                    except (IOError, OSError):
                        pass
    except (IOError, OSError):
        pass
    return pids


def audit_process_ridx_maps(pid):
    """Counts open .ridx FDs and active .ridx mmap regions in /proc."""
    open_ridx_fds = 0
    mapped_ridx_regions = 0
    fd_dir = Path(f"/proc/{pid}/fd")
    if fd_dir.exists():
        try:
            for item in fd_dir.iterdir():
                try:
                    target = os.readlink(item)
                    if target.endswith(".ridx"):
                        open_ridx_fds += 1
                except (IOError, OSError):
                    pass
        except (IOError, OSError):
            pass
    maps_file = Path(f"/proc/{pid}/maps")
    if maps_file.exists():
        try:
            with open(maps_file, "r", encoding="utf-8", errors="replace") as f:
                for line in f:
                    if ".ridx" in line:
                        mapped_ridx_regions += 1
        except (IOError, OSError):
            pass
    return open_ridx_fds, mapped_ridx_regions


def get_process_smaps_rollup(pid):
    """Parses /proc/{pid}/smaps_rollup returning (pss_bytes, anon_bytes, file_bytes)."""
    pss = 0
    anon = 0
    file_bytes = 0
    rollup = Path(f"/proc/{pid}/smaps_rollup")
    if rollup.exists():
        try:
            with open(rollup, "r", encoding="utf-8", errors="replace") as f:
                for line in f:
                    if line.startswith("Pss:"):
                        parts = line.split()
                        if len(parts) >= 2:
                            pss = int(parts[1]) * 1024
                    elif line.startswith("Anonymous:"):
                        parts = line.split()
                        if len(parts) >= 2:
                            anon = int(parts[1]) * 1024
                    elif line.startswith("Pss_File:"):
                        parts = line.split()
                        if len(parts) >= 2:
                            file_bytes = int(parts[1]) * 1024
        except (IOError, OSError):
            pass
    return pss, anon, file_bytes


def run_cmd_with_rss(cmd, cwd=None, timeout=600):
    import tempfile

    t0 = time.perf_counter()
    stdout_f = tempfile.TemporaryFile(mode="w+t", encoding="utf-8", errors="replace")
    stderr_f = tempfile.TemporaryFile(mode="w+t", encoding="utf-8", errors="replace")
    try:
        proc = subprocess.Popen(
            cmd,
            cwd=cwd,
            stdout=stdout_f,
            stderr=stderr_f,
            text=True,
        )
    except Exception as e:
        stdout_f.close()
        stderr_f.close()
        return -2, "", str(e), time.perf_counter() - t0, 0, 0, 0, 0, 0

    peak_rss = 0
    peak_anon = 0
    peak_file = 0
    peak_fds = 0
    peak_threads = 0

    pid = proc.pid

    while proc.poll() is None:
        tree_pids = get_process_tree_pids(pid)
        cur_rss = 0
        cur_anon = 0
        cur_file = 0
        cur_threads = 0
        cur_fds = 0

        for c_pid in tree_pids:
            try:
                status_path = Path(f"/proc/{c_pid}/status")
                with open(status_path, "r", encoding="utf-8") as f:
                    for line in f:
                        if line.startswith("VmRSS:"):
                            parts = line.split()
                            if len(parts) >= 2:
                                cur_rss += int(parts[1]) * 1024
                        elif line.startswith("RssAnon:"):
                            parts = line.split()
                            if len(parts) >= 2:
                                cur_anon += int(parts[1]) * 1024
                        elif line.startswith("RssFile:"):
                            parts = line.split()
                            if len(parts) >= 2:
                                cur_file += int(parts[1]) * 1024
                        elif line.startswith("Threads:"):
                            parts = line.split()
                            if len(parts) >= 2:
                                cur_threads += int(parts[1])
            except (IOError, OSError):
                pass

            try:
                fd_path = Path(f"/proc/{c_pid}/fd")
                cur_fds += len(os.listdir(fd_path))
            except (IOError, OSError):
                pass

        peak_rss = max(peak_rss, cur_rss)
        peak_anon = max(peak_anon, cur_anon)
        peak_file = max(peak_file, cur_file)
        peak_threads = max(peak_threads, cur_threads)
        peak_fds = max(peak_fds, cur_fds)

        time.sleep(0.01)
        if time.perf_counter() - t0 > timeout:
            proc.kill()
            proc.wait()
            stdout_f.seek(0)
            stderr_f.seek(0)
            stdout = stdout_f.read()
            stderr = stderr_f.read()
            stdout_f.close()
            stderr_f.close()
            return -1, stdout, "Timeout expired", time.perf_counter() - t0, peak_rss, peak_anon, peak_file, peak_fds, peak_threads

    proc.wait()
    stdout_f.seek(0)
    stderr_f.seek(0)
    stdout = stdout_f.read()
    stderr = stderr_f.read()
    stdout_f.close()
    stderr_f.close()
    elapsed = time.perf_counter() - t0

    if peak_rss == 0:
        try:
            ru = resource.getrusage(resource.RUSAGE_CHILDREN)
            mult = 1024 if sys.platform.startswith("linux") else 1
            peak_rss = ru.ru_maxrss * mult
            peak_anon = peak_rss
        except Exception:
            pass

    return proc.returncode, stdout, stderr, elapsed, peak_rss, peak_anon, peak_file, peak_fds, peak_threads


def load_corpus(corpus_path):
    p = Path(corpus_path)
    if not p.exists():
        p = p.with_suffix(".json") if p.with_suffix(".json").exists() else p.with_suffix(".tsv")
    if p.exists() and str(p).endswith(".json"):
        with open(p, "r", encoding="utf-8") as f:
            return json.load(f)
    if p.exists():
        with open(p, "r", encoding="utf-8") as f:
            lines = f.read().splitlines()
        return [
            {"repo": pt[0], "stars": int(pt[1]), "size_kb": int(pt[2]), "default_branch": pt[3], "commit": pt[4]}
            for line in lines[1:] if len(pt := line.split("\t")) >= 5
        ]
    return DEFAULT_PILOT_REPOS


def setup_repo(repo_entry, base_dir):
    repo_slug = repo_entry["repo"]
    commit_sha = repo_entry["commit"]
    repo_name = repo_slug.split("/")[-1]
    repo_dir = Path(base_dir) / repo_name

    if not repo_dir.exists():
        url = f"https://github.com/{repo_slug}.git"
        code, _, err, _ = run_cmd(["git", "clone", "--filter=blob:none", url, str(repo_dir)])
        if code != 0:
            return None, f"git clone failed: {err}"

    code, _, err, _ = run_cmd(["git", "fetch", "--all"], cwd=repo_dir)
    code, _, err, _ = run_cmd(["git", "checkout", commit_sha], cwd=repo_dir)
    if code != 0:
        return None, f"git checkout failed: {err}"

    return repo_dir, None
