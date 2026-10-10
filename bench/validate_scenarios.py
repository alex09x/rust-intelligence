# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Pre-run scenario validation: ensures all benchmark scenarios match source code context."""

import argparse
import json
import sys
from pathlib import Path


def validate_scenarios_for_repo(
    repo_dir: Path, scenarios: list, repo_name: str, excluded: set = None
) -> tuple[int, int, list[str]]:
    """Validates that scenarios strictly match source code context.

    Returns (valid_count, total_evaluated, errors).
    """
    if excluded is None:
        excluded = set()
    errors = []
    repo_dir = Path(repo_dir).resolve()
    valid_count = 0
    total_evaluated = 0

    for sc in scenarios:
        name = sc.get("name", "unknown")
        if name in excluded:
            continue
        total_evaluated += 1
        location = sc.get("location", [])
        if len(location) != 3:
            errors.append(f"[{repo_name}] {name}: invalid location schema {location}")
            continue

        rel_path, line, col = location
        full_path = repo_dir / rel_path
        if not full_path.is_file():
            errors.append(f"[{repo_name}] {name}: file does not exist: {rel_path}")
            continue

        try:
            content = full_path.read_text(encoding="utf-8", errors="replace")
        except OSError as e:
            errors.append(f"[{repo_name}] {name}: failed to read {rel_path}: {e}")
            continue

        lines = content.splitlines()
        if line <= 0 or line > len(lines):
            errors.append(f"[{repo_name}] {name}: line {line} out of bounds ({len(lines)} lines)")
            continue

        line_str = lines[line - 1]
        kind = sc.get("kind", "")
        target = sc.get("target", "")

        idx = col - 1
        if idx < 0 or idx > len(line_str):
            errors.append(f"[{repo_name}] {name}: col {col} out of bounds (line len {len(line_str)})")
            continue

        prefix = line_str[:idx]
        suffix = line_str[idx:]

        if not suffix.startswith(target):
            errors.append(
                f"[{repo_name}] {name}: target mismatch at {line}:{col}: "
                f"expected {target!r}, got {suffix[:30]!r}"
            )
            continue
        if kind == "dot" and not prefix.endswith("."):
            errors.append(
                f"[{repo_name}] {name}: dot mismatch at {line}:{col}: "
                f"prefix {prefix[-10:]!r} does not end with '.'"
            )
            continue
        if kind == "colon" and not prefix.endswith("::"):
            errors.append(
                f"[{repo_name}] {name}: colon mismatch at {line}:{col}: "
                f"prefix {prefix[-10:]!r} does not end with '::'"
            )
            continue

        valid_count += 1

    return (valid_count, total_evaluated, errors)


def main():
    parser = argparse.ArgumentParser(description="Validate benchmark scenarios against repo checkout")
    parser.add_argument("--repo-dir", required=True, help="Path to repository checkout")
    parser.add_argument("--scenarios-jsonl", required=True, help="Path to jsonl file containing scenarios")
    parser.add_argument("--repo-slug", required=True, help="Repo slug (e.g. serde-rs/serde or bevyengine/bevy)")
    parser.add_argument("--excluded", nargs="*", default=[], help="Scenario names to exclude from validation")
    args = parser.parse_args()

    repo_dir = Path(args.repo_dir)
    jsonl_path = Path(args.scenarios_jsonl)

    if not repo_dir.is_dir():
        print(f"Error: repo directory not found: {repo_dir}", file=sys.stderr)
        sys.exit(1)

    if not jsonl_path.is_file():
        print(f"Error: scenarios jsonl not found: {jsonl_path}", file=sys.stderr)
        sys.exit(1)

    target_scenarios = None
    with open(jsonl_path, "r", encoding="utf-8") as f:
        for line in f:
            if line.strip():
                d = json.loads(line)
                if d.get("repo") == args.repo_slug:
                    target_scenarios = d.get("scenarios", [])
                    break

    if target_scenarios is None:
        print(f"Error: no scenarios found for repo slug {args.repo_slug}", file=sys.stderr)
        sys.exit(1)

    excluded_set = set(args.excluded)
    valid_cnt, total_eval, errors = validate_scenarios_for_repo(
        repo_dir, target_scenarios, args.repo_slug, excluded=excluded_set
    )
    if errors:
        print(f"Validation FAILED: {valid_cnt}/{total_eval} valid, {len(errors)} error(s):", file=sys.stderr)
        for err in errors[:20]:
            print(f"  {err}", file=sys.stderr)
        if len(errors) > 20:
            print(f"  ... and {len(errors) - 20} more errors", file=sys.stderr)
        sys.exit(1)

    print(f"Validation PASSED: {valid_cnt}/{total_eval} scenarios verified for {args.repo_slug}")
    sys.exit(0)


if __name__ == "__main__":
    main()
