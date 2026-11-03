# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Unit tests for `position_ab_parity_code_action.py`, `code_action_resolve` row prep, and `code_action_compile.py`."""

import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from code_action_compile import (  # noqa: E402
    apply_action_in_worktree,
    compute_ra_compiling_coverage,
    extract_workspace_edits,
)
from position_ab_parity_exec import prepare_parity_rows  # noqa: E402
from position_ab_parity_rules import evaluate_row  # noqa: E402

REPO = "/workspace/serde"


def _mk_edit(uri: str, new_text: str, use_dc: bool = False) -> dict:
    te = {"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}}, "newText": new_text}
    if use_dc:
        return {"documentChanges": [{"textDocument": {"uri": uri, "version": None}, "edits": [te]}]}
    return {"changes": {uri: [te]}}


class TestCodeActionParityAndCompile(unittest.TestCase):
    def test_code_action_strict_exact_and_placeholder_stub_partial(self):
        row = {"method": "code_action", "file": "src/lib.rs", "line": 4, "character": 8, "symbol": "Foo", "cursor_kind": "decl:struct", "target_kind": "lib"}
        r_exact = {"outcome": "a", "res": [{"title": "Generate `new`", "kind": ""}, {"title": "Generate impl for `Foo`", "kind": ""}]}
        a_exact = {"outcome": "a", "res": [{"title": "Generate impl for `Foo`", "kind": ""}, {"title": "Generate `new`", "kind": ""}]}
        self.assertEqual(evaluate_row(row, r_exact, a_exact, REPO)[:2], ("exact_match", None))

        a_with_stub = {"outcome": "a", "res": [
            {"title": "Generate impl for `Foo`", "kind": ""},
            {"title": "Generate trait impl for `Foo`", "kind": ""},
            {"title": "Generate `new`", "kind": ""},
        ]}
        self.assertEqual(evaluate_row(row, r_exact, a_with_stub, REPO)[:2], ("partial", "p_code_action_ra_has_placeholder_stub"))

    def test_code_action_resolve_text_aware_exact_core_and_partial(self):
        uri = "file:///workspace/serde/src/lib.rs"
        row = {"method": "code_action_resolve", "file": "src/lib.rs", "line": 4, "character": 8, "symbol": "foo", "cursor_kind": "fn_def", "target_kind": "lib"}

        # 1. Exact match after collapsing whitespace / stripping leading-trailing newlines.
        r_res = {"outcome": "a", "res": [{"title": "Generate a documentation template", "kind": "refactor", "edit": _mk_edit(uri, "\n/// .\n///\n/// # Errors\n///\n/// This function will return an error if .\n")}]}
        a_res = {"outcome": "a", "res": [{"title": "Generate a documentation template", "kind": "", "edit": _mk_edit(uri, "/// .\n///\n/// # Errors\n///\n/// This function will return an error if .\n\n", use_dc=True)}]}
        self.assertEqual(evaluate_row(row, r_res, a_res, REPO)[:2], ("exact_match", None))

        # 2. Stub doc comment vs real template must NOT be exact_match or core_match.
        r_stub = {"outcome": "a", "res": [{"title": "Generate a documentation template", "kind": "refactor", "edit": _mk_edit(uri, "/// Documentation for `foo`.\n")}]}
        self.assertEqual(evaluate_row(row, r_stub, a_res, REPO)[:2], ("partial", "p_code_action_resolve_diff_edits"))

        # 3. Same item header (`type PollFn`) with receiver/visibility detail difference -> core_match.
        r_ty = {"outcome": "a", "res": [{"title": "Generate type alias for function", "kind": "refactor", "edit": _mk_edit(uri, "\npub(crate) type PollFn = fn(Pin<&mut Client>, cx: &mut Context<'_>) -> Poll<()>;\n")}]}
        a_ty = {"outcome": "a", "res": [{"title": "Generate type alias for function", "kind": "refactor", "edit": _mk_edit(uri, "\ntype PollFn = fn(cx: &mut Context<'_>) -> Poll<()>;\n", use_dc=True)}]}
        self.assertEqual(evaluate_row(row, r_ty, a_ty, REPO)[:2], ("core_match", "c_code_action_resolve_same_headers_diff_body"))

        # 4. Injected #[allow(...)] attribute must be flagged as partial, never core_match.
        r_allow = {"outcome": "a", "res": [{"title": "Generate type alias for function", "kind": "refactor", "edit": _mk_edit(uri, "\n#[allow(dead_code)]\npub(crate) type PollFn = fn(cx: &mut Context<'_>) -> Poll<()>;\n")}]}
        self.assertEqual(evaluate_row(row, r_allow, a_ty, REPO)[:2], ("partial", "p_code_action_resolve_diff_edits"))

        # 5. Non-header expression edit with different text -> partial.
        r_str = {"outcome": "a", "res": [{"title": "Add #", "kind": "refactor", "edit": _mk_edit(uri, 'r##"hello"##')}]}
        a_str = {"outcome": "a", "res": [{"title": "Add #", "kind": "refactor", "edit": _mk_edit(uri, 'r#"hello"#')}]}
        self.assertEqual(evaluate_row(row, r_str, a_str, REPO)[:2], ("partial", "p_code_action_resolve_diff_edits"))

        r_empty = {"outcome": "b", "res": None}
        self.assertEqual(evaluate_row(row, r_empty, a_res, REPO, lambda _: ["", "", "", "pub fn foo() {}"])[:2], ("mismatch", "p_b_to_a_code_action_resolve_ra_only"))

    def test_compute_ra_compiling_coverage(self):
        cov = compute_ra_compiling_coverage([
            ({"Add #", "Remove #"}, {"Add #", "Remove #"}),
            ({"Generate `new`"}, {"Generate `new`", "Convert to tuple struct"}),
            ({"Generate impl for `Foo`"}, set()),
        ])
        self.assertEqual(cov["ra_compiling_actions"], 4)
        self.assertEqual(cov["ridx_covering_ra_compiling"], 3)
        self.assertEqual(cov["coverage_of_ra_compiling_pct"], 75.0)
        self.assertEqual(cov["ra_compiling_positions"], 2)
        self.assertEqual(cov["ridx_any_covering_positions"], 2)
        self.assertEqual(cov["ridx_full_covering_positions"], 1)
        self.assertEqual(cov["uncovered_ra_compiling_by_title"], {"Convert to tuple struct": 1})

    def test_prepare_parity_rows_hydrates_code_action_resolve_from_code_action(self):
        positions = {
            "code_action": [{"name": "code_action_0_foo", "file": "src/lib.rs", "line": 2, "character": 8, "symbol": "foo"}],
            "code_action_resolve": [{"name": "code_action_resolve_0", "file": "", "line": 0, "character": 0, "symbol": None}],
        }
        rows = [{"method": "code_action_resolve", "file": "", "line": 0, "character": 0, "symbol": None, "scenario": {"name": "code_action_resolve_0"}}]
        prepare_parity_rows(rows, positions, REPO, [("src/lib.rs", "lib")], lambda _: ["", "pub fn foo() {}"])
        self.assertEqual(rows[0]["file"], "src/lib.rs")
        self.assertEqual(rows[0]["line"], 2)
        self.assertEqual(rows[0]["symbol"], "foo")
        self.assertEqual(rows[0]["target_kind"], "lib")

    def test_apply_action_in_worktree_preserves_same_offset_insertion_order(self):
        with tempfile.TemporaryDirectory() as src, tempfile.TemporaryDirectory() as wt:
            (Path(src) / "lib.rs").write_text("pub fn demo() {}\n", encoding="utf-8")
            (Path(wt) / "lib.rs").write_text("pub fn demo() {}\n", encoding="utf-8")
            uri = f"file://{src}/lib.rs"
            edit = {"changes": {uri: [
                {"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}}, "newText": "/// line 1\n"},
                {"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}}, "newText": "/// line 2\n"},
            ]}}
            parsed = extract_workspace_edits(edit)
            self.assertIsNotNone(parsed)
            creates, eds = parsed
            self.assertTrue(apply_action_in_worktree(creates, eds, src, wt))
            self.assertEqual((Path(wt) / "lib.rs").read_text(encoding="utf-8"), "/// line 1\n/// line 2\npub fn demo() {}\n")


if __name__ == "__main__":
    unittest.main()
