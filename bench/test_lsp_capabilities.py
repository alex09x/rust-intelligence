# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""
The `initialize` request carries the declared client capabilities, the same for both servers: the
codeLens capability and the rust-analyzer client commands that enable its lenses are present, and
nothing that would change other answers (hover actions, codeLens refresh) is declared.
"""

import json
import sys
import tempfile
from pathlib import Path

bench_dir = Path(__file__).resolve().parent
if str(bench_dir) not in sys.path:
    sys.path.insert(0, str(bench_dir))

from lsp_capabilities import CLIENT_CAPABILITIES, RA_CLIENT_COMMANDS
from lsp_process import SimpleLspProcess

# Records the initialize params to the file named in argv[1], answers it, and reports a quiescent
# server status so that the rust-analyzer flavour of the client finishes its initial wait.
FAKE_SERVER = r"""
import json, sys
inp, out = sys.stdin.buffer, sys.stdout.buffer
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
    if m.get("method") == "initialize":
        open(sys.argv[1], "w", encoding="utf-8").write(json.dumps(m["params"]))
        write_msg({"jsonrpc": "2.0", "id": m["id"], "result": {"capabilities": {}}})
        write_msg({"jsonrpc": "2.0", "method": "experimental/serverStatus",
                   "params": {"health": "ok", "quiescent": True}})
    elif "id" in m:
        write_msg({"jsonrpc": "2.0", "id": m["id"], "error": {"code": -32601, "message": "Method not found"}})
"""


def _initialize_params(td: str, is_ra: bool) -> dict:
    server = Path(td) / "fake_server.py"
    server.write_text(FAKE_SERVER, encoding="utf-8")
    record = Path(td) / f"initialize_{'ra' if is_ra else 'ridx'}.json"
    srv = SimpleLspProcess([sys.executable, "-u", str(server), str(record)], td, is_ra=is_ra)
    try:
        return json.loads(record.read_text(encoding="utf-8"))
    finally:
        srv.close()


def test_initialize_declares_the_same_capabilities_for_both_servers():
    with tempfile.TemporaryDirectory() as td:
        ridx = _initialize_params(td, is_ra=False)
        ra = _initialize_params(td, is_ra=True)
    assert ridx["capabilities"] == ra["capabilities"]
    assert ridx["capabilities"] == CLIENT_CAPABILITIES
    assert ridx["initializationOptions"] == ra["initializationOptions"] == {"checkOnSave": False}
    print("PASS: initialize carries CLIENT_CAPABILITIES for both servers")


def test_capabilities_enable_rust_analyzer_lenses_and_nothing_else():
    caps = CLIENT_CAPABILITIES
    assert caps["textDocument"]["codeLens"] == {"dynamicRegistration": True}
    ca = caps["textDocument"]["codeAction"]
    assert ca["dynamicRegistration"] is True
    assert ca["dataSupport"] is True
    assert ca["resolveSupport"] == {"properties": ["edit"]}
    assert "refactor" in ca["codeActionLiteralSupport"]["codeActionKind"]["valueSet"]
    assert caps["experimental"]["commands"]["commands"] == RA_CLIENT_COMMANDS
    assert RA_CLIENT_COMMANDS == ["rust-analyzer.runSingle", "rust-analyzer.debugSingle", "rust-analyzer.showReferences"]
    # Not declared on purpose: they would change hover, completion and codeAction answers, or make
    # the server send workspace/codeLens/refresh requests.
    for absent in ("rust-analyzer.gotoLocation", "rust-analyzer.triggerParameterHints", "rust-analyzer.rename"):
        assert absent not in caps["experimental"]["commands"]["commands"]
    assert "hoverActions" not in caps["experimental"]
    assert "codeLens" not in caps["workspace"]
    print("PASS: the declared commands are exactly the three that gate rust-analyzer code lenses")


if __name__ == "__main__":
    test_initialize_declares_the_same_capabilities_for_both_servers()
    test_capabilities_enable_rust_analyzer_lenses_and_nothing_else()
    print("PASS: all lsp_capabilities tests passed")
