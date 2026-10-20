# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0

"""
Shared helpers for publishing benchmark artifacts: canonical repository names,
host-private path sanitization and output verification.
"""

import hashlib
import re
from pathlib import Path
from typing import Any, Dict, List, Tuple

CANONICAL_REPOS = {
    "serde": "serde-rs/serde", "ripgrep": "BurntSushi/ripgrep",
    "tokio": "tokio-rs/tokio", "bevy": "bevyengine/bevy",
    "rust-analyzer": "rust-lang/rust-analyzer",
}

FORBIDDEN_DATA_PATTERNS = [
    re.compile(r"/home/[^/]+/"), re.compile(r"/Users/[^/]+/"), re.compile(r"/tmp/[^/]+/"),
]

FORBIDDEN_SIDECAR_PATTERNS = [
    re.compile(r"/home/[a-zA-Z0-9_.-]+/"), re.compile(r"/Users/[a-zA-Z0-9_.-]+/"),
    re.compile(r"/tmp/[a-zA-Z0-9_.-]+/"),
]


def compute_sha256(file_path: Path) -> str:
    """Computes SHA-256 hex digest of a file."""
    h = hashlib.sha256()
    with open(file_path, "rb") as f:
        while chunk := f.read(65536):
            h.update(chunk)
    return h.hexdigest()


def sanitize_text(text: str) -> Tuple[str, List[Dict[str, Any]]]:
    """Sanitizes paths according to canonical mapping and privacy requirements.

    Returns sanitized text and list of applied sanitization rules with replacement counts.
    """
    applied_rules: List[Dict[str, Any]] = []

    for repo_short, canonical in CANONICAL_REPOS.items():
        pattern = rf"/tmp/[^/]+/{re.escape(repo_short)}/"
        count = len(re.findall(pattern, text))
        if count > 0:
            text = re.sub(pattern, f"{canonical}/", text)
            applied_rules.append({"rule": f"/tmp/<build-dir>/{repo_short}/ -> {canonical}/", "replacements": count})

    gen_pattern = r"/tmp/[^/]+/([^/]+)/"
    count = len(re.findall(gen_pattern, text))
    if count > 0:
        text = re.sub(gen_pattern, r"\1/", text)
        applied_rules.append({"rule": "/tmp/<build-dir>/<repo>/ -> <repo>/", "replacements": count})

    for pat, desc, repl in [
        (r"/home/[^/]+/\.cargo/registry/", "/home/<user>/.cargo/registry/ -> <cargo-registry>/", "<cargo-registry>/"),
        (r"/home/[^/]+/\.rustup/toolchains/[^/]+/lib/rustlib/", "/home/<user>/.rustup/toolchains/<toolchain>/lib/rustlib/ -> <rustlib>/", "<rustlib>/"),
        (r"/home/[^/]+/", "/home/<user>/ -> <user-home>/", "<user-home>/"),
        (r"/Users/[^/]+/", "/Users/<user>/ -> <user-home>/", "<user-home>/"),
        (r"/tmp/", "/tmp/ -> <tmp>/", "<tmp>/"),
    ]:
        count = len(re.findall(pat, text))
        if count > 0:
            text = re.sub(pat, repl, text)
            applied_rules.append({"rule": desc, "replacements": count})

    return text, applied_rules


def verify_clean_output(content: str, filename: str) -> None:
    """Verifies that no host-private patterns appear in published text."""
    patterns = FORBIDDEN_SIDECAR_PATTERNS if filename.endswith(".provenance.json") else FORBIDDEN_DATA_PATTERNS
    for pat in patterns:
        matches = pat.findall(content)
        if matches:
            raise RuntimeError(
                f"Privacy check failed for {filename}: found {len(matches)} occurrences matching '{pat.pattern}'"
            )
