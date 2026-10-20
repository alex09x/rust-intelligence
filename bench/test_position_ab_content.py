# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""
Unit tests for position_ab_content on fixed rows: token decoding, where a token lands in the live
file, the token class with displaced and foreign tokens, the call hierarchy class, the root lib
class, the noise flag and the scrubbing of private path prefixes.
"""

import sys
from pathlib import Path

bench_dir = Path(__file__).resolve().parent
if str(bench_dir) not in sys.path:
    sys.path.insert(0, str(bench_dir))

from position_ab_content import candidate_dirs, dependency_roots, differing_rows, load_file_table
from position_ab_content_rules import (
    CLASS_CALLS, CLASS_ROOT, char_span, classify_calls, classify_rename, classify_root_lib, classify_tokens,
    decode_tokens, resolve_parent_dirs, scrub_path, span_kind, stored_forms_of, utf16_to_char,
)

REPO = "/opt/ridx-bench/repos/serde"
LINES = [
    "//! Serde provides implementations",  # 0: comment line
    "use crate::internals::ast::{Container, Data};",  # 1
    "fn build(cont: &Container) -> Data {",  # 2
    "    let generics = cont.generics;",  # 3
    "    where",  # 4
    "}",  # 5
]


def test_decode_tokens_follows_line_and_start_deltas():
    data = [1, 4, 5, 2, 0, 0, 7, 3, 1, 0, 2, 1, 4, 0, 0]
    assert decode_tokens(data) == [(1, 4, 5, 2), (1, 11, 3, 1), (3, 1, 4, 0)]
    assert decode_tokens(data + [9, 9]) == decode_tokens(data)


def test_span_kind_on_fixed_lines():
    assert span_kind(LINES, (9, 0, 1, 0)) == "past_eof"
    assert span_kind(LINES, (5, 0, 4, 0)) == "past_eol"
    assert span_kind(LINES, (0, 4, 5, 0)) == "comment_or_keyword"  # `Serde` on a `//!` line
    assert span_kind(LINES, (3, 4, 3, 0)) == "comment_or_keyword"  # `let`
    assert span_kind(LINES, (4, 4, 5, 0)) == "comment_or_keyword"  # `where`
    assert span_kind(LINES, (1, 11, 4, 1)) == "not_identifier"  # `inte` inside `internals`
    assert span_kind(LINES, (1, 4, 24, 1)) == "not_identifier"  # `crate::internals::ast::` runs into `{`
    assert span_kind(LINES, (1, 28, 9, 1)) == "identifier"  # `Container` in the use list
    assert span_kind(LINES, (2, 16, 9, 1)) == "identifier"  # `Container` in the signature


def test_span_kind_counts_utf16_columns():
    lines = ["let s = \"\U0001F980\"; let crab = 1;"]
    # the emoji is two UTF-16 units: `crab` starts at unit 18, not at character 17
    assert span_kind(lines, (0, 18, 4, 0)) == "identifier"
    assert span_kind(lines, (0, 17, 4, 0)) == "not_identifier"


def test_classify_tokens_displaced_and_foreign_tokens_pass():
    foreign = {(2, 16, 9, 1): "registry:serde_test-1.0.177/src/de.rs"}
    before = [(1, 11, 4, 1), (2, 16, 9, 1), (0, 4, 5, 0), (3, 4, 3, 0), (9, 0, 1, 0), (5, 0, 4, 0), (2, 3, 5, 3)]
    after = [(1, 4, 21, 1), (2, 3, 5, 3)]
    ok, ev = classify_tokens(LINES, before, after, foreign.get)
    assert ok
    assert ev["before_tokens"] == 7 and ev["after_tokens"] == 2 and ev["common"] == 1
    assert ev["removed"] == {
        "past_eof": 1, "past_eol": 1, "not_identifier": 1, "comment_or_keyword": 2,
        "identifier_starts_same_tail_token": 1, "unexplained": 0,
    }
    assert ev["removed_identifier_files"] == ["registry:serde_test-1.0.177/src/de.rs"]
    assert ev["added"] == {"displaced_by_removed_token": 1, "unexplained": 0}


def test_classify_tokens_flags_unexplained_removed_and_added():
    ok, ev = classify_tokens(LINES, [(2, 16, 9, 1)], [(2, 30, 4, 1)], lambda tok: None)
    assert not ok
    assert ev["removed"]["unexplained"] == 1 and ev["unexplained_removed"] == [[3, 16, 9, 1]]
    assert ev["added"] == {"displaced_by_removed_token": 0, "unexplained": 1}


def test_classify_calls_accepts_foreign_removals_only():
    entries = [("serde/src/lib.rs", "lib")]
    removed = [f"file://{REPO}/examples/demo.rs", "file:///opt/cargo/registry/src/index.crates.io-abc/syn-2.0.1/src/lib.rs"]
    ok, cls, ev = classify_calls(removed, [], REPO, entries)
    assert ok and cls == CLASS_CALLS
    # items are sorted by uri: the registry path sorts before the workspace path
    assert [(r["path"], r["where"]) for r in ev["removed_items"]] == [
        ("registry:syn-2.0.1/src/lib.rs", "outside_workspace"), ("ws:examples/demo.rs", "no_indexed_target"),
    ]
    ok, _, ev = classify_calls([f"file://{REPO}/serde/src/de.rs"], [], REPO, entries)
    assert not ok and ev["removed_items"][0]["where"] == "workspace"
    ok, _, _ = classify_calls([], [f"file://{REPO}/examples/demo.rs"], REPO, entries)
    assert not ok


def test_classify_root_lib_needs_crates_lib_and_root_location():
    before = [f"file://{REPO}/src/lib.rs", f"file://{REPO}/crates/bevy_app/src/lib.rs"]
    ok, ev = classify_root_lib("crates/bevy_app/src/lib.rs", before, [f"file://{REPO}/crates/bevy_app/src/lib.rs"], REPO)
    assert ok and ev["stored_path_hit_by_suffix_match"] == "src/lib.rs"
    assert ev["before_locations"] == ["ws:crates/bevy_app/src/lib.rs", "ws:src/lib.rs"]
    assert not classify_root_lib("crates/bevy_app/src/app.rs", before, [], REPO)[0]
    assert not classify_root_lib("crates/bevy_app/src/lib.rs", [f"file://{REPO}/crates/bevy_app/src/lib.rs"], [], REPO)[0]


def test_classify_rename_needs_another_symbol_before_and_the_cursor_symbol_after():
    err = {"code": -32803, "message": "Cannot rename external symbol 'into_iter'"}
    ok, ev = classify_rename("spawn10", err, {"placeholder": "spawn10", "range": {}})
    assert ok and ev["before_symbol_named"] == "into_iter" and ev["after_placeholder"] == "spawn10"
    ok, ev = classify_rename("spawn10", err, {"changes": {"file:///r/benches/spawn.rs": [{"newText": "x", "range": {}}]}})
    assert ok and ev["after_edits"] == 1
    assert not classify_rename("into_iter", err, {"placeholder": "into_iter"})[0]
    assert not classify_rename("spawn10", None, {"placeholder": "spawn10"})[0]
    assert not classify_rename("spawn10", err, None)[0]


def test_char_span_converts_utf16_columns_with_the_live_line():
    lines = ["let s = \"\U0001F600\"; let crab = 1;"]
    assert utf16_to_char(lines[0], 18) == 17
    assert char_span(lines, (0, 18, 4, 2)) == (0, 17, 4, 2)
    assert char_span(lines, (0, 3, 1, 2)) == (0, 3, 1, 2)
    assert char_span(lines, (3, 0, 1, 0)) is None


def test_stored_forms_of_separates_plain_parent_dir_and_same_tail():
    table = [
        ("stdx-1.ridx", "crates/stdx/src/lib.rs"),
        ("gizmos-2.ridx", "examples/2d/../helpers/checkbox.rs"),
        ("impl-3.ridx", "src/lib.rs"),
        ("syn-4.ridx", "lib.rs"),
    ]
    forms = stored_forms_of("crates/stdx/src/lib.rs", f"{REPO}/crates/stdx/src/lib.rs", table)
    assert forms["plain"] == [("stdx-1.ridx", "crates/stdx/src/lib.rs")]
    assert forms["parent_dir"] == []
    assert forms["same_tail"] == [("impl-3.ridx", "src/lib.rs"), ("syn-4.ridx", "lib.rs")]
    forms = stored_forms_of("examples/helpers/checkbox.rs", f"{REPO}/examples/helpers/checkbox.rs", table)
    assert forms["plain"] == [] and forms["parent_dir"] == [("gizmos-2.ridx", "examples/2d/../helpers/checkbox.rs")]
    forms = stored_forms_of("crates/imp/src/lib.rs", f"{REPO}/crates/imp/src/lib.rs", table)
    assert forms["plain"] == [] and forms["parent_dir"] == [] and len(forms["same_tail"]) == 2
    assert resolve_parent_dirs("examples/ui/styling/../../helpers/x.rs") == "examples/helpers/x.rs"


def test_load_file_table_keeps_file_lines_only(tmp_path):
    f = tmp_path / "t.tsv"
    f.write_text("FILE\ta.ridx\tsrc/lib.rs\nCHECK\ta.ridx\tsrc/../x.rs\tx.rs\tfound\nFILE\tb.ridx\tcrates/b/src/lib.rs\n")
    assert load_file_table([str(f)]) == [("a.ridx", "src/lib.rs"), ("b.ridx", "crates/b/src/lib.rs")]


def test_candidate_dirs_include_workspace_root_and_target_packages(tmp_path):
    (tmp_path / "crates" / "x" / "src").mkdir(parents=True)
    dirs = candidate_dirs(str(tmp_path), [("crates/x/src/lib.rs", "lib"), ("examples/demo.rs", "example")])
    assert str(tmp_path) in dirs and str(tmp_path / "crates" / "x") in dirs and str(tmp_path / "examples") in dirs


def test_dependency_roots_come_from_the_index_manifest(tmp_path):
    (tmp_path / "target" / "ridx").mkdir(parents=True)
    (tmp_path / "target" / "ridx" / "ridx_manifest.json").write_text(
        '{"dependency_shards": [{"source_root": "/opt/cargo/registry/src/i/zerocopy-0.8.62"}, {"source_root": ""}, {}]}'
    )
    assert dependency_roots(str(tmp_path)) == ["/opt/cargo/registry/src/i/zerocopy-0.8.62"]
    assert dependency_roots(str(tmp_path / "nowhere")) == []


def test_scrub_path_keeps_no_private_prefix():
    assert scrub_path(f"{REPO}/serde/src/lib.rs", REPO) == "ws:serde/src/lib.rs"
    assert scrub_path("/opt/cargo/registry/src/index.crates.io-abc/serde_json-1.0.140/src/de.rs", REPO) == "registry:serde_json-1.0.140/src/de.rs"
    assert scrub_path("/opt/cargo/git/checkouts/wgpu-0123abcd/9f8e7d/wgpu/src/lib.rs", REPO) == "git:wgpu/src/lib.rs"
    assert scrub_path("/opt/toolchains/x/lib/rustlib/src/rust/library/core/src/lib.rs", REPO) == "sysroot:library/core/src/lib.rs"
    assert scrub_path("/opt/elsewhere/pkg/src/lib.rs", REPO) == "external:src/lib.rs"


def test_differing_rows_flags_noise_between_the_two_before_servers():
    def row(b, a, b2):
        return {"results": {"before": {"outcome": "a", "digest": b}, "after": {"outcome": "a", "digest": a}, "before2": {"outcome": "a", "digest": b2}}}

    rep = {"rows": [row("x", "x", "x"), row("x", "y", "x"), row("x", "y", "z")]}
    out = list(differing_rows(rep, ["before", "after", "before2"]))
    assert [noise for _, noise in out] == [False, True]
    assert out[0][0] is rep["rows"][1]
