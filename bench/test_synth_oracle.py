# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Unit and compilation tests for the synthetic ground-truth generator and oracle (`bench/synth/`)."""

import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from position_ab_parity import ALL_PARITY_METHODS  # noqa: E402
from synth.oracle_cases import build_oracle_cases  # noqa: E402
from synth.source_builder import SourceBuilder  # noqa: E402
from synth.workspace_gen import generate_synth_workspace  # noqa: E402


class TestSynthOracle(unittest.TestCase):
    def test_source_builder_exact_coordinates(self):
        sb = SourceBuilder("crates/demo/src/lib.rs")
        line_idx, spans = sb.line_tokens(
            "pub fn ", ("compute_x", "def", "compute_x"), "(a: i64) -> i64 { a + 1 }"
        )
        self.assertEqual(line_idx, 0)
        self.assertEqual(len(spans), 1)
        rec = spans[0]
        self.assertEqual((rec.line, rec.col, rec.end_line, rec.end_col), (0, 7, 0, 16))
        sb.verify_spans()

    def test_workspace_gen_seed_determinism_and_exact_slices(self):
        with tempfile.TemporaryDirectory() as d1, tempfile.TemporaryDirectory() as d2, tempfile.TemporaryDirectory() as d3:
            m1 = generate_synth_workspace(Path(d1), seed=42, scale=2)
            m2 = generate_synth_workspace(Path(d2), seed=42, scale=2)
            m3 = generate_synth_workspace(Path(d3), seed=99, scale=2)

            self.assertEqual(m1["symbols"], m2["symbols"])
            self.assertEqual(m1["families"], m2["families"])
            self.assertNotEqual(m1["families"], m3["families"])

            # Verify every recorded span in m1 slices to the exact text in the written file
            for sym_id, recs in m1["symbols"].items():
                for rec in recs:
                    file_lines = (Path(d1) / rec["rel_path"]).read_text(encoding="utf-8").splitlines()
                    actual = file_lines[rec["line"]][rec["col"]:rec["end_col"]]
                    self.assertEqual(actual, rec["text"], f"Mismatch for {sym_id}: {rec}")

    def test_oracle_cases_cover_all_37_methods_plus_resolves(self):
        with tempfile.TemporaryDirectory() as tmp:
            manifest = generate_synth_workspace(Path(tmp), seed=42, scale=2)
            cases = build_oracle_cases(manifest)
            methods_covered = {c["method"] for c in cases}

            for m in ALL_PARITY_METHODS:
                self.assertIn(m, methods_covered, f"Missing method {m} in build_oracle_cases")
            for extra in ("inlay_hint_resolve", "workspace_symbol_resolve", "document_link_resolve", "code_action_resolve"):
                self.assertIn(extra, methods_covered, f"Missing resolve method {extra}")

    def test_generated_workspace_compiles_with_cargo_check(self):
        with tempfile.TemporaryDirectory() as tmp:
            ws_dir = Path(tmp)
            generate_synth_workspace(ws_dir, seed=42, scale=2)
            proc = subprocess.run(
                ["cargo", "check", "--workspace", "--tests", "--quiet"],
                cwd=str(ws_dir),
                capture_output=True,
                text=True,
                timeout=60,
            )
            self.assertEqual(proc.returncode, 0, f"cargo check failed:\n{proc.stderr}")


if __name__ == "__main__":
    unittest.main()
