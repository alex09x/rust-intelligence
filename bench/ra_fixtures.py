# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""rust-analyzer's own IDE fixtures, run against ridx on real Cargo workspaces.

    ra_fixtures.py extract --tests RA/crates/ide/src/goto_definition.rs --out cases.json
    ra_fixtures.py run --cases cases.json --ridx BIN --work DIR [--jobs 8] [--only NAME,...] --out results.json

rust-analyzer checks goto-definition on in-memory fixtures (`//- /lib.rs crate:foo deps:bar`, a `$0` cursor and
`//^^^` marks under the expected targets). ridx answers from what the compiler indexed, so each fixture becomes a
Cargo workspace: one package per fixture crate (its dependencies as path dependencies under their fixture names),
the fixture's files at their paths, then `ridx index` and one LSP session asking `textDocument/definition` at the
cursor. A case passes when the set of (file, start) of the answer equals the marked one; `check_unresolved` and
`check_no_definition` expect nothing. Fixtures on rust-analyzer's `minicore`, proc macros or `cfg:` are skipped
(their targets live in rust-analyzer's own stand-ins), as are crates named like the sysroot's.
"""

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from lsp_process import SimpleLspProcess  # noqa: E402

CHECK = re.compile(r'\n\s*(check|check_unresolved|check_no_definition|check_name)\(\s*(?:"(\w+)",\s*)?r(#+)"(.*?)"\3', re.S)
TEST = re.compile(r"#\[test\]\s*fn (\w+)\(\)")
SYSROOT = {"std", "core", "alloc", "proc_macro", "test"}
ON_WORD = False
RA_BIN = ""


def dedent(text):
    text = text[1:] if text.startswith("\n") else text
    lines = text.split("\n")
    indent = min((len(l) - len(l.lstrip(" ")) for l in lines if l.strip()), default=0)
    return "\n".join(l[indent:] for l in lines)


def extract(tests_file):
    src = Path(tests_file).read_text()
    if tests_file.endswith(".txt"):
        # Our own scenarios: `=== name` then the fixture, up to the next case; `#` lines before the first are notes.
        blocks = re.split(r"^=== (\S+)\s*$", src, flags=re.M)[1:]
        return [{"name": f"{n}#1", "kind": "check", "expected_name": None, "fixture": b.strip("\n") + "\n"} for n, b in zip(blocks[::2], blocks[1::2])]
    tests = [(m.start(), m.group(1)) for m in TEST.finditer(src)]
    cases, per_test = [], {}
    for m in CHECK.finditer(src):
        name = [n for at, n in tests if at < m.start()][-1] if tests else "?"
        per_test[name] = per_test.get(name, 0) + 1
        cases.append({"name": f"{name}#{per_test[name]}", "kind": m.group(1), "expected_name": m.group(2), "fixture": dedent(m.group(4))})
    return cases


def parse_fixture(text):
    """Files of a fixture: [(path, meta, text)], plus the fixture-wide metas (`minicore`, `proc_macros`, ...)."""
    files, globals_, cur = [], {}, None
    for line in text.split("\n"):
        if line.startswith("//- "):
            meta = line[4:].strip()
            if meta.startswith("/"):
                path, *attrs = meta.split()
                cur = [path, dict(a.split(":", 1) if ":" in a else (a, "") for a in attrs), []]
                files.append(cur)
            else:
                k, _, v = meta.partition(":")
                globals_[k.strip()] = v.strip()
            continue
        if cur is None:
            cur = ["/main.rs", {}, []]
            files.append(cur)
        cur[2].append(line)
    return [(p, m, "\n".join(t).rstrip("\n") + "\n") for p, m, t in files], globals_


def annotations(text):
    """rust-analyzer's `extract_annotations`: each `^^^` under `//` marks a range of the nearest line above it long
    enough; `^file` marks the whole file. Returns byte offsets of the starts."""
    res, line_start_map, line_start = [], {}, 0
    for line in text.splitlines(keepends=True):
        if "//" in line:
            prefix, suffix = line.split("//", 1)
            off = len(prefix.encode()) + 2
            length = off
            for mm in re.finditer(r"\^+", suffix.rstrip("\n")):
                rest = suffix[mm.end():]
                if rest.startswith("file"):
                    res.append(("file", 0))
                    continue
                start, end = off + mm.start(), off + mm.end()
                cands = [(ln, st) for ln, st in line_start_map.items() if ln >= end]
                if cands:
                    res.append(("range", min(cands)[1] + start))
        else:
            length = len(line.encode())
        line_start_map = {ln: st for ln, st in line_start_map.items() if ln > length}
        line_start_map[length] = line_start
        line_start += len(line.encode())
    return res


def skip_reason(case, files, globals_):
    if "minicore" in globals_:
        return "minicore"
    if "proc_macros" in globals_ or any("proc-macro" in t or "proc_macro" in t for _, _, t in files):
        return "proc-macro"
    if any("cfg" in m or "env" in m or "crate-attr" in m for _, m, _ in files):
        return "cfg/env"
    crates = {m["crate"].split(":")[-1].split("@")[0] for _, m, _ in files if "crate" in m}
    if crates & SYSROOT:
        return "sysroot-named crate"
    if sum(t.count("$0") for _, _, t in files) != 1:
        return "cursor range"
    return None


def materialize(case, root):
    """Writes the fixture as a Cargo workspace under `root`; returns (cursor file, byte offset, expected targets)."""
    files, globals_ = parse_fixture(case["fixture"])
    crates = []  # (name, root file, deps, edition)
    for path, meta, _ in files:
        if "crate" in meta:
            name = meta["crate"].split(":")[-1].split("@")[0].replace("-", "_")
            # `deps:alias=crate` depends on `crate` under the name `alias`.
            deps = [tuple(d.replace("-", "_").split("=", 1)) if "=" in d else (d.replace("-", "_"),) * 2 for d in meta.get("deps", "").split(",") if d]
            crates.append((name, path, deps, meta.get("edition", "2021")))
    if not crates:
        crates = [("main", files[0][0], [], files[0][1].get("edition", "2021"))]
    vfs = root / "vfs"
    cursor, expected = None, []
    for path, _, text in files:
        if "$0" in text:
            cursor = (path, len(text[: text.index("$0")].encode()))
            text = text.replace("$0", "", 1)
        f = vfs / path.lstrip("/")
        f.parent.mkdir(parents=True, exist_ok=True)
        f.write_text(text)
        expected += [(path, off) for kind, off in annotations(text)]
    # Each package sits in the directory of its crate's root file, as in a real project (ridx tells a file's crate by
    # the manifest directory above it); a second crate rooted in the same directory goes to `pkgs/<name>`.
    pkg_dirs, taken = {}, set()
    for name, croot, _, _ in crates:
        d = (vfs / croot.lstrip("/")).parent
        pkg_dirs[name] = d if d not in taken else root / "pkgs" / name
        taken.add(pkg_dirs[name])
    members = []
    for name, croot, deps, edition in crates:
        pkg = pkg_dirs[name]
        pkg.mkdir(parents=True, exist_ok=True)
        dep_lines = "".join(f'{alias} = {{ package = "fx_{d}", path = "{os.path.relpath(pkg_dirs[d], pkg)}" }}\n' for alias, d in deps if d not in SYSROOT and d in pkg_dirs)
        lib = os.path.relpath(vfs / croot.lstrip("/"), pkg)
        (pkg / "Cargo.toml").write_text(
            f'[package]\nname = "fx_{name}"\nversion = "0.1.0"\nedition = "{edition}"\nautobins = false\nautoexamples = false\nautotests = false\nautobenches = false\n\n'
            f'[lib]\nname = "{name}"\npath = "{lib}"\n\n[dependencies]\n{dep_lines}'
        )
        members.append(f'"{os.path.relpath(pkg, root)}"')
    (root / "Cargo.toml").write_text(f'[workspace]\nresolver = "2"\nmembers = [{", ".join(members)}]\n')
    return cursor, expected


def offset_to_pos(text, off):
    before = text.encode()[:off].decode(errors="ignore")
    line = before.count("\n")
    col = len(before.rsplit("\n", 1)[-1].encode("utf-16-le")) // 2
    return line, col


def run_case(case, ridx, work, driver):
    files, globals_ = parse_fixture(case["fixture"])
    reason = skip_reason(case, files, globals_)
    if reason:
        return {"name": case["name"], "status": "skipped", "reason": reason}
    root = work / re.sub(r"\W", "_", case["name"])
    shutil.rmtree(root, ignore_errors=True)
    root.mkdir(parents=True)
    cursor, expected = materialize(case, root)
    t0 = time.time()
    if RA_BIN:
        # rust-analyzer itself, for comparison: it needs no index, only `cargo check` to report errors.
        idx = subprocess.run(["cargo", "check", "--quiet"], cwd=root, capture_output=True, text=True, timeout=300)
        compiled = idx.returncode == 0
    else:
        idx = subprocess.run([ridx, "index", str(root)], cwd=root, capture_output=True, text=True, timeout=300)
        compiled = idx.returncode == 0 and (root / "target/ridx/ridx_manifest.json").is_file()
    index_s = time.time() - t0
    errors = [l for l in idx.stderr.splitlines() if l.startswith("error")][:3]
    vfs = root / "vfs"
    cpath = vfs / cursor[0].lstrip("/")
    ctext = cpath.read_text()
    off = cursor[1]
    word = lambda b: chr(b).isalnum() or b == b"_"[0]  # noqa: E731
    raw = ctext.encode()
    # `--on-word`: a cursor just past an identifier (`foo.spam$0;`) put on its last character, to tell a miss of the
    # navigation from rust-analyzer's taking the token left of a caret.
    if ON_WORD and off > 0 and word(raw[off - 1]) and (off == len(raw) or not word(raw[off])):
        off -= 1
    line, col = offset_to_pos(ctext, off)
    lsp = SimpleLspProcess([RA_BIN], cwd=str(root), is_ra=True) if RA_BIN else SimpleLspProcess([ridx, "lsp"], cwd=str(root))
    try:
        lsp.notify("textDocument/didOpen", {"textDocument": {"uri": cpath.as_uri(), "languageId": "rust", "version": 0, "text": ctext}})
        res, _, err = lsp.request("textDocument/definition", {"textDocument": {"uri": cpath.as_uri()}, "position": {"line": line, "character": col}}, timeout=10)
    finally:
        lsp.close()
    if err:
        return {"name": case["name"], "status": "error", "reason": f"definition: {err}", "compiled": compiled}
    got = res if isinstance(res, list) else ([res] if res else [])
    got_set = set()
    for loc in got:
        uri = loc.get("targetUri") or loc.get("uri", "")
        rng = loc.get("targetSelectionRange") or loc.get("range") or {}
        p = Path(uri.removeprefix("file://"))
        try:
            rel = "/" + str(p.resolve().relative_to(vfs.resolve()))
        except ValueError:
            rel = str(p)
        got_set.add((rel, rng.get("start", {}).get("line"), rng.get("start", {}).get("character")))
    want_set = set()
    for path, off in expected:
        t = (vfs / path.lstrip("/")).read_text()
        want_set.add((path, *offset_to_pos(t, off)))
    if case["kind"] in ("check_unresolved", "check_no_definition"):
        ok = not got_set
    elif case["kind"] == "check_name":
        ok = len(got_set) == 1
    else:
        ok = got_set == want_set
    return {
        "name": case["name"], "kind": case["kind"], "status": "pass" if ok else "fail", "compiled": compiled,
        "index_s": round(index_s, 1), "errors": errors, "cursor": [cursor[0], line, col],
        "expected": sorted(map(list, want_set)), "got": sorted(map(list, got_set)),
    }


def main():
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)
    ex = sub.add_parser("extract")
    ex.add_argument("--tests", required=True)
    ex.add_argument("--out", required=True)
    rn = sub.add_parser("run")
    rn.add_argument("--cases", required=True)
    rn.add_argument("--ridx", required=True)
    rn.add_argument("--driver", default=os.environ.get("RIDX_DRIVER_BIN", ""))
    rn.add_argument("--work", required=True)
    rn.add_argument("--jobs", type=int, default=6)
    rn.add_argument("--only", default="")
    rn.add_argument("--on-word", action="store_true", help="put a cursor just past an identifier on it")
    rn.add_argument("--ra", default="", help="ask this rust-analyzer binary instead of ridx")
    rn.add_argument("--out", required=True)
    a = ap.parse_args()
    if a.cmd == "extract":
        cases = extract(a.tests)
        Path(a.out).write_text(json.dumps(cases, indent=1))
        print(f"{len(cases)} cases")
        return
    global ON_WORD, RA_BIN
    ON_WORD, RA_BIN = a.on_word, a.ra
    cases = json.loads(Path(a.cases).read_text())
    if a.only:
        keep = set(a.only.split(","))
        cases = [c for c in cases if c["name"] in keep or c["name"].split("#")[0] in keep]
    # Every `ridx index` and LSP session inherits these: the driver to compile with, and no indexing of its own.
    os.environ.update(RIDX_DRIVER_BIN=a.driver, RIDX_REINDEX_ON_SAVE="0")
    work = Path(a.work).resolve()
    work.mkdir(parents=True, exist_ok=True)

    def one(c):
        try:
            return run_case(c, a.ridx, work, a.driver)
        except Exception as e:  # noqa: BLE001 - a broken case is a result, not the end of the run
            return {"name": c["name"], "status": "error", "reason": repr(e)[:300]}

    with ThreadPoolExecutor(a.jobs) as pool:
        results = list(pool.map(one, cases))
    Path(a.out).write_text(json.dumps(results, indent=1))
    by = {}
    for r in results:
        key = r["status"] + (f" ({r['reason']})" if r["status"] == "skipped" else "")
        by[key] = by.get(key, 0) + 1
    print(json.dumps(by, indent=1))
    fails = [r for r in results if r["status"] == "fail"]
    print(f"failing, compiled: {sum(r['compiled'] for r in fails)}, not compiled: {sum(not r['compiled'] for r in fails)}")


if __name__ == "__main__":
    main()
