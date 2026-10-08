# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""
Lightweight, robust LSP process client for benchmark queries and synchronization.
Uses non-blocking streaming I/O with explicit framing buffers to eliminate pipe deadlocks.
"""

import json
import os
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
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE
        )
        self.buffer = bytearray()
        self.next_id = 1
        self.exited = False
        # Set stdout pipe to non-blocking to prevent read deadlocks
        try:
            os.set_blocking(self.p.stdout.fileno(), False)
        except Exception:
            pass
        self._init_server()

    def send_msg(self, m):
        if self.exited or self.p.poll() is not None:
            self.exited = True
            return False
        try:
            b = json.dumps(m).encode("utf-8")
            hdr = f"Content-Length: {len(b)}\r\n\r\n".encode("utf-8")
            self.p.stdin.write(hdr + b)
            self.p.stdin.flush()
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
        else:
            self.send_response(req_id, None)

    def _parse_message_from_buffer(self):
        """Attempts to extract one framed JSON-RPC message from self.buffer."""
        sep = b"\r\n\r\n"
        idx = self.buffer.find(sep)
        sep_len = 4
        if idx == -1:
            sep = b"\n\n"
            idx = self.buffer.find(sep)
            sep_len = 2
        if idx == -1:
            return None

        header_bytes = bytes(self.buffer[:idx])
        content_length = None
        for line in header_bytes.decode("latin1", errors="ignore").splitlines():
            line = line.strip()
            if line.lower().startswith("content-length:"):
                try:
                    content_length = int(line.split(":", 1)[1].strip())
                except ValueError:
                    pass

        if content_length is None:
            del self.buffer[:idx + sep_len]
            return None

        body_start = idx + sep_len
        body_end = body_start + content_length
        if len(self.buffer) < body_end:
            return None  # Partial payload, await more bytes

        payload = bytes(self.buffer[body_start:body_end])
        del self.buffer[:body_end]
        try:
            return json.loads(payload.decode("utf-8", errors="ignore"))
        except Exception:
            return None

    def read_msg(self, timeout=1.0):
        """Reads one JSON-RPC message with timeout, avoiding peek deadlocks."""
        if self.exited or self.p.poll() is not None:
            self.exited = True
            return None

        # 1. Check existing buffer
        msg = self._parse_message_from_buffer()
        if msg is not None:
            return msg

        # 2. Wait for data on non-blocking pipe
        deadline = time.perf_counter() + timeout
        while time.perf_counter() < deadline:
            if self.p.poll() is not None:
                self.exited = True
                return None

            remaining = max(0.001, deadline - time.perf_counter())
            r, _, _ = select.select([self.p.stdout], [], [], min(0.1, remaining))
            if r:
                try:
                    chunk = os.read(self.p.stdout.fileno(), 65536)
                    if not chunk:
                        self.exited = True
                        return None  # EOF
                    self.buffer.extend(chunk)
                    msg = self._parse_message_from_buffer()
                    if msg is not None:
                        return msg
                except (BlockingIOError, InterruptedError):
                    pass
                except Exception:
                    self.exited = True
                    return None
        return None

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
        self.send_msg(req)
        t0 = time.perf_counter()
        while time.perf_counter() - t0 < 30.0:
            if self.exited or self.p.poll() is not None:
                self.exited = True
                break
            msg = self.read_msg(timeout=0.5)
            if not msg:
                continue
            if "method" in msg and "id" in msg:
                self.handle_server_request(msg)
                continue
            if msg.get("id") == init_id:
                break

        if not self.exited:
            self.send_msg({"jsonrpc": "2.0", "method": "initialized", "params": {}})
            if self.is_ra:
                self.wait_quiescent(timeout=30.0, settle_idle=1.0)

    def wait_quiescent(self, timeout=20.0, settle_idle=0.5):
        t0 = time.perf_counter()
        seen_busy = False
        last_busy = t0
        while time.perf_counter() - t0 < timeout:
            if self.exited or self.p.poll() is not None:
                self.exited = True
                return (time.perf_counter() - t0) * 1000.0
            msg = self.read_msg(timeout=0.1)
            if not msg:
                if seen_busy and (time.perf_counter() - last_busy > settle_idle):
                    return (time.perf_counter() - t0) * 1000.0
                if not seen_busy and (time.perf_counter() - t0 > settle_idle):
                    return (time.perf_counter() - t0) * 1000.0
                continue
            if "method" in msg and "id" in msg:
                self.handle_server_request(msg)
            if msg.get("method") == "experimental/serverStatus":
                quiescent = msg.get("params", {}).get("quiescent", False)
                if not quiescent:
                    seen_busy = True
                    last_busy = time.perf_counter()
                elif quiescent and seen_busy:
                    return (time.perf_counter() - t0) * 1000.0
        return (time.perf_counter() - t0) * 1000.0

    def request(self, method, params, timeout=5.0):
        if self.exited or self.p.poll() is not None:
            self.exited = True
            return None, 0.0, "PROCESS_EXITED"
        req_id = self.next_id
        self.next_id += 1
        req = {"jsonrpc": "2.0", "id": req_id, "method": method, "params": params}
        t0 = time.perf_counter()
        if not self.send_msg(req):
            return None, 0.0, "BROKEN_PIPE"
        while time.perf_counter() - t0 < timeout:
            if self.exited or self.p.poll() is not None:
                self.exited = True
                dur_us = (time.perf_counter() - t0) * 1_000_000.0
                return None, dur_us, "PROCESS_EXITED"
            msg = self.read_msg(timeout=0.2)
            if not msg:
                continue
            if "method" in msg and "id" in msg:
                self.handle_server_request(msg)
                continue
            if msg.get("id") == req_id:
                dur_us = (time.perf_counter() - t0) * 1_000_000.0
                return msg.get("result"), dur_us, msg.get("error")
        dur_us = (time.perf_counter() - t0) * 1_000_000.0
        return None, dur_us, "TIMEOUT"

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
                self.send_msg({"jsonrpc": "2.0", "id": req_id, "method": "shutdown", "params": None})
                t0 = time.perf_counter()
                while time.perf_counter() - t0 < 1.0:
                    msg = self.read_msg(timeout=0.2)
                    if msg and msg.get("id") == req_id:
                        break
                self.send_msg({"jsonrpc": "2.0", "method": "exit", "params": None})
                self.p.stdin.close()
                self.p.wait(timeout=1.0)
        except Exception:
            pass
        finally:
            try:
                self.p.kill()
                self.p.wait(timeout=0.5)
            except Exception:
                pass
            self.exited = True
