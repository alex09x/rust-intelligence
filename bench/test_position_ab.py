# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""
Unit tests for position_ab: scenarios rebuilt from positions records give the matrix's requests,
the outcome rule matches the matrix, completion digests ignore item order, the target kind and
cursor kind heuristics classify fixed inputs, and the server comparison counts fixed rows.
"""

import sys
from pathlib import Path

bench_dir = Path(__file__).resolve().parent
if str(bench_dir) not in sys.path:
    sys.path.insert(0, str(bench_dir))

from matrix_categories import build_params
from position_ab import (
    binary_label, build_rows, compare_servers, differing_only, matrix_check, row_differs, summarize_server,
)
from position_ab_classify import (
    REQUEST_KEYS, classify_outcome, cursor_kind, digest, in_cfg_test, scenario_from_record, target_kind_of,
)

REPO = "/r"


def test_request_keys_exclude_lifecycle_notifications_and_document_sync():
    assert "definition" in REQUEST_KEYS and "code_lens" in REQUEST_KEYS and "workspace_symbol" in REQUEST_KEYS
    for key in ("initialize", "did_open", "will_save_wait", "cancel_request", "mutation_case_b"):
        assert key not in REQUEST_KEYS


def test_scenario_rebuilds_the_matrix_request_for_a_position():
    rec = {"name": "definition_7_from", "file": "src/de.rs", "line": 1983, "character": 18, "symbol": "from"}
    sc = scenario_from_record("definition", rec, REPO)
    assert sc["location"] == ["/r/src/de.rs", 1983, 18] and sc["target"] == "from"
    params = build_params("definition", sc, REPO)
    assert params == {"textDocument": {"uri": "file:///r/src/de.rs"}, "position": {"line": 1982, "character": 17}}


def test_scenario_recovers_query_new_name_and_prev_id():
    ws = scenario_from_record("workspace_symbol", {"name": "ws_sym_12_split_for_impl", "file": "", "line": 0, "character": 0, "symbol": None}, REPO)
    assert ws["query"] == "split_for_impl" and "location" not in ws
    assert build_params("workspace_symbol", ws, REPO) == {"query": "split_for_impl"}
    rn = scenario_from_record("rename", {"name": "rename_3_to_vec", "file": "src/lib.rs", "line": 5, "character": 8, "symbol": "to_vec"}, REPO)
    assert rn["new_name"] == "to_vec_renamed_3"
    assert build_params("rename", rn, REPO)["newName"] == "to_vec_renamed_3"
    pr = scenario_from_record("prepare_rename", {"name": "prepare_rename_4_Foo", "file": "src/a.rs", "line": 1, "character": 1, "symbol": "Foo"}, REPO)
    assert pr["new_name"] == "Foo_renamed_4"
    st = scenario_from_record("semantic_tokens_delta", {"name": "semantic_tokens_delta_9_lib.rs", "file": "src/lib.rs", "line": 1, "character": 1, "symbol": None}, REPO)
    assert st["prev_id"] == "id_9"
    assert build_params("semantic_tokens_delta", st, REPO)["previousResultId"] == "id_9"
    assert build_params("incoming_calls", scenario_from_record(
        "incoming_calls", {"name": "incoming_calls_0_run", "file": "src/m.rs", "line": 10, "character": 4, "symbol": "run"}, REPO), REPO)["item"]["name"] == "run"


def test_outcome_rule_matches_the_matrix():
    assert classify_outcome("definition", None, None) == ("b", None)
    assert classify_outcome("definition", [], None) == ("b", None)
    assert classify_outcome("references", "", None) == ("b", None)
    assert classify_outcome("hover", {}, None) == ("b", None)
    assert classify_outcome("hover", {"contents": ""}, None) == ("b", None)
    assert classify_outcome("hover", {"contents": {"kind": "markdown", "value": "x"}}, None) == ("a", None)
    assert classify_outcome("definition", [{"uri": "file:///r/a.rs"}], None) == ("a", None)
    assert classify_outcome("definition", None, {"code": -32601, "message": "unknown"}) == ("c", "-32601")
    assert classify_outcome("definition", None, "TIMEOUT") == ("c", "TIMEOUT")


def test_digest_ignores_list_order_only_for_the_unordered_methods():
    a = [{"label": "a"}, {"label": "b"}]
    assert digest("completion", a) == digest("completion", list(reversed(a)))
    assert digest("completion", {"isIncomplete": False, "items": a}) == digest("completion", {"isIncomplete": False, "items": list(reversed(a))})
    assert digest("completion", {"isIncomplete": True, "items": a}) != digest("completion", {"isIncomplete": False, "items": a})
    locs = [{"uri": "file:///r/a.rs", "range": 1}, {"uri": "file:///r/b.rs", "range": 2}]
    assert digest("definition", locs) != digest("definition", list(reversed(locs)))
    assert len(digest("definition", locs)) == 16
    calls = [{"from": {"name": "a"}, "fromRanges": [{"x": 1}, {"x": 2}]}, {"from": {"name": "b"}, "fromRanges": []}]
    swapped = [{"from": {"name": "b"}, "fromRanges": []}, {"from": {"name": "a"}, "fromRanges": [{"x": 2}, {"x": 1}]}]
    assert digest("incoming_calls", calls) == digest("incoming_calls", swapped)
    assert digest("outgoing_calls", calls) != digest("outgoing_calls", swapped)
    edit = {"documentChanges": [{"edits": [{"newText": "n", "range": 1}, {"newText": "n", "range": 2}]}, {"edits": [{"newText": "n", "range": 3}]}]}
    edit_swapped = {"documentChanges": [{"edits": [{"newText": "n", "range": 3}]}, {"edits": [{"newText": "n", "range": 2}, {"newText": "n", "range": 1}]}]}
    assert digest("rename", edit) == digest("rename", edit_swapped)
    assert digest("rename", edit) != digest("rename", {"documentChanges": [{"edits": [{"newText": "n", "range": 3}]}]})


ENTRIES = [("serde/src/lib.rs", "lib"), ("serde/src/bin/tool.rs", "bin"), ("test_suite/tests/test_de.rs", "test"),
           ("examples/demo/main.rs", "example"), ("benches/parse.rs", "bench"), ("serde/build.rs", "custom-build")]


def test_target_kind_follows_entry_directories_and_stems():
    assert target_kind_of("serde/src/lib.rs", ENTRIES) == "lib"
    assert target_kind_of("serde/src/de/value.rs", ENTRIES) == "lib"
    assert target_kind_of("serde/src/bin/tool.rs", ENTRIES) == "bin"
    assert target_kind_of("serde/src/bin/helper.rs", ENTRIES) == "bin"
    assert target_kind_of("test_suite/tests/test_de.rs", ENTRIES) == "test"
    assert target_kind_of("test_suite/tests/test_de/common.rs", ENTRIES) == "test"
    assert target_kind_of("test_suite/tests/ui/fail.rs", ENTRIES) == "test"
    assert target_kind_of("examples/demo/util.rs", ENTRIES) == "example"
    assert target_kind_of("benches/other.rs", ENTRIES) == "bench"
    assert target_kind_of("serde/build.rs", ENTRIES) == "custom-build"
    assert target_kind_of("other/src/lib.rs", ENTRIES) == "none"
    assert target_kind_of("x.rs", []) == "none"


SOURCE = """use std::fmt;
pub struct Foo;
impl fmt::Debug for Foo {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result { f.write_str("Foo") }
}
fn helper(v: Vec<u8>) -> usize { v.len() }
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip() { assert_eq!(helper(vec![1]), 1); }
}
#[cfg(test)]
fn only_in_tests() {}
fn after() {}
""".split("\n")


def test_cursor_kind_and_cfg_test_detection():
    assert cursor_kind(SOURCE, 2, 12, "Foo") == {"cursor_kind": "decl:struct", "cfg_test": False}
    assert cursor_kind(SOURCE, 3, 1, None)["cursor_kind"] == "impl"
    assert cursor_kind(SOURCE, 4, 8, "fmt")["cursor_kind"] == "decl:fn"
    assert cursor_kind(SOURCE, 6, 36, "len")["cursor_kind"] == "method_call"
    assert cursor_kind(SOURCE, 6, 14, "Vec")["cursor_kind"] == "type_usage"
    assert cursor_kind(SOURCE, 1, 10, "fmt")["cursor_kind"] == "use"
    assert cursor_kind(SOURCE, 11, 8, "roundtrip") == {"cursor_kind": "decl:fn", "cfg_test": True}
    assert cursor_kind(SOURCE, 11, 22, "assert_eq")["cursor_kind"] == "macro"
    assert cursor_kind(SOURCE, 14, 4, "only_in_tests")["cfg_test"] is True
    assert cursor_kind(SOURCE, 15, 4, "after")["cfg_test"] is False
    assert in_cfg_test(SOURCE, 5) is False and in_cfg_test(SOURCE, 8) is True and in_cfg_test(SOURCE, 11) is False
    assert cursor_kind(SOURCE, 99, 1, None)["cursor_kind"] == "out_of_file"


def _rows():
    positions = {"definition": [
        {"name": "definition_0_Foo", "file": "src/lib.rs", "line": 2, "character": 12, "symbol": "Foo"},
        {"name": "definition_1_len", "file": "src/lib.rs", "line": 6, "character": 36, "symbol": "len"},
        {"name": "definition_2_x", "file": "src/lib.rs", "line": 11, "character": 8, "symbol": "roundtrip"},
    ], "hover": [{"name": "hover_0_Foo", "file": "src/lib.rs", "line": 2, "character": 12, "symbol": "Foo"}]}
    rows = build_rows(positions, ["definition", "hover"], None, REPO, [("src/lib.rs", "lib")], lambda rel: SOURCE)
    results = {
        "definition_0_Foo": ({"outcome": "a", "digest": "d1", "us": 10.0, "error": None}, {"outcome": "a", "digest": "d1", "us": 12.0, "error": None}),
        "definition_1_len": ({"outcome": "a", "digest": "d2", "us": 20.0, "error": None}, {"outcome": "b", "digest": "empty", "us": 8.0, "error": None}),
        "definition_2_x": ({"outcome": "b", "digest": "empty", "us": 5.0, "error": None}, {"outcome": "a", "digest": "d9", "us": 9.0, "error": None}),
        "hover_0_Foo": ({"outcome": "c", "digest": None, "us": 0.0, "error": "-32601"}, {"outcome": "c", "digest": None, "us": 0.0, "error": "-32601"}),
    }
    for row in rows:
        row["results"]["old"], row["results"]["new"] = results[row["name"]]
    return rows


def test_rows_carry_target_and_cursor_kinds_in_matrix_order():
    rows = _rows()
    assert [r["method"] for r in rows] == ["definition"] * 3 + ["hover"]
    assert rows[0]["target_kind"] == "lib" and rows[0]["cursor_kind"] == "decl:struct"
    assert rows[2]["cfg_test"] is True and rows[0]["rpc_method"] == "textDocument/definition"


def test_summary_comparison_and_matrix_check_on_fixed_rows():
    rows = _rows()
    old = summarize_server(rows, "old")
    assert (old["definition"]["a"], old["definition"]["b"], old["definition"]["c"]) == (2, 1, 0)
    assert old["hover"]["unsupported"] is True and old["hover"]["p50_us"] is None
    assert old["definition"]["p50_us"] == 10.0
    pairs = compare_servers(rows, ["old", "new"])
    assert len(pairs) == 1 and pairs[0]["same"] == 2 and pairs[0]["differing"] == 2
    d = pairs[0]["methods"]["definition"]
    assert d["transitions"] == {"a>b": 1, "b>a": 1} and [e["name"] for e in d["examples"]] == ["definition_1_len", "definition_2_x"]
    assert d["examples"][1]["cfg_test"] is True
    check = matrix_check(old, {"definition": {"tests_run": 3, "a_non_empty": 2, "b_empty": 1, "c_error": 0},
                              "hover": {"tests_run": 300, "a_non_empty": 250, "b_empty": 50, "c_error": 0}})
    assert check == {"compared": 1, "matching": 1, "mismatches": []}
    bad = matrix_check(old, {"definition": {"tests_run": 3, "a_non_empty": 3, "b_empty": 0, "c_error": 0}})
    assert bad["mismatches"] == [{"method": "definition", "ours": [2, 1, 0], "matrix": [3, 0, 0]}]


def test_differing_only_keeps_summaries_and_comparison_and_filters_rows():
    rows = _rows()
    names = ["old", "new"]
    report = {"repo": "r", "servers": [{"name": n, "summary": summarize_server(rows, n)} for n in names],
              "comparison": compare_servers(rows, names), "rows": rows, "metric_definitions": {"outcome": "x"}}
    filtered = differing_only(report, "full_serde.json", "ab" * 32)
    assert filtered["servers"] == report["servers"] and filtered["comparison"] == report["comparison"]
    assert [r["name"] for r in filtered["rows"]] == ["definition_1_len", "definition_2_x"]
    assert (filtered["rows_total"], filtered["rows_kept"], filtered["rows_filter"]) == (4, 2, "differing")
    assert filtered["full_file"] == {"name": "full_serde.json", "sha256": "ab" * 32}
    assert filtered["metric_definitions"]["outcome"] == "x" and "rows_filter" in filtered["metric_definitions"]
    assert [row_differs(r) for r in rows] == [False, True, True, False]
    assert len(report["rows"]) == 4


def test_binary_label_keeps_only_the_file_name():
    assert binary_label("/tmp/ridx_sec_8cb4341") == "ridx_sec_8cb4341"
    assert binary_label("/opt/ridx-bench/target-filter/release/ridx") == "ridx"
    assert binary_label("ridx") == "ridx"
