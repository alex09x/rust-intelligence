# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Unit tests for `ridx_refusal_justified` parity classification and adjusted summary fields."""

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from position_ab_parity import evaluate_all_rows  # noqa: E402
from position_ab_parity_rules import evaluate_row  # noqa: E402

REPO = "/workspace/serde"


class PositionAbParityRefusalTests(unittest.TestCase):
    def test_rename_external_ra_edit_with_reason_is_justified(self):
        row = {"method": "rename", "file": "src/lib.rs", "line": 10, "character": 8, "symbol": "fmt"}
        r_ans = {
            "outcome": "c",
            "res": None,
            "error": "-32600",
            "error_message": "Cannot rename associated item 'fmt' of external trait: trait definition is outside the workspace",
        }
        a_ans = {
            "outcome": "a",
            "res": {
                "changes": {
                    "file:///workspace/serde/src/lib.rs": [{"range": {"start": {"line": 9, "character": 7}, "end": {"line": 9, "character": 10}}, "newText": "renamed_sym"}],
                    "file:///rust/toolchains/lib/rustlib/src/rust/library/core/src/fmt/mod.rs": [{"range": {"start": {"line": 50, "character": 7}, "end": {"line": 50, "character": 10}}, "newText": "renamed_sym"}],
                }
            },
        }
        parity, cls, ev = evaluate_row(row, r_ans, a_ans, REPO)
        self.assertEqual(parity, "mismatch")
        self.assertEqual(cls, "ridx_refusal_justified")
        self.assertEqual(ev["external_file_count"], 1)
        self.assertIn("external trait", ev["ridx_reason"])

    def test_rename_in_repo_ra_edit_is_never_justified_even_with_reason(self):
        # In-workspace refusals (e.g. entrypoint `fn main` or proc-macro coupled) must NOT be
        # folded into `ridx_refusal_justified`; they are audited separately by `rename_compile.py`.
        row = {"method": "rename", "file": "examples/demo.rs", "line": 1, "character": 4, "symbol": "main"}
        r_ans = {
            "outcome": "c",
            "res": None,
            "error": "-32600",
            "error_message": "Cannot rename Cargo executable target entrypoint 'main'",
        }
        a_ans = {
            "outcome": "a",
            "res": {
                "changes": {
                    "file:///workspace/serde/examples/demo.rs": [{"range": {"start": {"line": 0, "character": 3}, "end": {"line": 0, "character": 7}}, "newText": "renamed_sym"}],
                }
            },
        }
        parity, cls, ev = evaluate_row(row, r_ans, a_ans, REPO)
        self.assertEqual((parity, cls), ("mismatch", "p_ridx_error_or_timeout"))
        self.assertIn("entrypoint", ev["ridx_reason"])

    def test_timeout_or_missing_reason_is_not_justified(self):
        row = {"method": "rename", "file": "src/lib.rs", "line": 10, "character": 8, "symbol": "fmt"}
        ra_ext = {
            "outcome": "a",
            "res": {"changes": {"file:///cargo/registry/src/itoa-1.0.0/src/lib.rs": [{"range": {"start": {"line": 1, "character": 0}, "end": {"line": 1, "character": 3}}, "newText": "x"}]}},
        }
        for bad_r in (
            {"outcome": "c", "res": None, "error": "TIMEOUT"},
            {"outcome": "c", "res": None, "error": "-32600", "error_message": ""},
            {"outcome": "b", "res": None, "error": None},
        ):
            parity, cls, _ = evaluate_row(row, bad_r, ra_ext, REPO)
            self.assertEqual(parity, "mismatch")
            self.assertNotEqual(cls, "ridx_refusal_justified")

    def test_prepare_rename_probe_and_strict_vs_adjusted_summary(self):
        rows = [
            {
                "method": "prepare_rename", "file": "src/lib.rs", "line": 4, "character": 8, "symbol": "helper",
                "_raw": {
                    "ridx1": {"outcome": "a", "res": {"range": {"start": {"line": 3, "character": 7}, "end": {"line": 3, "character": 13}}, "placeholder": "helper"}, "us": 90.0, "error": None},
                    "ra": {"outcome": "a", "res": {"range": {"start": {"line": 3, "character": 7}, "end": {"line": 3, "character": 13}}, "placeholder": "helper"}, "us": 70.0, "error": None},
                },
            },
            {
                "method": "prepare_rename", "file": "src/lib.rs", "line": 12, "character": 8, "symbol": "fmt",
                "_raw": {
                    "ridx1": {"outcome": "c", "res": None, "us": 95.0, "error": "-32600", "error_message": "Cannot rename associated item 'fmt' of external trait"},
                    "ra": {
                        "outcome": "a",
                        "res": {"range": {"start": {"line": 11, "character": 7}, "end": {"line": 11, "character": 10}}, "placeholder": "fmt"},
                        "us": 75.0,
                        "error": None,
                        "ra_rename_probe": {
                            "changes": {
                                "file:///rustlib/src/rust/library/core/src/fmt/mod.rs": [{"range": {"start": {"line": 1, "character": 0}, "end": {"line": 1, "character": 3}}, "newText": "renamed_sym"}],
                            }
                        },
                    },
                },
            },
        ]
        summary = evaluate_all_rows(rows, "ridx1", "ra", REPO, None)
        m = summary["methods"]["prepare_rename"]
        self.assertEqual(m["ra_non_empty"], 2)
        self.assertEqual(m["matched_on_ra_non_empty"], 1)
        self.assertEqual(m["matched_on_ra_non_empty_pct"], 50.0)
        self.assertEqual(m["ridx_refusal_justified"], 1)
        self.assertEqual(m["adjusted_matched_on_ra_non_empty"], 2)
        self.assertEqual(m["adjusted_on_ra_non_empty_pct"], 100.0)


if __name__ == "__main__":
    unittest.main()
