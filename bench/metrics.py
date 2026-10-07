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


def is_noise_rejected(machine_metrics, max_load=4.0):
    """
    Validates machine load against isolation threshold (Point 9).
    Returns (rejected: bool, reason: str).
    """
    load1 = machine_metrics.get("loadavg_1m")
    if load1 is not None and load1 > max_load:
        return True, f"High load average ({load1} > {max_load})"
    return False, ""


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
                df.write(
                    json.dumps({
                        "repo": repo_slug,
                        "scenario": s_name,
                        "capability": sc["capability"],
                        "target": sc["target"],
                        "location": sc.get("location"),
                        "ridx": r_results.get(s_name),
                        "ra": ra_results.get(s_name),
                    })
                    + "\n"
                )
