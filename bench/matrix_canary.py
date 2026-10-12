# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""
Rust-analyzer canary readiness verification.
Probes definition requests against sampled positions until consecutive non-empty
responses are confirmed, ensuring background crate indexing has settled.
"""

import time
from pathlib import Path
from typing import Any, Dict, List, Tuple

from matrix_categories import build_params


def is_non_empty_response(res: Any, err: Any) -> bool:
    """Returns True if the response represents a valid, non-empty LSP result."""
    if err is not None:
        return False
    if res is None:
        return False
    if isinstance(res, (list, str, dict)) and len(res) == 0:
        return False
    return True


def wait_ra_canary_ready(
    srv: Any,
    canary_positions: List[Dict[str, Any]],
    repo_dir: Path,
    timeout: float = 600.0,
    interval: float = 1.0,
    required_consecutive: int = 2,
) -> Tuple[float, int]:
    """
    Awaits rust-analyzer full readiness by probing textDocument/definition.
    Requires required_consecutive consecutive non-empty passes across all canary positions,
    with interval seconds between attempts.

    Returns (wait_ms, attempts).
    Raises RuntimeError if timeout is exceeded before readiness is established.
    """
    if not canary_positions:
        return 0.0, 0

    t0 = time.perf_counter()
    consecutive_success = 0
    attempts = 0

    while True:
        attempts += 1
        elapsed = time.perf_counter() - t0
        if elapsed > timeout:
            raise RuntimeError(
                f"rust-analyzer canary readiness timed out after {elapsed:.1f}s "
                f"({attempts} attempts, required {required_consecutive} consecutive successes)"
            )

        all_non_empty = True
        for sc in canary_positions:
            params = build_params("definition", sc, repo_dir, is_ra=True)
            res, dur, err = srv.request("textDocument/definition", params, timeout=5.0)
            if not is_non_empty_response(res, err):
                all_non_empty = False
                break

        if all_non_empty:
            consecutive_success += 1
            if consecutive_success >= required_consecutive:
                wait_ms = (time.perf_counter() - t0) * 1000.0
                return round(wait_ms, 2), attempts
        else:
            consecutive_success = 0

        time.sleep(interval)
