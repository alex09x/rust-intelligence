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

import json
import os
import subprocess
import threading
import time


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
        self.events = {}
        self.responses = {}
        self.t_sends = {}
        self.t_recvs = {}
        self.is_quiescent = False

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
            with self.lock:
                if self.exited or self.p.stdin.closed:
                    return False
                self.p.stdin.write(hdr + b)
                self.p.stdin.flush()
                if "id" in m:
                    self.t_sends[m["id"]] = time.perf_counter()
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

                    if "method" in msg and "id" in msg:
                        self.handle_server_request(msg)
                    elif "id" in msg:
                        req_id = msg["id"]
                        with self.lock:
                            self.t_recvs[req_id] = t_recv
                            ev = self.events.get(req_id)
                            if ev is not None:
                                self.responses[req_id] = msg
                                ev.set()
                    elif "method" in msg:
                        if msg.get("method") == "experimental/serverStatus":
                            self.is_quiescent = msg.get("params", {}).get("quiescent", False)
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
                "capabilities": {
                    "window": {"workDoneProgress": True},
                    "workspace": {
                        "configuration": True,
                        "workspaceFolders": True,
                        "applyEdit": True,
                        "symbol": {"dynamicRegistration": True},
                    },
                    "textDocument": {
                        "synchronization": {"dynamicRegistration": True, "willSave": True, "willSaveWaitUntil": True, "didSave": True},
                        "definition": {"dynamicRegistration": True, "linkSupport": True},
                        "references": {"dynamicRegistration": True},
                        "hover": {"dynamicRegistration": True, "contentFormat": ["markdown", "plaintext"]},
                        "completion": {"dynamicRegistration": True, "completionItem": {"snippetSupport": True}},
                        "signatureHelp": {"dynamicRegistration": True},
                        "documentSymbol": {"dynamicRegistration": True, "hierarchicalDocumentSymbolSupport": True},
                        "semanticTokens": {
                            "dynamicRegistration": True,
                            "tokenTypes": [
                                "comment", "keyword", "string", "number", "regexp", "operator", "namespace",
                                "type", "struct", "class", "interface", "enum", "typeParameter", "function",
                                "method", "decorator", "macro", "variable", "parameter", "property", "label",
                            ],
                            "tokenModifiers": [
                                "declaration", "definition", "readonly", "static", "deprecated",
                                "abstract", "async", "modification", "documentation", "defaultLibrary",
                            ],
                            "requests": {"full": True, "range": True},
                            "formats": ["relative"],
                        },
                        "inlayHint": {"dynamicRegistration": True},
                        "codeAction": {"dynamicRegistration": True},
                        "formatting": {"dynamicRegistration": True},
                    },
                    "experimental": {"serverStatusNotification": True},
                },
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
                self.wait_quiescent(timeout=20.0, settle_idle=0.5)

    def wait_quiescent(self, timeout=20.0, settle_idle=0.5):
        t0 = time.perf_counter()
        while time.perf_counter() - t0 < timeout:
            if self.exited or self.p.poll() is not None:
                self.exited = True
                break
            if self.is_quiescent:
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

        req = {"jsonrpc": "2.0", "id": req_id, "method": method, "params": params}
        t0 = time.perf_counter()

        if not self.send_msg(req):
            with self.lock:
                self.events.pop(req_id, None)
            return None, 0.0, "BROKEN_PIPE"

        signaled = ev.wait(timeout=timeout)

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
