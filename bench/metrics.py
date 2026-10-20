#!/usr/bin/env python3
"""
rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
Copyright (c) 2026 Alexander Panasenko

Contact: alex@prod.codes
Author: https://prod.codes/about/
Project: https://github.com/alex09x/rust-intelligence
SPDX-License-Identifier: MIT OR Apache-2.0

Percentiles and process telemetry (RSS, file descriptors, threads) for the benchmark harness.
"""

import os
import subprocess


def percentile(vals, p):
    if not vals:
        return 0.0
    s = sorted(vals)
    k = (len(s) - 1) * (p / 100.0)
    f = int(k)
    c = min(f + 1, len(s) - 1)
    d = k - f
    return s[f] * (1.0 - d) + s[c] * d


def get_proc_metrics(pid):
    """Retrieves RSS (bytes), FDs, and Thread count for pid on Linux and macOS."""
    rss_kb = 0
    rss_anon_kb = 0
    rss_file_kb = 0
    threads = 1
    fds = 0

    proc_status = os.path.join(f"/proc/{pid}/status")
    if os.path.exists(proc_status):
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
            fd_dir = f"/proc/{pid}/fd"
            if os.path.exists(fd_dir):
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
