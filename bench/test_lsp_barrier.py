# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""
Regression test for the barrier between benchmark methods.

drain() only waits for answers to requests. The document synchronization methods send
notifications only (didOpen, didSave, didClose), so their backlog stayed queued in the server
and timed out the first textDocument/definition requests of the next method (observed on bevy:
21 of 300). The fake server processes messages in order, spends 50 ms per slow notification and
answers unknown requests with -32601, as ridx and rust-analyzer do.
"""

import sys
import tempfile
from pathlib import Path

bench_dir = Path(__file__).resolve().parent
if str(bench_dir) not in sys.path:
    sys.path.insert(0, str(bench_dir))

from lsp_barrier import BARRIER_METHOD, barrier
from lsp_process import SimpleLspProcess

FAKE_SERVER = r"""
import json, sys, time
inp = sys.stdin.buffer
out = sys.stdout.buffer
processed = [0]
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
while True:
    m = read_msg()
    if m is None or m.get("method") == "exit":
        break
    if "id" not in m:
        if m.get("method") == "slow/notify":
            time.sleep(0.05)
            processed[0] += 1
        continue
    if m.get("method") == "initialize":
        write_msg({"jsonrpc": "2.0", "id": m["id"], "result": {"capabilities": {}}})
    elif m.get("method") == "stats/processed":
        write_msg({"jsonrpc": "2.0", "id": m["id"], "result": processed[0]})
    else:
        write_msg({"jsonrpc": "2.0", "id": m["id"], "error": {"code": -32601, "message": "Method not found"}})
"""

NOTIFICATIONS = 40


def _start(td: str) -> SimpleLspProcess:
    server = Path(td) / "fake_server.py"
    server.write_text(FAKE_SERVER, encoding="utf-8")
    return SimpleLspProcess([sys.executable, "-u", str(server)], td, is_ra=False)


def test_drain_alone_misses_queued_notifications():
    with tempfile.TemporaryDirectory() as td:
        srv = _start(td)
        try:
            for n in range(NOTIFICATIONS):
                srv.notify("slow/notify", {"n": n})
            waited_ms, left = srv.drain(timeout=30.0)
            assert left == 0
            assert waited_ms < 50.0, waited_ms
            # The backlog (40 x 50 ms) is still queued: a short request behind it times out.
            _res, _dur, err = srv.request("stats/processed", None, timeout=0.5)
            assert err == "TIMEOUT", err
            print(f"PASS: drain returned in {waited_ms} ms with the notification backlog still queued")
        finally:
            srv.p.kill()


def test_barrier_waits_for_queued_notifications():
    with tempfile.TemporaryDirectory() as td:
        srv = _start(td)
        try:
            for n in range(NOTIFICATIONS):
                srv.notify("slow/notify", {"n": n})
            srv.drain(timeout=30.0)
            waited_ms, answered = barrier(srv, timeout=30.0)
            assert answered
            assert waited_ms >= NOTIFICATIONS * 50 * 0.8, waited_ms
            processed, _dur, err = srv.request("stats/processed", None, timeout=0.5)
            assert (processed, err) == (NOTIFICATIONS, None)
            print(f"PASS: barrier waited {waited_ms} ms until all {NOTIFICATIONS} notifications were processed")
        finally:
            srv.p.kill()


def test_barrier_answer_is_method_not_found():
    with tempfile.TemporaryDirectory() as td:
        srv = _start(td)
        try:
            _res, _dur, err = srv.request(BARRIER_METHOD, None, timeout=10.0)
            assert err == {"code": -32601, "message": "Method not found"}, err
            waited_ms, answered = barrier(srv, timeout=10.0)
            assert answered
            assert waited_ms < 1000.0, waited_ms
            print(f"PASS: barrier with nothing queued answered in {waited_ms} ms")
        finally:
            srv.p.kill()


if __name__ == "__main__":
    test_drain_alone_misses_queued_notifications()
    test_barrier_waits_for_queued_notifications()
    test_barrier_answer_is_method_not_found()
    print("PASS: all lsp barrier tests passed")
