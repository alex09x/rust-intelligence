# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""
Regression test for timed-out requests in SimpleLspProcess.

A server slower than the client timeout used to receive every timed-out request and compute all
of them; the backlog then delayed and timed out the first requests of the next benchmark method
(observed on bevy: about 20 s per typeHierarchy/subtypes request against a 2 s client timeout).
The client now cancels a request when it stops waiting, and drain() waits for every outstanding
answer before the next method starts. The fake server mirrors ridx: a reader thread records
$/cancelRequest ids, and the worker rejects cancelled requests with -32800 without computing them.
"""

import sys
import tempfile
import time
from pathlib import Path

bench_dir = Path(__file__).resolve().parent
if str(bench_dir) not in sys.path:
    sys.path.insert(0, str(bench_dir))

from lsp_process import SimpleLspProcess

FAKE_SERVER = r"""
import json, queue, sys, threading, time
inp = sys.stdin.buffer
out = sys.stdout.buffer
cancelled = set()
work = queue.Queue()
computed = [0]
def read_msg():
    header = b""
    while not header.endswith(b"\r\n\r\n"):
        ch = inp.read(1)
        if not ch:
            return None
        header += ch
    length = int([l for l in header.decode().split("\r\n") if l.lower().startswith("content-length:")][0].split(":")[1])
    return json.loads(inp.read(length))
def write_msg(m):
    b = json.dumps(m).encode()
    out.write(b"Content-Length: %d\r\n\r\n" % len(b) + b)
    out.flush()
def reader():
    while True:
        m = read_msg()
        if m is None:
            work.put(None)
            return
        if m.get("method") == "$/cancelRequest":
            cancelled.add(m["params"]["id"])
        work.put(m)
threading.Thread(target=reader, daemon=True).start()
while True:
    m = work.get()
    if m is None or m.get("method") == "exit":
        break
    if "id" not in m:
        continue
    if m["id"] in cancelled:
        write_msg({"jsonrpc": "2.0", "id": m["id"], "error": {"code": -32800, "message": "Request cancelled"}})
        continue
    if m.get("method") == "initialize":
        write_msg({"jsonrpc": "2.0", "id": m["id"], "result": {"capabilities": {}}})
    elif m.get("method") == "slow/query":
        time.sleep(m["params"].get("sleep", 0.05))
        computed[0] += 1
        write_msg({"jsonrpc": "2.0", "id": m["id"], "result": [m["params"]["n"]]})
    elif m.get("method") == "stats/computed":
        write_msg({"jsonrpc": "2.0", "id": m["id"], "result": computed[0]})
"""


def _start(td: str) -> SimpleLspProcess:
    server = Path(td) / "fake_server.py"
    server.write_text(FAKE_SERVER, encoding="utf-8")
    return SimpleLspProcess([sys.executable, "-u", str(server)], td, is_ra=False)


def test_timed_out_requests_are_cancelled_and_drained():
    with tempfile.TemporaryDirectory() as td:
        srv = _start(td)
        try:
            # 500 ms per request keeps a 5 ms client timeout far below the server time even on a
            # loaded host, where the wait itself can overshoot by tens of milliseconds.
            t0 = time.perf_counter()
            errs = [srv.request("slow/query", {"n": n, "sleep": 0.5}, timeout=0.005)[2] for n in range(100)]
            send_s = time.perf_counter() - t0
            assert errs.count("TIMEOUT") == 100, errs
            assert srv.cancelled_on_timeout == 100
            waited_ms, left = srv.drain(timeout=30.0)
            assert left == 0, (waited_ms, left)
            computed, _dur, err = srv.request("stats/computed", None, timeout=10.0)
            assert err is None
            # Only a request already being computed when its cancel arrived is computed: at most one
            # per 500 ms of sending, plus the one in progress; without cancellation all 100 would be.
            assert computed <= int(send_s / 0.5) + 2, (computed, send_s)
            res, _dur, err = srv.request("slow/query", {"n": 100}, timeout=10.0)
            assert (res, err) == ([100], None)
            print(f"PASS: 100 timed-out requests cancelled, {computed} computed, drained in {waited_ms} ms")
        finally:
            srv.p.kill()


def test_without_cancel_every_request_is_computed():
    with tempfile.TemporaryDirectory() as td:
        srv = _start(td)
        srv.cancel_on_timeout = False
        try:
            for n in range(40):
                srv.request("slow/query", {"n": n}, timeout=0.005)
            assert srv.cancelled_on_timeout == 0
            waited_ms, left = srv.drain(timeout=30.0)
            assert left == 0
            computed, _dur, err = srv.request("stats/computed", None, timeout=10.0)
            assert (computed, err) == (40, None)
            print(f"PASS: without cancellation all 40 requests are computed; drain waited {waited_ms} ms")
        finally:
            srv.p.kill()


def test_drain_returns_immediately_when_nothing_is_outstanding():
    with tempfile.TemporaryDirectory() as td:
        srv = _start(td)
        try:
            res, _dur, err = srv.request("slow/query", {"n": 1}, timeout=10.0)
            assert (res, err) == ([1], None)
            waited_ms, left = srv.drain(timeout=30.0)
            assert left == 0
            assert waited_ms < 50.0, waited_ms
            print("PASS: drain with no outstanding requests returns at once")
        finally:
            srv.p.kill()


if __name__ == "__main__":
    test_timed_out_requests_are_cancelled_and_drained()
    test_without_cancel_every_request_is_computed()
    test_drain_returns_immediately_when_nothing_is_outstanding()
    print("PASS: all lsp cancel/drain tests passed")
