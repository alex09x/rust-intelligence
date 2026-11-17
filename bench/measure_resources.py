# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0

"""Resource use of an LSP server under an editor-like workload: RSS (with child processes) and CPU seconds.

Linux only (reads /proc). "ready" is the initialize reply for ridx, which reads an index built beforehand by `ridx index`,
and quiescence (all progress ended) for rust-analyzer, which analyzes the workspace after start.
Results of the published run: bench/published/resources_f7105fcc.jsonl (export: bench/export_resources.py).

Usage: measure_resources.py BENCH_DIR SERVER_KIND(ridx|ra) SERVER_BIN REPO_DIR OUT_JSON [N_FILES]
"""
import json
import os
import random
import re
import sys
import threading
import time
from pathlib import Path

sys.path.insert(0, sys.argv[1])
from lsp_process import SimpleLspProcess  # noqa: E402

kind, server_bin, repo, out = sys.argv[2], sys.argv[3], Path(sys.argv[4]).resolve(), sys.argv[5]
n_files = int(sys.argv[6]) if len(sys.argv) > 6 else 40
TICK = os.sysconf("SC_CLK_TCK")


def tree(pid):
    kids = {}
    for d in os.listdir("/proc"):
        if d.isdigit():
            try:
                ppid = int(open(f"/proc/{d}/stat").read().rsplit(")", 1)[1].split()[1])
                kids.setdefault(ppid, []).append(int(d))
            except (OSError, IndexError, ValueError):
                pass
    out_, stack = [], [pid]
    while stack:
        p = stack.pop()
        out_.append(p)
        stack.extend(kids.get(p, []))
    return out_


def sample(pid):
    rss, cpu = 0, 0.0
    for p in tree(pid):
        try:
            for line in open(f"/proc/{p}/status"):
                if line.startswith("VmRSS:"):
                    rss += int(line.split()[1])
            f = open(f"/proc/{p}/stat").read().rsplit(")", 1)[1].split()
            cpu += (int(f[11]) + int(f[12]) + int(f[13]) + int(f[14])) / TICK
        except (OSError, IndexError, ValueError):
            pass
    return rss / 1024.0, cpu


t0 = time.perf_counter()
cmd = [server_bin, "lsp"] if kind == "ridx" else [server_bin]
srv = SimpleLspProcess(cmd, str(repo), is_ra=(kind == "ra"))
pid = srv.p.pid
samples, stop = [], threading.Event()


def sampler():
    while not stop.is_set():
        r, c = sample(pid)
        samples.append((time.perf_counter() - t0, r, c))
        time.sleep(0.05)


threading.Thread(target=sampler, daemon=True).start()
ready_s = time.perf_counter() - t0
time.sleep(1.0)
rss_ready, cpu_ready = sample(pid)

files = sorted(p for p in repo.rglob("*.rs") if "target" not in p.parts and p.stat().st_size < 200_000)
random.Random(7).shuffle(files)
files = files[:n_files]
ident = re.compile(r"\b[A-Za-z_][A-Za-z0-9_]{2,}\b")
counts, errors, lat = {}, 0, {}


def ask(method, params, timeout=30.0):
    global errors
    res, us, err = srv.request(method, params, timeout=timeout)
    counts[method] = counts.get(method, 0) + 1
    lat.setdefault(method, []).append(us)
    if err:
        errors += 1
    return res


cpu_w0 = sample(pid)[1]
tw0 = time.perf_counter()
for f in files:
    uri = f"file://{f}"
    text = f.read_text(encoding="utf-8", errors="replace")
    srv.notify("textDocument/didOpen", {"textDocument": {"uri": uri, "languageId": "rust", "version": 1, "text": text}})
    td = {"uri": uri}
    ask("textDocument/documentSymbol", {"textDocument": td})
    ask("textDocument/semanticTokens/full", {"textDocument": td})
    ask("textDocument/foldingRange", {"textDocument": td})
    lines = text.split("\n")
    ask("textDocument/inlayHint", {"textDocument": td, "range": {"start": {"line": 0, "character": 0}, "end": {"line": min(len(lines), 200), "character": 0}}})
    pos = [(i, m.start()) for i, ln in enumerate(lines) if not ln.lstrip().startswith("//") for m in ident.finditer(ln)]
    random.Random(len(text)).shuffle(pos)
    for (ln, col) in pos[:8]:
        p = {"textDocument": td, "position": {"line": ln, "character": col}}
        ask("textDocument/hover", p)
        ask("textDocument/definition", p)
        ask("textDocument/typeDefinition", p)
        ask("textDocument/documentHighlight", p)
    for (ln, col) in pos[:2]:
        ask("textDocument/references", {"textDocument": td, "position": {"line": ln, "character": col}, "context": {"includeDeclaration": True}})
    dots = [(i, ln.index(".") + 1) for i, ln in enumerate(lines) if re.search(r"[a-z_)]\.[a-z_]", ln)][:2]
    for (ln, col) in dots:
        ask("textDocument/completion", {"textDocument": td, "position": {"line": ln, "character": col}})
    # An edit: a new line with a local binding, then requests against the dirty buffer.
    if pos:
        edit = {"range": {"start": {"line": pos[0][0], "character": 0}, "end": {"line": pos[0][0], "character": 0}}, "text": "\n"}
        srv.notify("textDocument/didChange", {"textDocument": {"uri": uri, "version": 2}, "contentChanges": [edit]})
        ask("textDocument/semanticTokens/full", {"textDocument": td})
        ask("textDocument/hover", {"textDocument": td, "position": {"line": pos[0][0] + 1, "character": pos[0][1]}})
    srv.notify("textDocument/didClose", {"textDocument": td})
work_s = time.perf_counter() - tw0
rss_after, cpu_after = sample(pid)
time.sleep(10.0)
rss_idle, cpu_idle = sample(pid)
stop.set()
srv.close()


def pct(v, q):
    v = sorted(v)
    return round(v[min(len(v) - 1, int(q * len(v)))] / 1000.0, 2) if v else None


res = {
    "kind": kind, "repo": repo.name, "files": len(files), "requests": sum(counts.values()), "errors": errors,
    "ready_s": round(ready_s, 2), "rss_ready_mb": round(rss_ready), "cpu_startup_s": round(cpu_ready, 2),
    "peak_rss_mb": round(max(s[1] for s in samples)), "rss_after_work_mb": round(rss_after),
    "rss_idle_10s_mb": round(rss_idle), "work_wall_s": round(work_s, 2), "cpu_work_s": round(cpu_after - cpu_w0, 2),
    "cpu_idle_10s_s": round(cpu_idle - cpu_after, 2),
    "lat_ms_p50_p95": {m: (pct(v, 0.5), pct(v, 0.95)) for m, v in lat.items()},
}
Path(out).write_text(json.dumps(res, indent=1))
print(json.dumps(res))
