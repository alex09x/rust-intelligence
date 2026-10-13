# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""
Regression test for the stdin/stdout pipe deadlock in SimpleLspProcess.

A single-threaded server reads requests in batches (as a server with a buffered stdin does) and
answers them back to back with responses larger than the OS pipe buffer, so it blocks on stdout
until the client drains it. The client meanwhile keeps sending
requests with a tiny timeout, so its stdin writes eventually block on a full pipe too. With the
blocking write performed under the reader's lock, the reader thread could not drain stdout and both
processes waited forever (observed on the rust-analyzer corpus, typeHierarchy/supertypes). The
client must complete the whole loop and still receive the large responses.
"""

import os
import sys
import tempfile
import threading
import time
from pathlib import Path

bench_dir = Path(__file__).resolve().parent
if str(bench_dir) not in sys.path:
    sys.path.insert(0, str(bench_dir))

from lsp_process import SimpleLspProcess

PAD = "y" * 400  # keeps each request well above 100 bytes so the stdin pipe fills within a few hundred requests

FAKE_SERVER = r"""
import json, os, select, sys, time
inp = os.fdopen(0, "rb", buffering=0)
out = sys.stdout.buffer
def read_exact(n):
    data = b""
    while len(data) < n:
        chunk = inp.read(n - len(data))
        if not chunk:
            return None
        data += chunk
    return data
def read_msg():
    header = b""
    while not header.endswith(b"\r\n\r\n"):
        ch = inp.read(1)
        if not ch:
            return None
        header += ch
    length = int([l for l in header.decode().split("\r\n") if l.lower().startswith("content-length:")][0].split(":")[1])
    body = read_exact(length)
    return None if body is None else json.loads(body)
def write_msg(m):
    b = json.dumps(m).encode()
    out.write(b"Content-Length: %d\r\n\r\n" % len(b) + b)
    out.flush()
def handle(m):
    if m.get("method") == "initialize":
        write_msg({"jsonrpc": "2.0", "id": m["id"], "result": {"capabilities": {}}})
    elif m.get("method") == "typeHierarchy/supertypes":
        time.sleep(0.005)  # slower than the client timeout, so unanswered requests pile up in stdin
        write_msg({"jsonrpc": "2.0", "id": m["id"], "result": [{"name": "x" * 80000, "n": m["params"]["n"]}]})
    elif m.get("method") == "shutdown":
        write_msg({"jsonrpc": "2.0", "id": m["id"], "result": None})
# Like a server with a buffered stdin, take every request that is already waiting (up to 40) before
# answering any of them: the answers then go out back to back without a read from stdin in between.
while True:
    m = read_msg()
    if m is None or m.get("method") == "exit":
        break
    batch = [m]
    while len(batch) < 40 and select.select([inp], [], [], 0)[0]:
        m = read_msg()
        if m is None:
            break
        batch.append(m)
    for m in batch:
        handle(m)
"""



def test_client_survives_large_responses_with_full_stdin_pipe():
    with tempfile.TemporaryDirectory() as td:
        server = Path(td) / "fake_server.py"
        server.write_text(FAKE_SERVER, encoding="utf-8")
        srv = SimpleLspProcess([sys.executable, "-u", str(server)], td, is_ra=False)
        assert srv.initial_quiescent is True

        outcome = {}

        def client_loop():
            timeouts = 0
            late_ids = set()
            for n in range(2000):
                _res, _dur, err = srv.request("typeHierarchy/supertypes", {"n": n, "pad": PAD}, timeout=0.001)
                if err == "TIMEOUT":
                    timeouts += 1
            # The server now works through the backlog; the channel must still deliver a response.
            res, _dur, err = srv.request("typeHierarchy/supertypes", {"n": 2000, "pad": PAD}, timeout=90.0)
            outcome["timeouts"] = timeouts
            outcome["final_err"] = err
            outcome["final_n"] = res[0]["n"] if res else None
            outcome["final_len"] = len(res[0]["name"]) if res else None

        t = threading.Thread(target=client_loop, daemon=True)
        t.start()
        t.join(timeout=120.0)
        alive = t.is_alive()
        try:
            srv.p.kill()
        except Exception:
            pass
        assert not alive, "client loop deadlocked: writer blocked on stdin while the reader waited for the lock"
        assert outcome["final_err"] is None, outcome
        assert outcome["final_n"] == 2000, outcome
        assert outcome["final_len"] == 80000, outcome
        assert outcome["timeouts"] >= 1990, outcome
        print(f"PASS: 2000 requests with a 1 ms timeout against a batching 5 ms server with 80 KB responses completed ({outcome['timeouts']} timeouts); final response received")


def test_write_lock_is_separate_from_reader_lock():
    with tempfile.TemporaryDirectory() as td:
        server = Path(td) / "fake_server.py"
        server.write_text(FAKE_SERVER, encoding="utf-8")
        srv = SimpleLspProcess([sys.executable, "-u", str(server)], td, is_ra=False)
        try:
            assert srv.write_lock is not srv.lock
            with srv.lock:
                acquired = srv.write_lock.acquire(timeout=1.0)
                assert acquired is True
                srv.write_lock.release()
        finally:
            srv.p.kill()
        print("PASS: stdin write lock is independent of the reader lock")


if __name__ == "__main__":
    test_client_survives_large_responses_with_full_stdin_pipe()
    test_write_lock_is_separate_from_reader_lock()
    print("PASS: all lsp pipe deadlock tests passed")
