# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""CLI runner for the synthetic multi-crate Rust ground-truth oracle (`bench/synth/` v1)."""

import argparse
import json
import subprocess
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from synth.oracle_cases import build_oracle_cases  # noqa: E402
from synth.oracle_eval import evaluate_synth_oracle  # noqa: E402
from synth.workspace_gen import generate_synth_workspace  # noqa: E402


def verify_seed_variation() -> dict:
    """Self-test: verifies that seed=42 and seed=7 produce distinct workspace topologies and case sets."""
    with tempfile.TemporaryDirectory(prefix="ridx_synth_selftest_") as td:
        root = Path(td)
        m42 = generate_synth_workspace(root / "s42", seed=42, scale=3)
        m7 = generate_synth_workspace(root / "s7", seed=7, scale=3)
        c42 = [c["case_id"] for c in build_oracle_cases(m42)]
        c7 = [c["case_id"] for c in build_oracle_cases(m7)]
        crates_differ = m42["topology"]["members"] != m7["topology"]["members"]
        depths_differ = m42["topology"]["nest_depths"] != m7["topology"]["nest_depths"]
        cases_differ = c42 != c7
        if not (crates_differ and depths_differ and cases_differ):
            raise RuntimeError(
                f"Seed variation self-test failed: crates_differ={crates_differ}, "
                f"depths_differ={depths_differ}, cases_differ={cases_differ}"
            )
        return {
            "passed": True,
            "seed42_crates": m42["topology"]["members"],
            "seed7_crates": m7["topology"]["members"],
            "seed42_depths": m42["topology"]["nest_depths"],
            "seed7_depths": m7["topology"]["nest_depths"],
        }


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


def compute_ra_divergences(ridx_report: dict, ra_report: dict) -> list:
    """Returns cases where RIDX matched the ground-truth oracle while rust-analyzer diverged."""
    ra_by_id = {(c["method"], c["case_id"]): c for c in ra_report.get("cases", [])}
    divergences = []
    for rc in ridx_report.get("cases", []):
        ac = ra_by_id.get((rc["method"], rc["case_id"]))
        if rc["passed"] and ac and not ac["passed"]:
            divergences.append({
                "method": rc["method"],
                "case_id": rc["case_id"],
                "ridx_detail": rc["detail"],
                "ra_detail": ac["detail"],
            })
    return divergences


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--seed", type=int, default=42, help="Deterministic RNG seed (default: 42)")
    ap.add_argument("--scale", type=int, default=6, help="Number of symbol families to generate (default: 6 -> 126 rename cases)")
    ap.add_argument("--workspace-dir", default=None, help="Directory to generate workspace in (default: temp dir)")
    ap.add_argument("--ridx", required=True, help="Path to ridx binary")
    ap.add_argument("--ra", default=None, help="Optional path to rust-analyzer binary")
    ap.add_argument("--timeout", type=float, default=5.0, help="Per-request timeout in seconds")
    ap.add_argument("--out", default=None, help="Optional output JSON path")
    args = ap.parse_args()

    selftest = verify_seed_variation()
    tmp_ctx = tempfile.TemporaryDirectory(prefix=f"ridx_synth_s{args.seed}_") if not args.workspace_dir else None
    ws_dir = Path(args.workspace_dir) if args.workspace_dir else Path(tmp_ctx.name)

    try:
        manifest = generate_synth_workspace(ws_dir, seed=args.seed, scale=args.scale)
        compile_and_index_synth(ws_dir, args.ridx)
        ridx_report = evaluate_synth_oracle([args.ridx, "lsp"], manifest, is_ra=False, timeout=args.timeout)
        full_report = {"seed_variation_selftest": selftest, "ridx": ridx_report}
        if args.ra:
            ra_report = evaluate_synth_oracle([args.ra], manifest, is_ra=True, timeout=args.timeout)
            full_report["ra"] = ra_report
            full_report["ra_divergences"] = compute_ra_divergences(ridx_report, ra_report)

        if args.out:
            out_path = Path(args.out)
            out_path.parent.mkdir(parents=True, exist_ok=True)
            out_path.write_text(json.dumps(full_report, indent=2) + "\n", encoding="utf-8")

        print(
            f"Synth Oracle v1 (seed={args.seed}, scale={args.scale}): "
            f"RIDX {ridx_report['total_passed']}/{ridx_report['total_cases']} ({ridx_report['overall_pass_pct']}%)"
        )
        for m, st in ridx_report["per_method"].items():
            print(
                f"  {m:28s}: {st['passed']:3d}/{st['total']:3d} ({st['pass_pct']:5.1f}%) "
                f"[prec={st['precision_pct']:5.1f}%, rec={st['recall_pct']:5.1f}%]"
            )
        if "ra" in full_report:
            ra_r = full_report["ra"]
            divs = full_report["ra_divergences"]
            print(
                f"  RA comparison: RA {ra_r['total_passed']}/{ra_r['total_cases']} ({ra_r['overall_pass_pct']}%), "
                f"ridx_better_than_ra={len(divs)} cases"
            )
        return 0
    finally:
        if tmp_ctx is not None:
            tmp_ctx.cleanup()


if __name__ == "__main__":
    sys.exit(main())
