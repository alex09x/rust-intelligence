# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Which code an action compile check really compiles.

`mitts check --workspace --all-targets` fails at a corpus' HEAD when one target cannot build in this environment
(a nightly-only bench, an example without `main`, a dev-dependency built against a second copy of the crate). The
check then has to leave something out; whatever it leaves out, an edit there "compiles" without being compiled.
`select_mode` finds the widest check that passes at HEAD, excluding failing workspace packages first and dropping
target kinds only when that is not enough; `edit_is_checked` tells whether an edited file is compiled by it.
"""

import json
import os
import re
import subprocess
from typing import Callable, Dict, List, Optional, Set, Tuple

# Target selections from the widest down, with the target kinds each compiles.
CANDIDATES: List[Tuple[List[str], Set[str]]] = [
    (["--all-targets"], {"lib", "bin", "test", "example", "bench"}),
    (["--lib", "--bins", "--tests", "--examples"], {"lib", "bin", "test", "example"}),
    (["--lib", "--bins", "--tests", "--benches"], {"lib", "bin", "test", "bench"}),
    (["--lib", "--bins", "--tests"], {"lib", "bin", "test"}),
    ([], {"lib", "bin"}),
]

_FAILED = re.compile(r"Failed to compile (\S+) v\S+")
_FAILED_KIND = re.compile(r"Failed to compile \S+ v\S+ \[(\w+):(\w[\w-]*)\]")


def workspace_members(wt: str) -> Dict[str, str]:
    """Workspace member manifest directories (real paths) -> package names, from `cargo metadata --no-deps`."""
    r = subprocess.run(["cargo", "metadata", "--no-deps", "--format-version", "1", "--offline"], cwd=wt, capture_output=True, text=True)
    if r.returncode != 0:
        return {}
    meta = json.loads(r.stdout)
    members = set(meta.get("workspace_members") or [])
    return {os.path.realpath(os.path.dirname(p["manifest_path"])): p["name"] for p in meta.get("packages", []) if p.get("id") in members}


def failing_packages(log: str) -> Set[str]:
    return set(_FAILED.findall(log))


def select_mode(run: Callable[[List[str]], Tuple[bool, str]], members: Dict[str, str]) -> Tuple[List[str], Set[str], Set[str]]:
    """The widest `(args, excluded packages, target kinds)` whose check passes at HEAD. `run(args)` checks with
    `args` and returns `(ok, log)`. A failing workspace member is excluded; a failing dependency is traced to the
    members whose own check fails, when there are few enough to probe one by one."""
    names = set(members.values())
    for args, kinds in CANDIDATES:
        excluded: Set[str] = set()
        for _ in range(4):
            ok, log = run(args + [a for p in sorted(excluded) for a in ("--exclude", p)])
            if ok:
                return args, excluded, kinds
            # A bench or example that cannot build (a nightly `#![feature]`, an example without `main`) drops that
            # target kind, keeping its package's libraries and tests checked, before any package is excluded.
            failed_kinds = {"bench" if k == "bench" or t == "bench" else k for k, t in _FAILED_KIND.findall(log)}
            if failed_kinds and failed_kinds <= {"bench", "example"} and failed_kinds & kinds:
                break
            culprits = failing_packages(log) & names
            if not culprits and len(names) <= 16:
                culprits = {n for n in sorted(names - excluded) if not run(args + ["-p", n])[0]}
            if not culprits or culprits <= excluded:
                break
            excluded |= culprits
    return [], set(), {"lib", "bin"}


def _target_kind(rel: str) -> str:
    if rel.startswith("tests/"):
        return "test"
    if rel.startswith("examples/"):
        return "example"
    if rel.startswith("benches/"):
        return "bench"
    if rel.startswith("src/bin/") or rel == "src/main.rs":
        return "bin"
    return "lib"


def edit_is_checked(path: str, first_line: int, members: Dict[str, str], excluded: Set[str], kinds: Set[str]) -> bool:
    """Whether the check selected compiles the code at line `first_line` (0-based) of `path`: its package is a
    workspace member not excluded, its target kind is checked, and without test targets it lies before the file's
    first `#[cfg(test)]`."""
    d = os.path.dirname(os.path.realpath(path))
    pkg_dir: Optional[str] = None
    while d and d != "/":
        if d in members:
            pkg_dir = d
            break
        if os.path.isfile(os.path.join(d, "Cargo.toml")):
            return False  # a package outside the workspace (`bevy/benches`)
        d = os.path.dirname(d)
    if pkg_dir is None or members[pkg_dir] in excluded:
        return False
    kind = _target_kind(os.path.relpath(os.path.realpath(path), pkg_dir))
    if kind not in kinds:
        return False
    if "test" not in kinds:
        try:
            with open(path, encoding="utf-8", errors="replace") as fh:
                lines = fh.read().split("\n")
        except OSError:
            return False
        cfg_test = next((i for i, ln in enumerate(lines) if "#[cfg(test)]" in ln), None)
        if cfg_test is not None and first_line >= cfg_test:
            return False
    return True
