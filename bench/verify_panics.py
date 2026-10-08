#!/usr/bin/env python3
"""
rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
Copyright (c) 2026 Alexander Panasenko

Contact: alex@prod.codes
Author: https://prod.codes/about/
Project: https://github.com/alex09x/rust-intelligence
SPDX-License-Identifier: MIT OR Apache-2.0

Standalone script to verify compiler ICE and caught panics on heavy generic crates.
"""

import json
import os
import subprocess
import sys
import time
from pathlib import Path

REPOS = [
    ("diesel", "https://github.com/diesel-rs/diesel.git", "d8568c0d851c1db3e377c103542714655c58c0f9"),
    ("tantivy", "https://github.com/quickwit-oss/tantivy.git", "1783018f9e6c0ffa4a884df3812011d8d13e4b52"),
    ("gitoxide", "https://github.com/Byron/gitoxide.git", "9272d45a1b08ca6f26779389954bd718f8308a80"),
    ("sqlx", "https://github.com/launchbadge/sqlx.git", "8b65c2fb42a3a523b77000f41b648986d1d49ba0"),
    ("wasmtime", "https://github.com/bytecodealliance/wasmtime.git", "ddf01e51152c521ae0217e3f5fc1c69e120d0ab1"),
    ("burn", "https://github.com/tracel-ai/burn.git", "da05a1980e4d7170697ff795819aca61c3684dcb"),
    ("datafusion", "https://github.com/apache/datafusion.git", "db83fccb9ac10f843be04daa5826501d2338caf4"),
    ("polars", "https://github.com/pola-rs/polars.git", "af3e850f441f056585e1e9663a211f152dac5472"),
    ("bevy", "https://github.com/bevyengine/bevy.git", "68c80f3b19ddf9e4ae1ce7dd8b5a68273204c2e8"),
]

BASE_DIR = Path("/tmp/mitts-bench")
LOG_DIR = Path("/tmp/panic_checks")
LOG_DIR.mkdir(parents=True, exist_ok=True)
RIDX_BIN = "/tmp/rust-intelligence/target/release/ridx"


def run_verification():
    results = []
    res_file = LOG_DIR / "results.json"
    if res_file.exists():
        try:
            results = json.loads(res_file.read_text(encoding="utf-8"))
        except Exception:
            results = []

    completed_names = {r["repo"] for r in results}

    for name, url, commit in REPOS:
        if name in completed_names:
            print(f"[SKIP] {name} already verified.")
            continue

        print(f"\n═══════════════════════════════════════════════════════")
        print(f" Verifying Repo: {name} (commit {commit[:8]})")
        print(f"═══════════════════════════════════════════════════════")
        repo_dir = BASE_DIR / name

        if not repo_dir.exists():
            print(f"Cloning {name}...")
            code = subprocess.run(["git", "clone", "--filter=blob:none", url, str(repo_dir)], check=False).returncode
            if code != 0:
                print(f"Failed to clone {name}")
                continue

        subprocess.run(["git", "checkout", commit], cwd=repo_dir, check=False)

        cas_dir = f"/tmp/clean-cas/{name}"
        subprocess.run(["rm", "-rf", str(repo_dir / "target" / "ridx"), cas_dir], check=False)
        Path(cas_dir).mkdir(parents=True, exist_ok=True)

        log_path = LOG_DIR / f"{name}.log"
        cmd = ["taskset", "-c", "24-31", RIDX_BIN, "index", "."]
        env = os.environ.copy()
        env["RIDX_CACHE_DIR"] = cas_dir
        env["PKG_CONFIG_PATH"] = f"{Path.home()}/.local/lib/pkgconfig:" + env.get("PKG_CONFIG_PATH", "")

        t0 = time.time()
        with open(log_path, "w", encoding="utf-8") as lf:
            proc = subprocess.run(cmd, cwd=repo_dir, stdout=lf, stderr=subprocess.STDOUT, env=env, check=False)
        dur = time.time() - t0

        log_content = log_path.read_text(encoding="utf-8", errors="replace")
        ice_detected = "internal compiler error" in log_content.lower()

        sidecars = list(repo_dir.glob("target/**/.deps/*.json"))
        total_panics = 0
        for sc in sidecars:
            try:
                d = json.loads(sc.read_text(encoding="utf-8", errors="replace"))
                total_panics += d.get("caught_panics", 0)
            except Exception:
                pass

        crates_count = 0
        defs_count = 0
        refs_count = 0
        types_count = 0
        for line in log_content.splitlines():
            if "Indexed" in line and "crate(s)" in line:
                try:
                    parts = line.split()
                    idx_crate = parts.index("crate(s),")
                    crates_count = int(parts[idx_crate - 1])
                    idx_defs = parts.index("total")
                    defs_count = int(parts[idx_defs - 1])
                    idx_refs = parts.index("references,")
                    refs_count = int(parts[idx_refs - 1])
                    idx_types = parts.index("types")
                    types_count = int(parts[idx_types - 1])
                except Exception:
                    pass

        res = {
            "repo": name,
            "commit": commit,
            "exit_code": proc.returncode,
            "duration_sec": round(dur, 2),
            "ice_detected": ice_detected,
            "caught_panics": total_panics,
            "crates_count": crates_count,
            "defs_count": defs_count,
            "refs_count": refs_count,
            "types_count": types_count,
            "sidecars_count": len(sidecars),
        }
        results.append(res)
        print(f"--> Result {name}: exit={proc.returncode}, dur={round(dur, 1)}s, ICE={ice_detected}, caught_panics={total_panics}, defs={defs_count}, refs={refs_count}")
        res_file.write_text(json.dumps(results, indent=2), encoding="utf-8")

    print("\nAll repos verified successfully!")


if __name__ == "__main__":
    run_verification()
