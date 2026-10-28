# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Unit tests for `position_ab_parity_norm.py`, `position_ab_parity_rules.py`, and
`position_ab_parity.py`."""

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from position_ab_parity import evaluate_all_rows  # noqa: E402
from position_ab_parity_norm import (  # noqa: E402
    clean_path, normalize_completion, normalize_hover, normalize_locations, normalize_references,
    normalize_rename, normalize_symbols,
)
from position_ab_parity_rules import evaluate_row  # noqa: E402

REPO = "/workspace/serde"


class TestPositionAbParityNorm(unittest.TestCase):
    def test_clean_path_normalizes_workspace_and_sysroot(self):
        self.assertEqual(clean_path("file:///workspace/serde/src/lib.rs", REPO), "ws:src/lib.rs")
        self.assertEqual(clean_path("file:///library/core/src/option.rs", REPO), "sysroot:library/core/src/option.rs")
        self.assertEqual(
            clean_path("file:///opt/rustup/lib/rustlib/src/rust/library/std/src/path.rs", REPO),
            "sysroot:library/std/src/path.rs",
        )

    def test_normalize_locations_location_vs_location_link(self):
        ridx_loc = {
            "uri": "file:///workspace/serde/src/lib.rs",
            "range": {"start": {"line": 9, "character": 4}, "end": {"line": 9, "character": 11}},
        }
        ra_link = [{
            "targetUri": "file:///workspace/serde/src/lib.rs",
            "targetRange": {"start": {"line": 5, "character": 0}, "end": {"line": 12, "character": 1}},
            "targetSelectionRange": {"start": {"line": 9, "character": 4}, "end": {"line": 9, "character": 11}},
        }]
        nr = normalize_locations(ridx_loc, REPO)
        na = normalize_locations(ra_link, REPO)
        self.assertEqual(nr["targets"], [("ws:src/lib.rs", 10)])
        self.assertEqual(na["targets"], [("ws:src/lib.rs", 10)])
        self.assertEqual(na["full_ranges"], [("ws:src/lib.rs", 6, 13)])

    def test_normalize_references_and_highlights(self):
        refs = [
            {"uri": "file:///workspace/serde/src/a.rs", "range": {"start": {"line": 1, "character": 2}, "end": {"line": 1, "character": 5}}},
            {"uri": "file:///library/core/src/option.rs", "range": {"start": {"line": 9, "character": 0}, "end": {"line": 9, "character": 4}}},
        ]
        norm = normalize_references(refs, REPO)
        self.assertEqual(norm["ws_lines"], [("ws:src/a.rs", 2)])
        self.assertEqual(len(norm["spans"]), 2)

    def test_normalize_hover_extracts_decl_and_type(self):
        ridx_h = {"contents": {"kind": "markdown", "value": "```rust\nfn unwrap_err(self) -> E\nType: Error\n```"}}
        ra_h = {
            "contents": {
                "kind": "markdown",
                "value": "```rust\ncore::result::Result\n```\n\n```rust\npub fn unwrap_err(self) -> E\n```\n---\nDocs",
            }
        }
        nr, na = normalize_hover(ridx_h), normalize_hover(ra_h)
        self.assertEqual(nr["decl"], ("fn", "unwrap_err"))
        self.assertEqual(na["decl"], ("fn", "unwrap_err"))
        self.assertEqual(nr["sig_line"], na["sig_line"])
        self.assertEqual(nr["type_line"], "Error")

    def test_normalize_completion_strips_trait_suffix(self):
        items = [{"label": "try_into(as TryInto)"}, {"label": "len"}, {"label": "println!"}]
        norm = normalize_completion(items, top_n=2)
        self.assertEqual(norm["labels"], ["len", "println", "try_into"])
        self.assertEqual(norm["top_n"], ["try_into", "len"])

    def test_normalize_rename_and_symbols(self):
        edit_changes = {"changes": {"file:///workspace/serde/src/lib.rs": [
            {"range": {"start": {"line": 0, "character": 3}, "end": {"line": 0, "character": 6}}, "newText": "bar"}
        ]}}
        edit_doc = {"documentChanges": [{"textDocument": {"uri": "file:///workspace/serde/src/lib.rs"}, "edits": [
            {"range": {"start": {"line": 0, "character": 3}, "end": {"line": 0, "character": 6}}, "newText": "bar"}
        ]}]}
        self.assertEqual(normalize_rename(edit_changes, REPO), normalize_rename(edit_doc, REPO))
        syms = [{"name": "Foo", "kind": 23, "selectionRange": {"start": {"line": 4, "character": 0}}, "children": [
            {"name": "bar", "kind": 6, "selectionRange": {"start": {"line": 6, "character": 4}}}
        ]}]
        self.assertEqual(normalize_symbols(syms), [("Foo", 23, 5), ("bar", 6, 7)])


class TestPositionAbParityRules(unittest.TestCase):
    def test_definition_exact_and_subset_is_partial(self):
        row = {"method": "definition", "file": "src/lib.rs", "line": 10, "character": 5, "symbol": "Foo", "cursor_kind": "type_usage", "target_kind": "lib"}
        r_ans = {"outcome": "a", "res": {"uri": "file:///workspace/serde/src/lib.rs", "range": {"start": {"line": 1, "character": 0}, "end": {"line": 1, "character": 3}}}}
        a_ans = {"outcome": "a", "res": [{"targetUri": "file:///workspace/serde/src/lib.rs", "targetSelectionRange": {"start": {"line": 1, "character": 0}}}]}
        parity, cls, _ = evaluate_row(row, r_ans, a_ans, REPO)
        self.assertEqual((parity, cls), ("exact_match", None))

        a_multi = {"outcome": "a", "res": [
            {"targetUri": "file:///workspace/serde/src/lib.rs", "targetSelectionRange": {"start": {"line": 1, "character": 0}}},
            {"targetUri": "file:///workspace/serde/src/macro.rs", "targetSelectionRange": {"start": {"line": 50, "character": 0}}},
        ]}
        parity, cls, _ = evaluate_row(row, r_ans, a_multi, REPO)
        self.assertEqual((parity, cls), ("partial", "c_nav_subset_of_ra_targets"))

    def test_hover_negative_same_name_different_kind_and_variant_header(self):
        row = {"method": "hover", "file": "src/lib.rs", "line": 4, "character": 5, "symbol": "foo", "cursor_kind": "ident", "target_kind": "lib"}
        r_fn = {"outcome": "a", "res": {"contents": {"kind": "markdown", "value": "```rust\nfn foo()\n```"}}}
        a_mod = {"outcome": "a", "res": {"contents": {"kind": "markdown", "value": "```rust\npub mod foo\n```"}}}
        self.assertEqual(evaluate_row(row, r_fn, a_mod, REPO)[:2], ("mismatch", "p_hover_different_decl_head"))

        row_var = {"method": "hover", "file": "src/lib.rs", "line": 8, "character": 5, "symbol": "Seq", "cursor_kind": "ident", "target_kind": "lib"}
        r_var = {"outcome": "a", "res": {"contents": {"kind": "markdown", "value": "```rust\nvariant Seq\n```"}}}
        a_var = {"outcome": "a", "res": {"contents": {"kind": "markdown", "value": "```rust\ncrate::Value\n```\n\n```rust\nSeq(Vec<Value>)\n```"}}}
        self.assertEqual(evaluate_row(row_var, r_var, a_var, REPO)[:2], ("mismatch", "p_hover_variant_header"))

    def test_completion_single_label_overlap_is_partial_not_core_match(self):
        row = {"method": "completion", "file": "src/lib.rs", "line": 12, "character": 8, "symbol": "len", "cursor_kind": "method_call", "target_kind": "lib"}
        r_one = {"outcome": "a", "res": [{"label": "len"}] + [{"label": f"junk_{i}"} for i in range(19)]}
        a_many = {"outcome": "a", "res": [{"label": "len"}] + [{"label": f"ra_item_{i}"} for i in range(19)]}
        parity, cls, ev = evaluate_row(row, r_one, a_many, REPO)
        self.assertEqual((parity, cls), ("partial", "p_comp_partial_target_present"))
        self.assertAlmostEqual(ev["ra_label_recall"], 0.05)

        r_high = {"outcome": "a", "res": [{"label": "len"}] + [{"label": f"ra_item_{i}"} for i in range(17)]}
        parity_hi, cls_hi, _ = evaluate_row(row, r_high, a_many, REPO)
        self.assertEqual((parity_hi, cls_hi), ("core_match", "c_comp_label_set_parity"))

    def test_ridx_only_and_b_to_a_predicates_and_unclassified_guard(self):
        row_build = {"method": "definition", "file": "serde/build.rs", "line": 5, "character": 5, "symbol": "var", "cursor_kind": "let", "target_kind": "custom-build"}
        r_b = {"outcome": "b", "res": None}
        a_a = {"outcome": "a", "res": [{"targetUri": "file:///library/std/src/env.rs", "targetSelectionRange": {"start": {"line": 10, "character": 0}}}]}
        self.assertEqual(evaluate_row(row_build, r_b, a_a, REPO)[:2], ("mismatch", "p_target_custom_build"))
        self.assertEqual(evaluate_row(row_build, a_a, r_b, REPO)[:2], ("ridx_only", "c_ridx_answers_ra_empty_or_err"))

        row_assoc = {"method": "definition", "file": "src/de.rs", "line": 2, "character": 12, "symbol": "Error", "cursor_kind": "path", "target_kind": "lib"}
        self.assertEqual(evaluate_row(row_assoc, r_b, a_a, REPO, lambda _: ["", "type E = D::Error;"])[:2], ("mismatch", "p_b_to_a_type_relative_assoc_path"))
        row_self_assoc = {**row_assoc, "character": 15}
        self.assertEqual(evaluate_row(row_self_assoc, r_b, a_a, REPO, lambda _: ["", "type E = Self::Error;"])[:2], ("mismatch", "p_b_to_a_self_assoc_path"))

        # Cursor column disambiguates when both Self::Error and serde::de::Error appear on the same line.
        mixed_line = "type E = (Self::Error, serde::de::Error);"
        row_on_mod_qual = {**row_assoc, "character": 34}
        self.assertEqual(evaluate_row(row_on_mod_qual, r_b, a_a, REPO, lambda _: ["", mixed_line])[:2], ("mismatch", "p_b_to_a_qualified_path_cursor"))

        # Without a matching predicate, an unexplained b>a row must remain unclassified.
        row_unknown = {"method": "definition", "file": "src/de.rs", "line": 2, "character": 8, "symbol": "mystery", "cursor_kind": "ident", "target_kind": "lib"}
        self.assertEqual(evaluate_row(row_unknown, r_b, a_a, REPO, lambda _: ["", "let x = mystery;"])[:2], ("mismatch", "unclassified"))

    def test_raw_string_unstored_file_cfg_gate_and_macro_predicates(self):
        r_b = {"outcome": "b", "res": None}
        a_a = {"outcome": "a", "res": [{"targetUri": "file:///workspace/serde/src/lib.rs", "targetSelectionRange": {"start": {"line": 1, "character": 0}}}]}
        row_raw = {"method": "references", "file": "src/tests.rs", "line": 3, "character": 7, "symbol": "Foo", "cursor_kind": "decl:struct", "target_kind": "lib"}
        raw_lines = ["fn t() {", '    check(r#"', "struct Foo;", '"#);', "}"]
        self.assertEqual(evaluate_row(row_raw, r_b, a_a, REPO, lambda _: raw_lines)[:2], ("mismatch", "p_b_to_a_raw_string_fixture"))

        # Ordinary string ending in 'r"' must NOT trigger raw-string detection on later lines.
        non_raw_lines = ['let s = "color";', "let x = Foo;", 'let end = "r";']
        self.assertEqual(evaluate_row(row_raw, r_b, a_a, REPO, lambda _: non_raw_lines)[:2], ("mismatch", "unclassified"))

        # Unstored workspace file when ws_table is provided
        ws_table = [("shard0", "src/lib.rs")]
        row_unstored = {"method": "definition", "file": "examples/demo.rs", "line": 2, "character": 4, "symbol": "Res", "cursor_kind": "type_usage", "target_kind": "example"}
        self.assertEqual(evaluate_row(row_unstored, r_b, a_a, REPO, lambda _: ["", "let x: Res;"], ws_table)[:2], ("mismatch", "p_b_to_a_unstored_workspace_file"))

        # #[cfg(...)] gated item (non-test)
        row_cfg = {"method": "references", "file": "src/lib.rs", "line": 3, "character": 10, "symbol": "Opt", "cursor_kind": "type_usage", "target_kind": "lib"}
        cfg_lines = ["struct S {", "    #[cfg(tokio_unstable)]", "    field: Opt,", "}"]
        self.assertEqual(evaluate_row(row_cfg, r_b, a_a, REPO, lambda _: cfg_lines, ws_table)[:2], ("mismatch", "p_b_to_a_cfg_gated_item"))

        # Registry version skew and macro call target in navigation (using /opt/ws/ instead of /home/)
        row_ver = {"method": "definition", "file": "src/lib.rs", "line": 2, "character": 4, "symbol": "Lit", "cursor_kind": "path", "target_kind": "lib"}
        r_v1 = {"outcome": "a", "res": {"uri": "file:///opt/ws/.cargo/registry/src/idx/syn-3.0.8/src/lit.rs", "range": {"start": {"line": 31, "character": 0}}}}
        a_v2 = {"outcome": "a", "res": [{"targetUri": "file:///opt/ws/.cargo/registry/src/idx/syn-3.0.6/src/lit.rs", "targetSelectionRange": {"start": {"line": 31, "character": 0}}}]}
        self.assertEqual(evaluate_row(row_ver, r_v1, a_v2, REPO)[:2], ("mismatch", "p_nav_registry_crate_version_skew"))

        # Macro call target requires cursor column on the macro token itself, not elsewhere on the line.
        r_m1 = {"outcome": "a", "res": {"uri": "file:///workspace/serde/src/a.rs", "range": {"start": {"line": 1, "character": 0}}}}
        a_m2 = {"outcome": "a", "res": [{"targetUri": "file:///workspace/serde/src/b.rs", "targetSelectionRange": {"start": {"line": 9, "character": 0}}}]}
        row_mac_on = {"method": "definition", "file": "src/lib.rs", "line": 2, "character": 16, "symbol": "join", "cursor_kind": "let", "target_kind": "lib"}
        row_mac_off = {**row_mac_on, "character": 4}
        mac_line = ["", "let join = foo::join!(x);"]
        self.assertEqual(evaluate_row(row_mac_on, r_m1, a_m2, REPO, lambda _: mac_line)[:2], ("mismatch", "p_nav_macro_call_target"))
        self.assertEqual(evaluate_row(row_mac_off, r_m1, a_m2, REPO, lambda _: mac_line)[:2], ("mismatch", "unclassified"))

    def test_references_all_ra_only_lines_checked_without_slice_and_mixed_ratio(self):
        row = {"method": "references", "file": "src/lib.rs", "line": 1, "character": 4, "symbol": "visit_u8", "cursor_kind": "fn_def", "target_kind": "lib"}
        loc = lambda ln: {"uri": "file:///workspace/serde/src/lib.rs", "range": {"start": {"line": ln - 1, "character": 0}, "end": {"line": ln - 1, "character": 8}}}
        r_ans = {"outcome": "a", "res": [loc(1)]}
        a_ans = {"outcome": "a", "res": [loc(i) for i in range(1, 14)]}
        all_decl_file = ["fn visit_u8() {}"] * 13
        parity, cls, ev = evaluate_row(row, r_ans, a_ans, REPO, lambda _: all_decl_file)
        self.assertEqual((parity, cls), ("partial", "p_refs_trait_impl_sibling_decl"))
        self.assertEqual(ev["ra_only_total"], 12)
        self.assertEqual(ev["decl_lines"], 12)
        self.assertEqual(ev["decl_ratio"], 1.0)

        # If the 12th ra_only line (beyond [:10]) is a call site, ratio < 1.0 -> mixed, not sibling_decl.
        mixed_file = ["fn visit_u8() {}"] * 12 + ["v.visit_u8();"]
        parity_m, cls_m, ev_m = evaluate_row(row, r_ans, a_ans, REPO, lambda _: mixed_file)
        self.assertEqual((parity_m, cls_m), ("partial", "p_refs_partial_mixed_decl_and_usages"))
        self.assertEqual(ev_m["decl_lines"], 11)
        self.assertAlmostEqual(ev_m["decl_ratio"], 11 / 12, places=4)

    def test_evaluate_all_rows_excludes_partial_and_ridx_only_from_parity_rate(self):
        rows = [
            {
                "method": "definition", "file": "src/lib.rs", "line": 1, "character": 1, "symbol": "Foo",
                "cursor_kind": "type_usage", "target_kind": "lib",
                "_raw": {
                    "ridx": {"outcome": "a", "res": {"uri": "file:///workspace/serde/src/lib.rs", "range": {"start": {"line": 1, "character": 0}}}, "us": 10.0, "error": None},
                    "ra": {"outcome": "a", "res": [
                        {"targetUri": "file:///workspace/serde/src/lib.rs", "targetSelectionRange": {"start": {"line": 1, "character": 0}}},
                        {"targetUri": "file:///workspace/serde/src/macro.rs", "targetSelectionRange": {"start": {"line": 9, "character": 0}}},
                    ], "us": 50.0, "error": None},
                },
            },
            {
                "method": "definition", "file": "src/lib.rs", "line": 2, "character": 1, "symbol": "Bar",
                "cursor_kind": "ident", "target_kind": "lib",
                "_raw": {
                    "ridx": {"outcome": "a", "res": {"uri": "file:///workspace/serde/src/lib.rs", "range": {"start": {"line": 5, "character": 0}}}, "us": 5.0, "error": None},
                    "ra": {"outcome": "b", "res": None, "us": 20.0, "error": None},
                },
            },
        ]
        summary = evaluate_all_rows(rows, "ridx", "ra", REPO, None)
        self.assertEqual(summary["unclassified"], 0)
        self.assertEqual(summary["methods"]["definition"]["partial"], 1)
        self.assertEqual(summary["methods"]["definition"]["ridx_only"], 1)
        self.assertEqual(summary["methods"]["definition"]["matched_on_ra_non_empty"], 0)
        self.assertEqual(summary["methods"]["definition"]["matched_on_ra_non_empty_pct"], 0.0)


if __name__ == "__main__":
    unittest.main()
