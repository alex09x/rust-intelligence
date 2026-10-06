#!/usr/bin/env python3
"""
rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
Copyright (c) 2026 Alexander Panasenko

Contact: alex@prod.codes
Author: https://prod.codes/about/
Project: https://github.com/alex09x/rust-intelligence
SPDX-License-Identifier: MIT OR Apache-2.0

Scenario sampling and call-site cursor resolution for symmetric LSP benchmarks.
"""

import json
import random
from pathlib import Path
from corpus import run_cmd

SUPPORTED_KINDS = [
    "Function",
    "Struct",
    "Enum",
    "Trait",
    "Method",
    "AssocFn",
    "TypeAlias",
]


def resolve_identifier_col(file_path, line_1based, def_name, fallback_col=1):
    try:
        with open(file_path, "r", encoding="utf-8", errors="replace") as f:
            for idx, text in enumerate(f, 1):
                if idx == line_1based:
                    pos = text.find(def_name)
                    if pos >= 0:
                        return pos + 1
                    break
    except Exception:
        pass
    return fallback_col


def sample_scenarios(repo_dir, ridx_bin, count=30, seed=42):
    random.seed(seed)
    all_defs = []
    seen_defs = set()
    target_ridx = repo_dir / "target" / "ridx"

    for q in ["a", "e", "i", "s", "t", "r"]:
        code, out, _, _ = run_cmd(
            [str(ridx_bin), "symbols", "--json", str(target_ridx), q],
            cwd=repo_dir,
        )
        if code != 0 or not out:
            continue
        try:
            data = json.loads(out)
            for s in data.get("symbols", []):
                f = s.get("file", "")
                k = s.get("kind", "")
                if not f.startswith(str(repo_dir)):
                    continue
                if k not in SUPPORTED_KINDS:
                    continue
                loc_key = (f, s.get("start_line", 1), s.get("start_col", 1))
                if loc_key not in seen_defs:
                    seen_defs.add(loc_key)
                    all_defs.append(s)
        except Exception:
            continue

    if not all_defs:
        return []

    grouped = {}
    for d in all_defs:
        k = d.get("kind", "Other")
        grouped.setdefault(k, []).append(d)

    sampled_defs = []
    per_group = max(1, count // max(1, len(grouped)))
    for k, items in grouped.items():
        sampled_defs.extend(random.sample(items, min(len(items), per_group)))

    if len(sampled_defs) < count:
        remaining = [d for d in all_defs if d not in sampled_defs]
        needed = count - len(sampled_defs)
        sampled_defs.extend(random.sample(remaining, min(len(remaining), needed)))

    scenarios = []
    for idx, d in enumerate(sampled_defs):
        name = d.get("name", "")
        f = d.get("file", "")
        line = d.get("start_line", 1)
        col = d.get("start_col", 1)

        # Attempt to find authentic first-party reference call-site
        call_loc = None
        code, out, _, _ = run_cmd(
            [str(ridx_bin), "refs", "--json", str(target_ridx), f"{f}:{line}:{col}"],
            cwd=repo_dir,
        )
        if code == 0 and out:
            try:
                ref_data = json.loads(out)
                refs = ref_data.get("references", [])
                fp_refs = [r for r in refs if r.get("file", "").startswith(str(repo_dir))]
                if fp_refs:
                    chosen = fp_refs[0]
                    call_loc = [
                        chosen.get("file", f),
                        chosen.get("start_line", line),
                        chosen.get("start_col", col),
                    ]
            except Exception:
                pass

        if not call_loc:
            exact_col = resolve_identifier_col(f, line, name, fallback_col=col)
            call_loc = [f, line, exact_col]

        for cap in ["def", "hover", "refs"]:
            scenarios.append({
                "name": f"{cap}_{idx}_{name}",
                "capability": cap,
                "target": name,
                "location": call_loc,
            })

    return scenarios
