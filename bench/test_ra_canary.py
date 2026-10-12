# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""
Unit tests verifying rust-analyzer canary readiness logic on mock LSP server.
"""

import sys
import time
from pathlib import Path

bench_dir = Path(__file__).resolve().parent
if str(bench_dir) not in sys.path:
    sys.path.insert(0, str(bench_dir))

from matrix_canary import is_non_empty_response, wait_ra_canary_ready


class MockLspServer:
    """Mock LSP server yielding a programmed sequence of responses to requests."""

    def __init__(self, response_sequence):
        self.response_sequence = list(response_sequence)
        self.requests = []

    def request(self, method, params, timeout=5.0):
        self.requests.append({"method": method, "params": params})
        if self.response_sequence:
            resp = self.response_sequence.pop(0)
            return resp, 1.0, None
        return [], 1.0, None


def test_canary_empty_empty_nonempty_nonempty():
    """Sequence: empty -> empty -> non-empty -> non-empty succeeds on 4th request."""
    loc = [{"uri": "file:///test.rs", "range": {"start": {"line": 1, "character": 1}, "end": {"line": 1, "character": 5}}}]
    responses = [
        [],     # Attempt 1: empty
        None,   # Attempt 2: empty (null)
        loc,    # Attempt 3: non-empty (consecutive = 1)
        loc,    # Attempt 4: non-empty (consecutive = 2 -> READY)
    ]
    srv = MockLspServer(responses)
    canary_positions = [{"file": "test.rs", "location": ["test.rs", 1, 1]}]

    wait_ms, attempts = wait_ra_canary_ready(
        srv, canary_positions, Path("/tmp"), timeout=5.0, interval=0.01, required_consecutive=2
    )
    assert attempts == 4, f"Expected 4 attempts, got {attempts}"
    assert len(srv.requests) == 4, f"Expected 4 requests sent, got {len(srv.requests)}"
    assert wait_ms >= 0.0, f"Expected non-negative wait_ms, got {wait_ms}"
    print(f"PASS: empty -> empty -> non-empty -> non-empty succeeded on attempt {attempts} ({len(srv.requests)} requests)")


def test_canary_timeout_on_all_empty():
    """Sequence of only empty responses raises RuntimeError on timeout."""
    srv = MockLspServer([[] for _ in range(50)])
    canary_positions = [{"file": "test.rs", "location": ["test.rs", 1, 1]}]

    timed_out = False
    try:
        wait_ra_canary_ready(
            srv, canary_positions, Path("/tmp"), timeout=0.05, interval=0.01, required_consecutive=2
        )
    except RuntimeError as e:
        timed_out = True
        assert "canary readiness timed out" in str(e)
        print(f"PASS: timeout raised RuntimeError as expected ({e})")

    assert timed_out, "Expected RuntimeError on timeout"


def test_canary_reset_on_intermittent_empty():
    """Sequence: non-empty -> empty -> non-empty -> non-empty succeeds on attempt 4."""
    loc = [{"uri": "file:///test.rs", "range": {"start": {"line": 1, "character": 1}, "end": {"line": 1, "character": 5}}}]
    responses = [
        loc,    # Attempt 1: non-empty (consecutive = 1)
        [],     # Attempt 2: empty (reset to 0)
        loc,    # Attempt 3: non-empty (consecutive = 1)
        loc,    # Attempt 4: non-empty (consecutive = 2 -> READY)
    ]
    srv = MockLspServer(responses)
    canary_positions = [{"file": "test.rs", "location": ["test.rs", 1, 1]}]

    wait_ms, attempts = wait_ra_canary_ready(
        srv, canary_positions, Path("/tmp"), timeout=5.0, interval=0.01, required_consecutive=2
    )
    assert attempts == 4, f"Expected 4 attempts, got {attempts}"
    assert len(srv.requests) == 4, f"Expected 4 requests sent, got {len(srv.requests)}"
    print(f"PASS: intermittent failure correctly reset consecutive counter and succeeded on attempt {attempts}")


def test_canary_all_positions_must_be_nonempty():
    """For 3 canary positions, all 3 must yield non-empty responses in the same attempt."""
    loc = [{"uri": "file:///test.rs", "range": {"start": {"line": 1, "character": 1}, "end": {"line": 1, "character": 5}}}]
    # Attempt 1: pos 1 returns empty (fails attempt immediately)
    # Attempt 2: pos 1 ok, pos 2 returns empty (fails attempt)
    # Attempt 3: pos 1 ok, pos 2 ok, pos 3 ok (consecutive = 1)
    # Attempt 4: pos 1 ok, pos 2 ok, pos 3 ok (consecutive = 2 -> READY)
    responses = [
        [],                 # Attempt 1: pos 1 fails
        loc, [],            # Attempt 2: pos 1 passes, pos 2 fails
        loc, loc, loc,      # Attempt 3: all 3 pass
        loc, loc, loc,      # Attempt 4: all 3 pass
    ]
    srv = MockLspServer(responses)
    canary_positions = [
        {"file": "a.rs", "location": ["a.rs", 1, 1]},
        {"file": "b.rs", "location": ["b.rs", 2, 2]},
        {"file": "c.rs", "location": ["c.rs", 3, 3]},
    ]

    wait_ms, attempts = wait_ra_canary_ready(
        srv, canary_positions, Path("/tmp"), timeout=5.0, interval=0.01, required_consecutive=2
    )
    assert attempts == 4, f"Expected 4 attempts, got {attempts}"
    assert len(srv.requests) == 9, f"Expected 9 total queries, got {len(srv.requests)}"
    print(f"PASS: 3-position canary verified all positions across {attempts} attempts ({len(srv.requests)} queries)")


if __name__ == "__main__":
    test_canary_empty_empty_nonempty_nonempty()
    test_canary_timeout_on_all_empty()
    test_canary_reset_on_intermittent_empty()
    test_canary_all_positions_must_be_nonempty()
    print("PASS: all rust-analyzer canary unit tests passed successfully")
