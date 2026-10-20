# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Checks by content every differing row of a `position_ab.py` report and writes the classes
sidecar next to the published copy.

    position_ab_content.py --repo DIR --report position_ab_<a>_vs_<b>_<node>_<corpus>.json \\
        --before BIN --after BIN [--file-table FILE_TABLE.tsv ...] \\
        --out position_ab_<a>_vs_<b>_<node>_<corpus>.classes.json

The report names three servers: before, after and a second before process. A row is noise when
the two before processes disagree; it is recorded with the noise flag and no class. Every other
differing row is asked again from both binaries and checked by content:
  i   semantic tokens: the full token set of the file decides for full, range and delta rows.
      Every token the after set lacks must land past the end of the live file, past the end of
      its line, on a span that is not an identifier or path, on a comment line or a keyword, or
      be an identifier that the after binary emits for a dependency file sharing the path tail.
      Every token the after set adds must overlap a removed token on the same line, which the
      before overlap filter preferred because it was shorter.
  ii  call hierarchy: every removed item must point outside the workspace or into a file under
      no indexed target; nothing may be added.
  iv  file under no indexed target: the after answer must be empty.
  v   root package lib.rs: a position in crates/*/src/lib.rs whose before answer named the root
      package's src/lib.rs, the workspace-relative stored path that the old suffix match hit.
  vi  prepareRename or rename: the before answer was the "Cannot rename external symbol" error
      naming a symbol other than the one under the cursor (the symbol at the same coordinates of
      a file sharing the path tail); the after answer names the symbol under the cursor.
With `--file-table` (the `FILE` lines printed by the ridx-format `file_table` example for the
workspace and dependency shards) two more classes are decided from the stored paths first:
  iv' file not stored: no shard stores the live path in any form; the after answer must be empty,
      and the shards whose stored path is a component suffix of the live path are listed.
  viii stored with `..`: the live path is stored only with `..` components (a relative `include!`);
      the before answer must be empty or an error and the after answer non-empty.
A row failing its check is `unclassified`. Paths in the evidence are relative to the workspace,
the cargo registry, a git checkout or the sysroot source tree; no private prefix is kept.
"""

import argparse
import glob
import json
import os
import subprocess
import sys
from collections import Counter
from pathlib import Path

bench_dir = Path(__file__).resolve().parent
if str(bench_dir) not in sys.path:
    sys.path.insert(0, str(bench_dir))

from lsp_process import SimpleLspProcess  # noqa: E402
from matrix_categories import LSP_SPECS, build_params  # noqa: E402
from position_ab_classify import load_target_entries, scenario_from_record  # noqa: E402
from position_ab_content_rules import (  # noqa: E402
    CALL_KEYS, CLASS_NONE, CLASS_NOT_STORED, CLASS_PARENT_DIR, CLASS_RENAME, CLASS_ROOT, CLASS_TOKENS, RENAME_KEYS, RPC,
    TOKEN_KEYS, char_span, classify_calls, classify_rename, classify_root_lib, classify_tokens, decode_tokens, is_empty,
    scrub_path, stored_forms_of, token_data, uris_of,
)

def candidate_dirs(repo, entries):
    """Package directories whose files could share a path tail with a workspace file: the workspace
    root and the package directories of its targets, the source roots of the index's dependency
    shards (ridx_manifest.json), cargo metadata when it resolves offline, every crate of the
    registry and git checkouts, and the sysroot library crates."""
    dirs = {str(repo)}
    for entry, _ in entries:
        d = os.path.dirname(os.path.join(repo, entry))
        dirs.add(os.path.dirname(d) if os.path.basename(d) == "src" else d)
    try:
        manifest = json.load(open(os.path.join(repo, "target", "ridx", "ridx_manifest.json"), encoding="utf-8"))
        dirs |= {d["source_root"] for d in manifest.get("dependency_shards", []) if d.get("source_root")}
    except (OSError, ValueError):
        pass
    try:
        meta = subprocess.run(["cargo", "metadata", "--format-version", "1", "--offline"], cwd=repo, capture_output=True, text=True, check=True)
        dirs |= {str(Path(p["manifest_path"]).parent) for p in json.loads(meta.stdout)["packages"]}
    except (OSError, ValueError, subprocess.CalledProcessError):
        pass
    home = os.path.expanduser("~")
    dirs |= set(glob.glob(os.path.join(home, ".cargo", "registry", "src", "*", "*")))
    dirs |= set(glob.glob(os.path.join(home, ".cargo", "git", "checkouts", "*", "*")))
    try:
        sysroot = subprocess.run(["rustc", "--print", "sysroot"], capture_output=True, text=True, check=True).stdout.strip()
        dirs |= set(glob.glob(os.path.join(sysroot, "lib", "rustlib", "src", "rust", "library", "*")))
    except (OSError, subprocess.CalledProcessError):
        pass
    return sorted(dirs)


def dependency_roots(repo):
    """Source roots of the index's dependency shards (ridx_manifest.json), whether or not they still
    exist on disk."""
    try:
        manifest = json.load(open(os.path.join(repo, "target", "ridx", "ridx_manifest.json"), encoding="utf-8"))
    except (OSError, ValueError):
        return []
    return sorted({d["source_root"] for d in manifest.get("dependency_shards", []) if d.get("source_root")})


def same_tail_candidates(repo, rel, dirs):
    """Files under `dirs` whose path ends with a component suffix of `rel`, the live file excluded."""
    parts = rel.split("/")
    live = os.path.realpath(os.path.join(repo, rel))
    out = set()
    for d in dirs:
        for k in range(1, len(parts) + 1):
            cand = os.path.join(d, *parts[-k:])
            if os.path.isfile(cand) and os.path.realpath(cand) != live:
                out.add(cand)
    return sorted(out)


def load_file_table(paths):
    """(shard, stored path) pairs from the `FILE` lines of file_table outputs."""
    out = []
    for path in paths:
        for line in Path(path).read_text(encoding="utf-8", errors="replace").splitlines():
            parts = line.split("\t")
            if len(parts) == 3 and parts[0] == "FILE":
                out.append((parts[1], parts[2]))
    return out


class Checker:
    def __init__(self, repo, before_bin, after_bin, file_table=None):
        self.repo = str(Path(repo).resolve())
        self.entries = load_target_entries(self.repo)
        self.file_table = file_table
        self.root_lib_stored = any(stored == "src/lib.rs" for _, stored in (file_table or [])) or os.path.isfile(os.path.join(self.repo, "src", "lib.rs"))
        self.before = SimpleLspProcess([before_bin, "lsp"], self.repo, is_ra=False)
        self.after = SimpleLspProcess([after_bin, "lsp"], self.repo, is_ra=False)
        self.dirs = None
        self.dep_roots = []
        self.tokens_cache = {}
        self.lines_cache = {}

    def close(self):
        self.before.close()
        self.after.close()

    def lines(self, rel):
        if rel not in self.lines_cache:
            try:
                self.lines_cache[rel] = Path(self.repo, rel).read_text(encoding="utf-8", errors="replace").split("\n")
            except OSError:
                self.lines_cache[rel] = []
        return self.lines_cache[rel]

    def full_tokens(self, server, abs_path):
        key = (server is self.after, abs_path)
        if key not in self.tokens_cache:
            res, _, _ = server.request(RPC["semantic_tokens_full"], {"textDocument": {"uri": f"file://{abs_path}"}}, timeout=60.0)
            data = token_data(res)
            self.tokens_cache[key] = decode_tokens(data) if data is not None else None
        return self.tokens_cache[key]

    def token_starts(self, abs_path):
        """(line, character column) of every token of `abs_path` from the after server. The file
        may be gone from disk (a registry crate replaced since indexing): the shard still answers,
        and without its text the UTF-16 columns are taken as character columns."""
        key = ("starts", abs_path)
        if key not in self.tokens_cache:
            toks = self.full_tokens(self.after, abs_path) or []
            try:
                lines = Path(abs_path).read_text(encoding="utf-8", errors="replace").split("\n")
            except OSError:
                lines = None
            starts = set()
            for t in toks:
                if lines is None:
                    starts.add((t[0], t[1]))
                else:
                    span = char_span(lines, t)
                    starts.add((span[0], span[1]) if span else (t[0], t[1]))
            self.tokens_cache[key] = starts
        return self.tokens_cache[key]

    def foreign_lookup(self, rel):
        """Names the same-tail file of a shard whose after tokens start where the removed token starts,
        in character columns: 606ed0e converted a foreign token's columns with the live file's text
        and clamped its end to the live line, so only the start survives the round trip."""
        if self.dirs is None:
            self.dirs = candidate_dirs(self.repo, self.entries)
            self.dep_roots = dependency_roots(self.repo)
        cands = same_tail_candidates(self.repo, rel, self.dirs)
        tail_parts = rel.split("/")
        for root in self.dep_roots:
            for k in range(1, len(tail_parts) + 1):
                cands.append(os.path.join(root, *tail_parts[-k:]))
        cands = sorted(set(cands))
        lines = self.lines(rel)

        def foreign_of(tok):
            span = char_span(lines, tok)
            if span is None:
                return None
            wanted = (span[0], span[1])
            for c in cands:
                if wanted in self.token_starts(c):
                    return scrub_path(c, self.repo)
            return None

        return foreign_of

    def check_row(self, row, before_name, after_name):
        key, rel = row["method"], row["file"]
        rec = {"name": row["name"], "file": rel, "line": row["line"], "character": row["character"], "symbol": row["symbol"]}
        params = build_params(key, scenario_from_record(key, rec, self.repo), self.repo)
        res_b, _, err_b = self.before.request(RPC[key], params, timeout=60.0)
        res_a, _, _ = self.after.request(RPC[key], params, timeout=60.0)
        if self.file_table is not None:
            forms = stored_forms_of(rel, os.path.join(self.repo, rel), self.file_table)
            summary = {k: [{"shard": sh, "stored": st} for sh, st in v[:10]] for k, v in forms.items()}
            summary["same_tail_count"] = len(forms["same_tail"])
            if not forms["plain"] and not forms["parent_dir"]:
                ok = is_empty(res_a)
                return (CLASS_NOT_STORED if ok else "unclassified"), {"stored_forms": summary, "after_empty": ok, "target_kind": row["target_kind"]}
            if not forms["plain"]:
                before_empty = is_empty(res_b) or err_b is not None
                ok = before_empty and not is_empty(res_a)
                return (CLASS_PARENT_DIR if ok else "unclassified"), {"stored_forms": summary, "before_empty_or_error": before_empty, "after_empty": is_empty(res_a)}
        if row["target_kind"] == "none":
            ok = is_empty(res_a)
            cls, ev = CLASS_NONE, {"before_outcome": row["results"][before_name]["outcome"], "after_empty": ok, "after_outcome": row["results"][after_name]["outcome"]}
        elif key in TOKEN_KEYS:
            abs_path = os.path.join(self.repo, rel)
            tb, ta = self.full_tokens(self.before, abs_path), self.full_tokens(self.after, abs_path)
            if tb is None or ta is None:
                ok, cls, ev = False, CLASS_TOKENS, {"error": "no token data from one of the servers"}
            else:
                ok, ev = classify_tokens(self.lines(rel), tb, ta, self.foreign_lookup(rel))
                cls = CLASS_TOKENS
        elif key in CALL_KEYS:
            ub, ua = Counter(uris_of(res_b)), Counter(uris_of(res_a))
            ok, cls, ev = classify_calls(list((ub - ua).elements()), list((ua - ub).elements()), self.repo, self.entries, self.root_lib_stored)
        elif key in RENAME_KEYS:
            ok, ev = classify_rename(row["symbol"], err_b, res_a)
            cls = CLASS_RENAME
        else:
            ok, ev = classify_root_lib(rel, uris_of(res_b), uris_of(res_a), self.repo)
            cls = CLASS_ROOT
        return (cls if ok else "unclassified"), ev


def differing_rows(rep, names):
    b, a, b2 = names[0], names[1], names[2] if len(names) > 2 else None

    def answer(row, name):
        r = row["results"][name]
        return r["outcome"], r["digest"]

    for row in rep["rows"]:
        if answer(row, b) == answer(row, a):
            continue
        yield row, bool(b2) and answer(row, b) != answer(row, b2)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--repo", required=True)
    ap.add_argument("--report", required=True, help="full position_ab report (all rows)")
    ap.add_argument("--before", required=True, help="ridx binary of the first server")
    ap.add_argument("--after", required=True, help="ridx binary of the second server")
    ap.add_argument("--file-table", action="append", default=[], help="FILE lines of the file_table example; repeatable")
    ap.add_argument("--out", required=True)
    args = ap.parse_args(argv)
    rep = json.load(open(args.report, encoding="utf-8"))
    names = [s["name"] for s in rep["servers"]]
    file_table = load_file_table(args.file_table) if args.file_table else None
    checker = Checker(args.repo, args.before, args.after, file_table)
    rows, summary = [], Counter()
    try:
        for row, noise in differing_rows(rep, names):
            entry = {k: row[k] for k in ("method", "file", "line", "character", "symbol", "target_kind")}
            entry["noise_before_vs_before2"] = noise
            if noise:
                entry["class"], entry["evidence"] = None, {}
                summary["noise"] += 1
            else:
                entry["class"], entry["evidence"] = checker.check_row(row, names[0], names[1])
                summary[entry["class"]] += 1
            rows.append(entry)
    finally:
        checker.close()
    out = {
        "schema": "position_ab_classes/1", "repo_dir_name": rep["repo_dir_name"], "report": os.path.basename(args.report),
        "servers": [{k: s[k] for k in ("name", "commit", "dirty", "binary") if k in s} for s in rep["servers"]],
        "file_tables": [os.path.basename(p) for p in args.file_table], "stored_paths": len(file_table or []),
        "rows_differing": len(rows), "summary": dict(sorted(summary.items())), "rows": rows,
    }
    Path(args.out).write_text(json.dumps(out, indent=1, sort_keys=False) + "\n", encoding="utf-8")
    print(f"{rep['repo_dir_name']}: differing {len(rows)}, classes {dict(sorted(summary.items()))}; wrote {args.out}")
    return 1 if summary["unclassified"] else 0


if __name__ == "__main__":
    sys.exit(main())
