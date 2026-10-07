# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""
Lightweight LSP process client for benchmark queries and synchronization.
"""

import json
import select
import subprocess
import time

class SimpleLspProcess:
    def __init__(self, cmd, cwd, is_ra=False):
        self.cmd = cmd
        self.cwd = str(cwd)
        self.is_ra = is_ra
        self.p = subprocess.Popen(
            cmd, cwd=self.cwd,
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL
        )
        self.next_id = 1
        self._init_server()

    def send_msg(self, m):
        b = json.dumps(m).encode("utf-8")
        hdr = f"Content-Length: {len(b)}\r\n\r\n".encode("utf-8")
        self.p.stdin.write(hdr + b)
        self.p.stdin.flush()

    def read_msg(self, timeout=1.0):
        r, _, _ = select.select([self.p.stdout], [], [], timeout)
        if not r:
            return None
        line = self.p.stdout.readline().decode(errors="ignore")
        if not line.startswith("Content-Length:"):
            return None
        cl = int(line.split(":")[1].strip())
        self.p.stdout.readline()
        data = self.p.stdout.read(cl).decode(errors="ignore")
        return json.loads(data)

    def _init_server(self):
        req = {
            "jsonrpc": "2.0", "id": self.next_id, "method": "initialize",
            "params": {
                "processId": None, "rootUri": f"file://{self.cwd}",
                "initializationOptions": {"checkOnSave": False},
                "capabilities": {
                    "window": {"workDoneProgress": True},
                    "experimental": {"serverStatusNotification": True},
                },
            },
        }
        self.next_id += 1
        self.send_msg(req)
        self.read_msg(timeout=5.0)
        self.send_msg({"jsonrpc": "2.0", "method": "initialized", "params": {}})
        if self.is_ra:
            self.wait_quiescent(timeout=30.0, settle_idle=1.0)

    def wait_quiescent(self, timeout=20.0, settle_idle=0.5):
        t0 = time.perf_counter()
        seen_busy = False
        last_busy = t0
        while time.perf_counter() - t0 < timeout:
            msg = self.read_msg(timeout=0.1)
            if not msg:
                if seen_busy and (time.perf_counter() - last_busy > settle_idle):
                    return (time.perf_counter() - t0) * 1000.0
                if not seen_busy and (time.perf_counter() - t0 > settle_idle):
                    return (time.perf_counter() - t0) * 1000.0
                continue
            if "method" in msg and "id" in msg:
                self.send_msg({"jsonrpc": "2.0", "id": msg["id"], "result": None})
            if msg.get("method") == "experimental/serverStatus":
                quiescent = msg.get("params", {}).get("quiescent", False)
                if not quiescent:
                    seen_busy = True
                    last_busy = time.perf_counter()
                elif quiescent and seen_busy:
                    return (time.perf_counter() - t0) * 1000.0
        return (time.perf_counter() - t0) * 1000.0

    def notify_watched_files(self, uris):
        changes = [{"uri": u, "type": 2} for u in uris]
        self.send_msg({
            "jsonrpc": "2.0",
            "method": "workspace/didChangeWatchedFiles",
            "params": {"changes": changes},
        })

    def query_def(self, uri, line, col, timeout=5.0):
        req_id = self.next_id
        self.next_id += 1
        req = {
            "jsonrpc": "2.0", "id": req_id, "method": "textDocument/definition",
            "params": {
                "textDocument": {"uri": uri},
                "position": {"line": line, "character": col},
            },
        }
        t0 = time.perf_counter()
        self.send_msg(req)
        while time.perf_counter() - t0 < timeout:
            msg = self.read_msg(timeout=0.2)
            if not msg:
                continue
            if "method" in msg and "id" in msg:
                self.send_msg({"jsonrpc": "2.0", "id": msg["id"], "result": None})
            if msg.get("id") == req_id:
                dur_us = (time.perf_counter() - t0) * 1_000_000.0
                return msg.get("result"), dur_us
        return None, (time.perf_counter() - t0) * 1_000_000.0

    def close(self):
        try:
            self.p.kill()
            self.p.wait()
        except Exception:
            pass
