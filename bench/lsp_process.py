# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""
Lightweight, thread-safe LSP process client for benchmark queries and synchronization.
Uses an asynchronous background reader thread to continuously drain stdout, eliminating
OS pipe buffer deadlocks caused by unconsumed server notifications (e.g. publishDiagnostics).
"""

import copy
import json
import os
import subprocess
import threading
import time

from lsp_capabilities import CLIENT_CAPABILITIES


class SimpleLspProcess:
    def __init__(self, cmd, cwd, is_ra=False):
        self.cmd = cmd
        self.cwd = str(cwd)
        self.is_ra = is_ra
        self.stderr_path = f"/tmp/lsp_{'ra' if is_ra else 'ridx'}_{os.getpid()}_{id(self)}.stderr"
        self.stderr_file = open(self.stderr_path, "w", encoding="utf-8", errors="replace")
        self.p = subprocess.Popen(
            cmd, cwd=self.cwd,
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self.stderr_file
        )
        self.next_id = 1
        self.exited = False
        self.lock = threading.Lock()
        # Serializes writes to the server's stdin. It is never held together with self.lock, so a
        # write that blocks on a full stdin pipe cannot stop the reader thread from draining stdout.
        self.write_lock = threading.Lock()
        # Ids of requests sent but not yet answered, including ones the client stopped waiting for.
        self.outstanding = set()
        # A request that times out is cancelled with $/cancelRequest, as an editor does, so the
        # server can drop it from its queue instead of computing an answer nobody reads.
        self.cancel_on_timeout = True
        self.cancelled_on_timeout = 0
        self.events = {}
        self.responses = {}
        self.t_sends = {}
        self.t_recvs = {}
        self.is_quiescent = False
        self.initial_quiescent = False
        self.initial_wait_ms = 0.0
        self.t_init_start = time.perf_counter()
        self.active_progress_tokens = set()
        self.progress_titles_seen = []
        self.progress_events = []
        self.active_token_titles = {}
        self.last_msg_time = time.perf_counter()
        self.open_docs = set()

        self.reader_thread = threading.Thread(target=self._reader_loop, daemon=True)
        self.reader_thread.start()
        self._init_server()

    def send_msg(self, m):
        if self.exited or self.p.poll() is not None:
            self.exited = True
            return False
        try:
            b = json.dumps(m).encode("utf-8")
            hdr = f"Content-Length: {len(b)}\r\n\r\n".encode("utf-8")
            # The blocking write happens outside self.lock. When the server is itself blocked on a
            # full stdout pipe (a large response) and this write blocks on a full stdin pipe, the
            # reader thread must still be able to take self.lock and drain stdout; otherwise both
            # processes wait on each other forever.
            with self.write_lock:
                if self.exited or self.p.stdin.closed:
                    return False
                self.p.stdin.write(hdr + b)
                self.p.stdin.flush()
            with self.lock:
                if "id" in m:
                    self.t_sends[m["id"]] = time.perf_counter()
                method = m.get("method")
                params = m.get("params") or {}
                if isinstance(params, dict):
                    td = params.get("textDocument") or {}
                    if isinstance(td, dict):
                        uri = td.get("uri")
                        if uri:
                            if method == "textDocument/didOpen":
                                self.open_docs.add(uri)
                            elif method == "textDocument/didClose":
                                self.open_docs.discard(uri)
            return True
        except (BrokenPipeError, OSError):
            self.exited = True
            return False

    def send_response(self, req_id, result=None):
        self.send_msg({"jsonrpc": "2.0", "id": req_id, "result": result})

    def handle_server_request(self, msg):
        """Answers server-to-client requests with schema-valid responses."""
        req_id = msg.get("id")
        method = msg.get("method", "")
        if method == "workspace/configuration":
            items = msg.get("params", {}).get("items", [{}])
            self.send_response(req_id, [{}] * len(items))
        elif method == "workspace/workspaceFolders":
            self.send_response(req_id, [{"uri": f"file://{self.cwd}", "name": "root"}])
        elif method == "workspace/applyEdit":
            self.send_response(req_id, {"applied": True})
        elif method == "window/workDoneProgress/create":
            self.send_response(req_id, None)
        elif method in ("client/registerCapability", "client/unregisterCapability"):
            self.send_response(req_id, None)
        else:
            self.send_response(req_id, None)

    def _reader_loop(self):
        """Continuously drains stdout into framed JSON-RPC messages."""
        buf = bytearray()
        sep = b"\r\n\r\n"
        alt_sep = b"\n\n"

        while not self.exited and self.p.poll() is None:
            try:
                chunk = os.read(self.p.stdout.fileno(), 65536)
                if not chunk:
                    break
                buf.extend(chunk)

                while True:
                    idx = buf.find(sep)
                    sep_len = 4
                    if idx == -1:
                        idx = buf.find(alt_sep)
                        sep_len = 2
                    if idx == -1:
                        break

                    header_bytes = bytes(buf[:idx])
                    cl = None
                    for line in header_bytes.decode("latin1", errors="ignore").splitlines():
                        if line.lower().startswith("content-length:"):
                            try:
                                cl = int(line.split(":", 1)[1].strip())
                            except ValueError:
                                pass
                            break

                    if cl is None:
                        del buf[:idx + sep_len]
                        continue

                    body_end = idx + sep_len + cl
                    if len(buf) < body_end:
                        break  # Await more payload bytes

                    t_recv = time.perf_counter()
                    payload = bytes(buf[idx + sep_len:body_end])
                    del buf[:body_end]

                    try:
                        msg = json.loads(payload.decode("utf-8", errors="ignore"))
                    except Exception:
                        continue

                    with self.lock:
                        self.last_msg_time = t_recv

                    if "method" in msg and "id" in msg:
                        self.handle_server_request(msg)
                    elif "id" in msg:
                        req_id = msg["id"]
                        with self.lock:
                            self.outstanding.discard(req_id)
                            ev = self.events.get(req_id)
                            if ev is not None:
                                self.t_recvs[req_id] = t_recv
                                self.responses[req_id] = msg
                                ev.set()
                            else:
                                # Late answer to a request the client stopped waiting for.
                                self.t_sends.pop(req_id, None)
                    elif "method" in msg:
                        method = msg.get("method")
                        if method == "experimental/serverStatus":
                            with self.lock:
                                self.is_quiescent = msg.get("params", {}).get("quiescent", False)
                        elif method == "$/progress":
                            params = msg.get("params", {})
                            tok = params.get("token")
                            val = params.get("value", {})
                            kind = val.get("kind")
                            with self.lock:
                                if kind == "begin":
                                    title = val.get("title")
                                    if tok is not None and title:
                                        self.active_token_titles[str(tok)] = str(title)
                                    if tok is not None:
                                        self.active_progress_tokens.add(str(tok))
                                    if title and str(title) not in self.progress_titles_seen:
                                        self.progress_titles_seen.append(str(title))
                                elif kind == "end":
                                    title = val.get("title") or self.active_token_titles.get(str(tok), "")
                                    if tok is not None:
                                        self.active_progress_tokens.discard(str(tok))
                                else:
                                    title = val.get("title") or self.active_token_titles.get(str(tok), "")

                                if kind in ("begin", "end"):
                                    t_ms = round((time.perf_counter() - self.t_init_start) * 1000.0, 2)
                                    self.progress_events.append({
                                        "title": str(title or ""),
                                        "kind": str(kind),
                                        "t_ms": t_ms,
                                    })
            except Exception:
                break

        self.exited = True
        with self.lock:
            for ev in self.events.values():
                ev.set()

    def _init_server(self):
        init_id = self.next_id
        self.next_id += 1
        req = {
            "jsonrpc": "2.0", "id": init_id, "method": "initialize",
            "params": {
                "processId": None, "rootUri": f"file://{self.cwd}",
                "initializationOptions": {"checkOnSave": False},
                "capabilities": copy.deepcopy(CLIENT_CAPABILITIES),
            },
        }

        ev = threading.Event()
        with self.lock:
            self.events[init_id] = ev

        if not self.send_msg(req):
            return

        ev.wait(timeout=30.0)
        with self.lock:
            self.events.pop(init_id, None)
            self.responses.pop(init_id, None)

        if not self.exited:
            self.send_msg({"jsonrpc": "2.0", "method": "initialized", "params": {}})
            if self.is_ra:
                self.initial_wait_ms = self.wait_quiescent(timeout=600.0, settle_idle=0.5)
                with self.lock:
                    self.initial_quiescent = bool(self.is_quiescent and len(self.active_progress_tokens) == 0)
            else:
                self.initial_quiescent = True
                self.initial_wait_ms = 0.0

    def wait_quiescent(self, timeout=600.0, settle_idle=0.5):
        """Waits until server is quiescent, all progress tokens complete, and idle for settle_idle seconds."""
        t0 = time.perf_counter()
        while time.perf_counter() - t0 < timeout:
            if self.exited or self.p.poll() is not None:
                self.exited = True
                break
            with self.lock:
                quiescent = self.is_quiescent
                no_active = len(self.active_progress_tokens) == 0
                idle = (time.perf_counter() - self.last_msg_time) >= settle_idle
            if quiescent and no_active and idle:
                break
            time.sleep(0.05)
        return (time.perf_counter() - t0) * 1000.0

    def request(self, method, params, timeout=5.0):
        if self.exited or self.p.poll() is not None:
            self.exited = True
            return None, 0.0, "PROCESS_EXITED"

        req_id = self.next_id
        self.next_id += 1
        ev = threading.Event()

        with self.lock:
            self.events[req_id] = ev
            self.outstanding.add(req_id)

        req = {"jsonrpc": "2.0", "id": req_id, "method": method, "params": params}
        t0 = time.perf_counter()

        if not self.send_msg(req):
            with self.lock:
                self.events.pop(req_id, None)
                self.outstanding.discard(req_id)
            return None, 0.0, "BROKEN_PIPE"

        signaled = ev.wait(timeout=timeout)
        if not signaled and self.cancel_on_timeout:
            with self.lock:
                still_pending = req_id in self.outstanding
            if still_pending:
                self.notify("$/cancelRequest", {"id": req_id})
                self.cancelled_on_timeout += 1

        with self.lock:
            self.events.pop(req_id, None)
            msg = self.responses.pop(req_id, None)
            t_send = self.t_sends.pop(req_id, None)
            t_recv = self.t_recvs.pop(req_id, None)

        if t_send is not None and t_recv is not None:
            dur_us = (t_recv - t_send) * 1_000_000.0
        else:
            dur_us = (time.perf_counter() - t0) * 1_000_000.0

        if self.exited or self.p.poll() is not None:
            self.exited = True
            return None, dur_us, "PROCESS_EXITED"

        if not signaled or msg is None:
            return None, dur_us, "TIMEOUT"

        return msg.get("result"), dur_us, msg.get("error")

    def drain(self, timeout=600.0, poll=0.05):
        """Waits until every request sent so far has been answered or rejected by the server.

        Returns (waited_ms, still_outstanding). Called between benchmark methods so the backlog of
        one method cannot delay, and time out, the first requests of the next one.
        """
        t0 = time.perf_counter()
        while True:
            with self.lock:
                left = len(self.outstanding)
            if left == 0 or self.exited or self.p.poll() is not None:
                break
            if time.perf_counter() - t0 >= timeout:
                break
            time.sleep(poll)
        return round((time.perf_counter() - t0) * 1000.0, 2), left

    def notify(self, method, params):
        return self.send_msg({"jsonrpc": "2.0", "method": method, "params": params})

    def notify_watched_files(self, uris):
        changes = [{"uri": u, "type": 2} for u in uris]
        self.notify("workspace/didChangeWatchedFiles", {"changes": changes})

    def query_def(self, uri, line, col, timeout=5.0):
        params = {
            "textDocument": {"uri": uri},
            "position": {"line": line, "character": col},
        }
        res, dur, _ = self.request("textDocument/definition", params, timeout=timeout)
        return res, dur

    def close(self):
        try:
            if not self.exited and self.p.poll() is None:
                req_id = self.next_id
                self.next_id += 1
                ev = threading.Event()
                with self.lock:
                    self.events[req_id] = ev
                self.send_msg({"jsonrpc": "2.0", "id": req_id, "method": "shutdown", "params": None})
                ev.wait(timeout=1.0)
                with self.lock:
                    self.events.pop(req_id, None)
                    self.responses.pop(req_id, None)
                self.send_msg({"jsonrpc": "2.0", "method": "exit", "params": None})
                with self.lock:
                    self.p.stdin.close()
                self.p.wait(timeout=1.0)
        except Exception:
            pass
        finally:
            self.exited = True
            try:
                self.p.kill()
                self.p.wait(timeout=0.5)
            except Exception:
                pass
            try:
                self.reader_thread.join(timeout=0.5)
            except Exception:
                pass
            try:
                if hasattr(self, "stderr_file") and not self.stderr_file.closed:
                    self.stderr_file.close()
            except Exception:
                pass
