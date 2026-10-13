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

from matrix_canary import ensure_ra_ready, is_non_empty_response, wait_ra_canary_ready


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


class MockQuiescentServer(MockLspServer):
    """Mock server that also records wait_quiescent calls like SimpleLspProcess."""

    def __init__(self, response_sequence, quiescent_ms=42.0):
        super().__init__(response_sequence)
        self.quiescent_calls = []
        self.quiescent_ms = quiescent_ms

    def wait_quiescent(self, timeout=600.0, settle_idle=0.5):
        self.quiescent_calls.append((timeout, settle_idle))
        return self.quiescent_ms


def test_ensure_ra_ready_waits_quiescent_then_canary():
    """ensure_ra_ready calls wait_quiescent once with the given settle window, then the canary."""
    loc = [{"uri": "file:///opt/vendor/x.rs", "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}}}]
    srv = MockQuiescentServer([[], loc, loc], quiescent_ms=1234.5)
    canary_positions = [{"file": "a.rs", "location": ["a.rs", 1, 1]}]
    ready = ensure_ra_ready(srv, canary_positions, Path("/tmp"), timeout=5.0, interval=0.01, settle_idle=2.0)
    assert srv.quiescent_calls == [(5.0, 2.0)], f"Expected one wait_quiescent(5.0, 2.0) call, got {srv.quiescent_calls}"
    assert ready["quiescent_wait_ms"] == 1234.5, f"Expected quiescent_wait_ms 1234.5, got {ready['quiescent_wait_ms']}"
    assert ready["canary_attempts"] == 3, f"Expected 3 canary attempts, got {ready['canary_attempts']}"
    assert len(srv.requests) == 3, f"Expected 3 requests, got {len(srv.requests)}"
    print(f"PASS: ensure_ra_ready waited quiescent ({ready['quiescent_wait_ms']} ms) and canary succeeded on attempt {ready['canary_attempts']}")


def test_ensure_ra_ready_without_wait_quiescent():
    """A server without wait_quiescent still passes through the canary with quiescent_wait_ms 0."""
    loc = [{"uri": "file:///opt/vendor/x.rs", "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}}}]
    srv = MockLspServer([loc, loc])
    ready = ensure_ra_ready(srv, [{"file": "a.rs", "location": ["a.rs", 1, 1]}], Path("/tmp"), timeout=5.0, interval=0.01)
    assert ready["quiescent_wait_ms"] == 0.0, f"Expected 0.0, got {ready['quiescent_wait_ms']}"
    assert ready["canary_attempts"] == 2, f"Expected 2 attempts, got {ready['canary_attempts']}"
    print("PASS: ensure_ra_ready without wait_quiescent ran the canary only")


if __name__ == "__main__":
    test_canary_empty_empty_nonempty_nonempty()
    test_canary_timeout_on_all_empty()
    test_canary_reset_on_intermittent_empty()
    test_canary_all_positions_must_be_nonempty()
    test_ensure_ra_ready_waits_quiescent_then_canary()
    test_ensure_ra_ready_without_wait_quiescent()
    print("PASS: all rust-analyzer canary unit tests passed successfully")
