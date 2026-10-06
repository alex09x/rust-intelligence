#!/usr/bin/env python3
"""
rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
Copyright (c) 2026 Alexander Panasenko

Contact: alex@prod.codes
Author: https://prod.codes/about/
Project: https://github.com/alex09x/rust-intelligence
SPDX-License-Identifier: MIT OR Apache-2.0

Corpus loading and repository setup utilities for benchmarking.
"""

import json
import os
import subprocess
import time
from pathlib import Path

DEFAULT_PILOT_REPOS = [
    {"repo": "tokio-rs/tokio", "commit": "b2636752450484955e7ad334bac678424d51bc4a"},
    {"repo": "BurntSushi/ripgrep", "commit": "3fce3b5bb0236da2df6d99672afb8a719642eca7"},
    {"repo": "serde-rs/serde", "commit": "6693a89cca77e0151437da1c7f890090b9ebf04c"},
    {"repo": "bevyengine/bevy", "commit": "68c80f3b19ddf9e4ae1ce7dd8b5a68273204c2e8"},
    {"repo": "rust-lang/rust-analyzer", "commit": "021bfc15e24909389bd208d1622b7f4f3942b91a"},
]


def run_cmd(cmd, cwd=None, timeout=600):
    t0 = time.perf_counter()
    try:
        proc = subprocess.run(
            cmd,
            cwd=cwd,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            timeout=timeout,
        )
        elapsed = time.perf_counter() - t0
        return proc.returncode, proc.stdout, proc.stderr, elapsed
    except subprocess.TimeoutExpired:
        return -1, "", "Timeout expired", time.perf_counter() - t0
    except Exception as e:
        return -2, "", str(e), time.perf_counter() - t0


def load_corpus(corpus_path):
    p = Path(corpus_path)
    if not p.exists():
        p = p.with_suffix(".json") if p.with_suffix(".json").exists() else p.with_suffix(".tsv")
    if p.exists() and str(p).endswith(".json"):
        with open(p, "r", encoding="utf-8") as f:
            return json.load(f)
    if p.exists():
        with open(p, "r", encoding="utf-8") as f:
            lines = f.read().splitlines()
        return [
            {"repo": pt[0], "stars": int(pt[1]), "size_kb": int(pt[2]), "default_branch": pt[3], "commit": pt[4]}
            for line in lines[1:] if len(pt := line.split("\t")) >= 5
        ]
    return DEFAULT_PILOT_REPOS


def setup_repo(repo_entry, base_dir):
    repo_slug = repo_entry["repo"]
    commit_sha = repo_entry["commit"]
    repo_name = repo_slug.split("/")[-1]
    repo_dir = Path(base_dir) / repo_name

    if not repo_dir.exists():
        url = f"https://github.com/{repo_slug}.git"
        code, _, err, _ = run_cmd(["git", "clone", "--filter=blob:none", url, str(repo_dir)])
        if code != 0:
            return None, f"git clone failed: {err}"

    code, _, err, _ = run_cmd(["git", "fetch", "--all"], cwd=repo_dir)
    code, _, err, _ = run_cmd(["git", "checkout", commit_sha], cwd=repo_dir)
    if code != 0:
        return None, f"git checkout failed: {err}"

    return repo_dir, None
