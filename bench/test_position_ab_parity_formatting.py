# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Formatting parity compares the document each side's edits produce, not the edit granularity."""

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from position_ab_parity_norm_ext import normalize_formatting  # noqa: E402
from position_ab_parity_rules_ext import compare_formatting  # noqa: E402

SOURCE = "fn foo() {\n    || ..;\n  let x=1;\n}\n"
FORMATTED = "fn foo() {\n    || ..;\n    let x = 1;\n}\n"


def edit(sl, sc, el, ec, text):
    return {"range": {"start": {"line": sl, "character": sc}, "end": {"line": el, "character": ec}}, "newText": text}


class TestFormattingParity(unittest.TestCase):
    def test_whole_file_replacement_matches_minimal_diff_with_same_result(self):
        whole = normalize_formatting([edit(0, 0, 4, 0, FORMATTED)])
        minimal = normalize_formatting([edit(2, 0, 2, 2, "    "), edit(2, 7, 2, 8, " = ")])
        cls, sub, _ = compare_formatting(whole, minimal, SOURCE)
        self.assertEqual((cls, sub), ("core_match", "c_fmt_same_result_text"))

    def test_different_result_is_not_a_match(self):
        whole = normalize_formatting([edit(0, 0, 4, 0, FORMATTED.replace("x = 1", "x = 2"))])
        minimal = normalize_formatting([edit(2, 0, 2, 2, "    "), edit(2, 7, 2, 8, " = ")])
        cls, _, _ = compare_formatting(whole, minimal, SOURCE)
        self.assertEqual(cls, "mismatch")

    def test_without_source_keeps_the_old_strict_comparison(self):
        whole = normalize_formatting([edit(0, 0, 4, 0, FORMATTED)])
        minimal = normalize_formatting([edit(2, 0, 2, 2, "    "), edit(2, 7, 2, 8, " = ")])
        cls, _, _ = compare_formatting(whole, minimal)
        self.assertEqual(cls, "mismatch")


if __name__ == "__main__":
    unittest.main()
