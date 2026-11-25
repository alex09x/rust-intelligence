# rust-intelligence — LSP smoke test of release binaries: an editor's session on a small crate.
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# usage: python3 lsp_smoke.py BIN_DIR
#
# Starts `ridx lsp`, opens a crate as an editor does (file URIs as VS Code writes them: on Windows `file:///c%3A/...`)
# and checks definition, hover and references of a method call, then shutdown and exit. Exits 1 on any miss.
import json
import os
import subprocess
import sys
import tempfile
import time
import urllib.parse
from pathlib import Path

LIB = """pub struct Greeter;

impl Greeter {
    pub fn hello(&self) -> String {
        String::from("hi")
    }
}

pub fn run() -> String {
    Greeter.hello()
}

pub fn again() -> String {
    Greeter.hello()
}
"""


def uri(path: Path) -> str:
    """The URI VS Code sends: on Windows a lower-case drive letter with its colon escaped."""
    text = path.resolve().as_posix()
    if len(text) > 1 and text[1] == ":":
        return "file:///" + text[0].lower() + "%3A" + urllib.parse.quote(text[2:])
    return "file://" + urllib.parse.quote(text)


class Server:
    def __init__(self, cmd, cwd):
        self.p = subprocess.Popen(cmd, cwd=cwd, stdin=subprocess.PIPE, stdout=subprocess.PIPE)
        self.next_id = 0

    def send(self, msg):
        body = json.dumps(msg).encode()
        self.p.stdin.write(b"Content-Length: %d\r\n\r\n" % len(body) + body)
        self.p.stdin.flush()

    def read(self):
        length = None
        while True:
            line = self.p.stdout.readline()
            if not line:
                raise SystemExit("the server closed its output")
            line = line.strip()
            if not line:
                break
            if line.lower().startswith(b"content-length:"):
                length = int(line.split(b":")[1])
        return json.loads(self.p.stdout.read(length))

    def request(self, method, params):
        self.next_id += 1
        my_id = self.next_id
        self.send({"jsonrpc": "2.0", "id": my_id, "method": method, "params": params})
        while True:
            msg = self.read()
            if "method" in msg and "id" in msg:  # a request of the server's: answer it
                self.send({"jsonrpc": "2.0", "id": msg["id"], "result": None})
            elif msg.get("id") == my_id:
                return msg

    def notify(self, method, params):
        self.send({"jsonrpc": "2.0", "method": method, "params": params})


def main():
    bin_dir = Path(sys.argv[1]).resolve()
    ridx = bin_dir / ("ridx.exe" if os.name == "nt" else "ridx")
    work = Path(tempfile.mkdtemp(prefix="ridx-lsp-smoke-"))
    (work / "src").mkdir()
    (work / "Cargo.toml").write_text('[package]\nname = "smoke"\nversion = "0.1.0"\nedition = "2021"\n')
    lib = work / "src" / "lib.rs"
    lib.write_text(LIB)
    doc = {"uri": uri(lib)}
    s = Server([str(ridx), "lsp"], work)
    init = s.request("initialize", {"processId": os.getpid(), "rootUri": uri(work), "capabilities": {}})
    print("server:", init["result"].get("serverInfo"))
    s.notify("initialized", {})
    s.notify("textDocument/didOpen", {"textDocument": {**doc, "languageId": "rust", "version": 1, "text": LIB}})
    at_call = {"textDocument": doc, "position": {"line": 9, "character": 13}}  # `hello` in run()

    failures = []
    deadline = time.time() + 300
    definition = None
    while time.time() < deadline:
        definition = s.request("textDocument/definition", at_call).get("result")
        if definition:
            break
        time.sleep(2)
    print("definition:", json.dumps(definition))
    target = definition[0] if isinstance(definition, list) and definition else definition or {}
    if not (target.get("uri", "").lower().endswith("/src/lib.rs") and target.get("range", {}).get("start", {}).get("line") == 3):
        failures.append("definition of hello() is not src/lib.rs line 4")

    hover = s.request("textDocument/hover", at_call).get("result") or {}
    text = json.dumps(hover)
    print("hover:", text[:300])
    if "fn hello" not in text:
        failures.append("hover of hello() does not show its signature")

    refs = s.request("textDocument/references", {**at_call, "context": {"includeDeclaration": False}}).get("result") or []
    lines = sorted(r["range"]["start"]["line"] for r in refs)
    print("references at lines:", lines, "uris:", sorted({r["uri"] for r in refs}))
    if lines != [9, 13]:
        failures.append(f"references of hello() at lines {lines}, expected [9, 13]")

    s.request("shutdown", None)
    s.notify("exit", None)
    code = s.p.wait(timeout=20)
    print("exit status:", code)
    if code != 0:
        failures.append(f"exit status {code}")
    for f in failures:
        print("FAIL:", f)
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
