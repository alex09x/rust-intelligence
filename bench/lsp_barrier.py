# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""
Barrier between benchmark methods.

SimpleLspProcess.drain() waits for answers to requests only. Notifications such as didOpen and
didClose have no answer, so a backlog of them is invisible to drain() and delays the first
requests of the next method. A server reads its input in order, so once it answers a request
sent after the notifications it has also processed every notification before it. The barrier
sends a method no server implements; both servers answer it with -32601 (MethodNotFound).
"""

import time

BARRIER_METHOD = "bench/barrier"


def barrier(srv, timeout=600.0):
    """Waits until the server has processed every message sent before this call.

    Returns (waited_ms, answered); answered is False when the server did not reply in time or exited.
    """
    t0 = time.perf_counter()
    _res, _dur, err = srv.request(BARRIER_METHOD, None, timeout=timeout)
    answered = err not in ("TIMEOUT", "PROCESS_EXITED", "BROKEN_PIPE")
    return round((time.perf_counter() - t0) * 1000.0, 2), answered
