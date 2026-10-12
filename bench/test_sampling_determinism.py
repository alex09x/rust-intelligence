# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""
Unit test verifying deterministic scenario sampling under different PYTHONHASHSEED values.
"""

import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path


CHILD_SCRIPT = """
import json
import sys
from pathlib import Path

repo_dir = sys.argv[1]
bench_dir = sys.argv[2]
if bench_dir not in sys.path:
    sys.path.insert(0, bench_dir)

from matrix_sampling import sample_full_lsp_matrix

scenarios = sample_full_lsp_matrix(repo_dir, per_method=50, seed=42)
print(json.dumps(scenarios, sort_keys=True))
"""


def run_child(repo_dir, hash_seed, bench_dir):
    env = dict(os.environ, PYTHONHASHSEED=str(hash_seed))
    cmd = [sys.executable, "-c", CHILD_SCRIPT, str(repo_dir), str(bench_dir)]
    res = subprocess.run(cmd, capture_output=True, text=True, env=env, cwd=str(bench_dir))
    if res.returncode != 0:
        raise RuntimeError(f"Child failed under PYTHONHASHSEED={hash_seed}:\n{res.stderr}")
    return json.loads(res.stdout)


def test_sampling_determinism():
    bench_dir = Path(__file__).resolve().parent
    with tempfile.TemporaryDirectory() as td:
        src = Path(td) / "src"
        src.mkdir(parents=True)
        (src / "lib.rs").write_text("pub fn alpha() {}\npub fn beta() {}\npub struct Gamma;\n", encoding="utf-8")
        (src / "foo.rs").write_text("pub fn delta() {}\npub struct Epsilon;\npub enum Zeta { V1 }\n", encoding="utf-8")
        (src / "bar.rs").write_text("pub fn eta() {}\npub struct Theta;\n", encoding="utf-8")
        (src / "baz.rs").write_text("pub fn iota() {}\npub struct Kappa;\n", encoding="utf-8")

        res_seed1 = run_child(td, 1, bench_dir)
        res_seed2 = run_child(td, 2, bench_dir)

        all_methods = sorted(set(res_seed1.keys()) | set(res_seed2.keys()))
        mismatches = []
        for m in all_methods:
            s1 = res_seed1.get(m)
            s2 = res_seed2.get(m)
            if s1 != s2:
                mismatches.append(m)

        if mismatches:
            print(f"FAIL: {len(mismatches)} method(s) non-deterministic: {mismatches}")
        else:
            print(f"PASS: all {len(all_methods)} methods deterministic under fixed seed")

        assert not mismatches, f"non-deterministic: {mismatches}"


if __name__ == "__main__":
    try:
        test_sampling_determinism()
    except AssertionError:
        sys.exit(1)
    sys.exit(0)

