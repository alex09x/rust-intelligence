# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""
Unit tests for matrix_categories: the request parameters built for sampled items follow the
LSP parameter shapes the servers read.
"""

import sys
from pathlib import Path

bench_dir = Path(__file__).resolve().parent
if str(bench_dir) not in sys.path:
    sys.path.insert(0, str(bench_dir))

from matrix_categories import build_params

REPO_DIR = "/repo"
FILE_SCENARIO = {"file": "/repo/src/lib.rs"}


def test_text_document_content_params_carry_uri_at_top_level():
    # TextDocumentContentParams is `{"uri": DocumentUri}`; ridx reads `params.uri` and answers an
    # empty text for any other shape.
    for is_ra in (False, True):
        params = build_params("text_document_content", FILE_SCENARIO, REPO_DIR, is_ra=is_ra)
        assert params == {"uri": "file:///repo/src/lib.rs"}, params
    print("PASS: textDocument/content params are {uri}")


def test_whole_document_requests_keep_text_document_identifier():
    for key in ("document_symbol", "folding_range", "code_lens", "document_diagnostic",
                "document_link", "document_color"):
        params = build_params(key, FILE_SCENARIO, REPO_DIR)
        assert params == {"textDocument": {"uri": "file:///repo/src/lib.rs"}}, (key, params)
    print("PASS: whole-document requests send a TextDocumentIdentifier")


if __name__ == "__main__":
    test_text_document_content_params_carry_uri_at_top_level()
    test_whole_document_requests_keep_text_document_identifier()
    print("PASS: all matrix_categories tests passed")
