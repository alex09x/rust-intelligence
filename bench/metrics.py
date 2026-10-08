#!/usr/bin/env python3
"""
rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
Copyright (c) 2026 Alexander Panasenko

Contact: alex@prod.codes
Author: https://prod.codes/about/
Project: https://github.com/alex09x/rust-intelligence
SPDX-License-Identifier: MIT OR Apache-2.0

Metrics collection, Prometheus queries, and discrepancy logging for benchmarks.
"""

import json
import math
import os
import urllib.parse
import urllib.request


def percentile(vals, p):
    if not vals:
        return 0.0
    s = sorted(vals)
    k = (len(s) - 1) * (p / 100.0)
    f = int(k)
    c = min(f + 1, len(s) - 1)
    d = k - f
    return s[f] * (1.0 - d) + s[c] * d


def get_local_load_avg():
    try:
        return os.getloadavg()
    except Exception:
        return (0.0, 0.0, 0.0)


class PrometheusCollector:
    """Collects machine-level metrics (CPU, RAM, load) from Prometheus or local fallback."""

    def __init__(self, endpoint=None, instance="ram9"):
        self.endpoint = (endpoint or os.environ.get("PROMETHEUS_URL", "")).rstrip("/")
        self.instance = instance

    def query_instant(self, query):
        if not self.endpoint:
            return None
        try:
            url = f"{self.endpoint}/api/v1/query?{urllib.parse.urlencode({'query': query})}"
            req = urllib.request.Request(url, headers={"Accept": "application/json"})
            with urllib.request.urlopen(req, timeout=5) as resp:
                data = json.loads(resp.read().decode("utf-8"))
                if data.get("status") == "success":
                    results = data.get("data", {}).get("result", [])
                    if results:
                        return float(results[0].get("value", [0, 0])[1])
        except Exception:
            pass
        return None

    def collect_machine_metrics(self, start_ts, end_ts):
        """Returns machine metrics dictionary from Prometheus if available, else local fallback."""
        if self.endpoint:
            inst_pat = f".*{self.instance}.*"
            load1 = self.query_instant(f'node_load1{{instance=~"{inst_pat}"}}')
            cpu_idle = self.query_instant(
                f'avg(rate(node_cpu_seconds_total{{mode="idle",instance=~"{inst_pat}"}}[1m])) * 100'
            )
            mem_total = self.query_instant(f'node_memory_MemTotal_bytes{{instance=~"{inst_pat}"}}')
            mem_avail = self.query_instant(f'node_memory_MemAvailable_bytes{{instance=~"{inst_pat}"}}')

            if load1 is not None or cpu_idle is not None:
                cpu_busy = round(100.0 - cpu_idle, 2) if cpu_idle is not None else None
                mem_used_mb = None
                if mem_total is not None and mem_avail is not None:
                    mem_used_mb = round((mem_total - mem_avail) / (1024 * 1024), 2)
                return {
                    "source": "prometheus",
                    "instance": self.instance,
                    "loadavg_1m": load1,
                    "cpu_busy_pct": cpu_busy,
                    "mem_used_mb": mem_used_mb,
                    "duration_sec": round(end_ts - start_ts, 2),
                }

        # Fallback to local system metrics
        load = get_local_load_avg()
        return {
            "source": "local_fallback",
            "loadavg_1m": load[0],
            "loadavg_5m": load[1],
            "loadavg_15m": load[2],
            "duration_sec": round(end_ts - start_ts, 2),
        }


def is_noise_rejected(machine_metrics, max_load=None):
    """
    Validates machine load against isolation threshold (Point 9).
    Returns (rejected: bool, reason: str).
    """
    if max_load is None:
        cpus = os.cpu_count() or 4
        max_load = max(4.0, cpus * 0.5)
    load1 = machine_metrics.get("loadavg_1m")
    if load1 is not None and load1 > max_load:
        return True, f"High load average ({load1} > {max_load})"
    return False, ""


