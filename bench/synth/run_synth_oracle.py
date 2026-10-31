# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""CLI runner for the synthetic multi-crate Rust ground-truth oracle (`bench/synth/`)."""

import argparse
import json
import subprocess
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from synth.oracle_eval import evaluate_synth_oracle  # noqa: E402
from synth.workspace_gen import generate_synth_workspace  # noqa: E402


def compile_and_index_synth(ws_dir: Path, ridx_bin: str) -> None:
    """Verifies that the generated synthetic workspace compiles with `cargo check` and indexes it with `ridx`."""
    chk = subprocess.run(
        ["cargo", "check", "--workspace", "--tests", "--quiet"],
        cwd=str(ws_dir),
        capture_output=True,
        text=True,
        timeout=120,
    )
    if chk.returncode != 0:
        raise RuntimeError(f"Synthetic workspace failed cargo check:\n{chk.stderr}")

    idx = subprocess.run(
        [ridx_bin, "index", str(ws_dir)],
        cwd=str(ws_dir),
        capture_output=True,
        text=True,
        timeout=120,
    )
    if idx.returncode != 0:
        raise RuntimeError(f"ridx index failed on synthetic workspace:\n{idx.stderr}")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--seed", type=int, default=42, help="Deterministic RNG seed (default: 42)")
    ap.add_argument("--scale", type=int, default=3, help="Number of symbol families to generate (default: 3)")
    ap.add_argument("--workspace-dir", default=None, help="Directory to generate workspace in (default: temp dir)")
    ap.add_argument("--ridx", required=True, help="Path to ridx binary")
    ap.add_argument("--ra", default=None, help="Optional path to rust-analyzer binary")
    ap.add_argument("--timeout", type=float, default=5.0, help="Per-request timeout in seconds")
    ap.add_argument("--out", default=None, help="Optional output JSON path")
    args = ap.parse_args()

    tmp_ctx = tempfile.TemporaryDirectory(prefix=f"ridx_synth_s{args.seed}_") if not args.workspace_dir else None
    ws_dir = Path(args.workspace_dir) if args.workspace_dir else Path(tmp_ctx.name)

    try:
        manifest = generate_synth_workspace(ws_dir, seed=args.seed, scale=args.scale)
        compile_and_index_synth(ws_dir, args.ridx)
        ridx_report = evaluate_synth_oracle([args.ridx, "lsp"], manifest, is_ra=False, timeout=args.timeout)
        full_report = {"ridx": ridx_report}
        if args.ra:
            full_report["ra"] = evaluate_synth_oracle([args.ra], manifest, is_ra=True, timeout=args.timeout)

        if args.out:
            out_path = Path(args.out)
            out_path.parent.mkdir(parents=True, exist_ok=True)
            out_path.write_text(json.dumps(full_report, indent=2) + "\n", encoding="utf-8")

        print(
            f"Synth Oracle (seed={args.seed}, scale={args.scale}): "
            f"RIDX {ridx_report['total_passed']}/{ridx_report['total_cases']} ({ridx_report['overall_pass_pct']}%)"
        )
        for m, st in ridx_report["per_method"].items():
            print(f"  {m:28s}: {st['passed']:2d}/{st['total']:2d} ({st['pass_pct']:5.1f}%)")
        return 0
    finally:
        if tmp_ctx is not None:
            tmp_ctx.cleanup()


if __name__ == "__main__":
    sys.exit(main())