def classify_discrepancy(r_res, ra_res, target="", location=None):
    """
    Classifies discrepancy between RIDX and rust-analyzer into fine-grained data-driven categories:
    - 'missing_in_ra': RIDX found target, RA returned null / Not found
    - 'associated_type_binding': Associated type constraint in trait bounds (e.g. Value, Error, Ok)
    - 'missing_in_ridx': RA found target, RIDX returned null / Not found
    - 'core_vs_std_reexport': Class D: Canonical definition in core/alloc vs std/local facade re-export
    - 'multi_version_dependency': Multi-version dependency resolution (e.g. syn-2 vs syn-3)
    - 'same_file_decl_vs_impl': Class C: Trait Default Method vs Declaration (semantically equivalent, valid location according to "points to where defined", NOT an error)
    - 'line_level_match': Class B: Same file, within 2 lines tolerance
    - 'divergent_target': Different symbols/files
    """
    r_found = bool(r_res and r_res.get("found"))
    ra_found = bool(ra_res and ra_res.get("found"))
    if r_found and not ra_found:
        return "missing_in_ra"
    if not r_found and ra_found:
        if target in ("Value", "Error", "Ok", "Variant", "Item"):
            return "associated_type_binding"
        return "missing_in_ridx"
    if not r_found and not ra_found:
        return "both_not_found"

    r_span = (r_res.get("span") or "").strip()
    ra_span = (ra_res.get("span") or "").strip()

    is_r_core = "/library/core/" in r_span or "/library/alloc/" in r_span
    is_ra_std_or_local = not is_r_core and ("/library/std/" in ra_span or not ra_span.startswith("/home/alex09x/.rustup/"))
    is_ra_core = "/library/core/" in ra_span or "/library/alloc/" in ra_span
    is_r_std_or_local = not is_ra_core and ("/library/std/" in r_span or not r_span.startswith("/home/alex09x/.rustup/"))

    if (is_r_core and is_ra_std_or_local) or (is_ra_core and is_r_std_or_local):
        return "core_vs_std_reexport"

    import re
    r_pkg = re.search(r"/([a-zA-Z0-9_\-]+)-([0-9]+\.[0-9]+\.[0-9]+)/", r_span)
    ra_pkg = re.search(r"/([a-zA-Z0-9_\-]+)-([0-9]+\.[0-9]+\.[0-9]+)/", ra_span)
    if r_pkg and ra_pkg:
        if r_pkg.group(1) == ra_pkg.group(1) and r_pkg.group(2) != ra_pkg.group(2):
            return "multi_version_dependency"

    if ":" in r_span and ":" in ra_span:
        r_parts = r_span.rsplit(":", 2)
        ra_parts = ra_span.rsplit(":", 2)
        if len(r_parts) >= 2 and len(ra_parts) >= 2 and r_parts[0] == ra_parts[0]:
            try:
                r_line = int(r_parts[1])
                ra_line = int(ra_parts[1])
                if abs(r_line - ra_line) <= 2:
                    return "line_level_match"
                else:
                    return "same_file_decl_vs_impl"
            except ValueError:
                pass

    return "divergent_target"


def analyze_discrepancies(discrepancies):
    """Aggregates discrepancy records into categorized counts."""
    from collections import Counter
    return dict(Counter(d.get("classification", "unknown") for d in discrepancies))


def record_discrepancies(discrepancies_path, repo_slug, scenarios, matches, ridx_metrics, ra_metrics):
    """Appends discrepancy records to JSONL when ridx and rust-analyzer results diverge."""
    if not discrepancies_path:
        return
    r_results = ridx_metrics.get("query_results", {})
    ra_results = ra_metrics.get("query_results", {})
    with open(discrepancies_path, "a", encoding="utf-8") as df:
        for sc in scenarios:
            s_name = sc["name"]
            if not matches.get(s_name, False):
                r_res = r_results.get(s_name)
                ra_res = ra_results.get(s_name)
                df.write(
                    json.dumps({
                        "repo": repo_slug,
                        "scenario": s_name,
                        "capability": sc["capability"],
                        "target": sc["target"],
                        "location": sc.get("location"),
                        "classification": classify_discrepancy(r_res, ra_res, target=sc.get("target", ""), location=sc.get("location")),
                        "ridx": r_res,
                        "ra": ra_res,
                    })
                    + "\n"
                )


def wilson_score_interval(successes, total, z=1.96):
    """
    Computes Wilson score confidence interval for a binomial proportion.
    Default z=1.96 corresponds to a 95% confidence level.
    Returns (center_pct, lower_pct, upper_pct).
    """
    if total <= 0:
        return 0.0, 0.0, 0.0
    p = successes / total
    denom = 1.0 + (z * z) / total
    center = (p + (z * z) / (2.0 * total)) / denom
    margin = (z / denom) * math.sqrt((p * (1.0 - p)) / total + (z * z) / (4.0 * total * total))
    lower = max(0.0, center - margin)
    upper = min(1.0, center + margin)
    return center * 100.0, lower * 100.0, upper * 100.0


def format_ratio_with_ci(successes, total, z=1.96):
    """Formats ratio with percentage and 95% Wilson confidence interval."""
    if total <= 0:
        return "0 / 0 (N/A)"
    pct = (successes / total) * 100.0
    _, lower, upper = wilson_score_interval(successes, total, z=z)
    return f"{successes} / {total} ({pct:.1f}%, 95% CI [{lower:.1f}%, {upper:.1f}%])"


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

    import subprocess
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

