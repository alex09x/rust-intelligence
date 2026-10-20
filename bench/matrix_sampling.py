# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""
Comprehensive scenario sampler for 100% LSP 3.17 benchmark matrix.
Generates 300 distinct test varieties per method across all 78 LSP methods.
"""

from pathlib import Path
import random
try:
    from scan import scan_source_positions
    from matrix_categories import LSP_SPECS
except ImportError:
    from bench.scan import scan_source_positions
    from bench.matrix_categories import LSP_SPECS


def _sample(pool, n):
    if not pool:
        return []
    if len(pool) >= n:
        return random.sample(pool, n)
    return random.choices(pool, k=n)


def sample_full_lsp_matrix(repo_dir, per_method=300, seed=42):
    """
    Builds the complete benchmark test matrix: 78 LSP 3.17 methods, the four lifecycle
    methods once and the other 74 with per_method sampled items each (22,204 requests
    per server at per_method=300).
    """
    random.seed(seed)
    usages, declarations = scan_source_positions(repo_dir)
    scenarios = {}

    files = sorted({u[0] for u in usages} | {d[0] for d in declarations})
    if not files:
        files = sorted(str(p) for p in Path(repo_dir).glob("src/**/*.rs"))

    decl_names = sorted({d[3] for d in declarations})
    type_decls = [d for d in declarations if d[4] in ("struct", "enum", "trait")] or declarations
    fn_decls = [d for d in declarations if d[4] == "fn"] or declarations
    call_usages = [u for u in usages if u[4] == "method_call"] or usages

    half = per_method // 2

    # Loop over all 78 defined specs
    for mid, key, rpc_method, domain, rpc_type in LSP_SPECS:
        items = []

        if key in ("definition", "type_definition", "document_highlight", "moniker",
                   "selection_range", "inline_value", "inline_completion"):
            pool = _sample(usages, per_method)
            items = [{"name": f"{key}_{i}_{n}", "location": [f, l, c], "target": n, "kind": k, "file": f}
                     for i, (f, l, c, n, k) in enumerate(pool)]

        elif key == "declaration":
            pool = _sample(declarations, per_method)
            items = [{"name": f"decl_{i}_{n}", "location": [f, l, c], "target": n, "kind": k, "file": f}
                     for i, (f, l, c, n, k) in enumerate(pool)]

        elif key == "implementation":
            pool = _sample(type_decls, per_method)
            items = [{"name": f"impl_{i}_{n}", "location": [f, l, c], "target": n, "kind": k, "file": f}
                     for i, (f, l, c, n, k) in enumerate(pool)]

        elif key == "references":
            u_pool = _sample(usages, half)
            d_pool = _sample(declarations, per_method - half)
            items = [{"name": f"refs_{i}_{n}", "location": [f, l, c], "target": n, "kind": k, "file": f}
                     for i, (f, l, c, n, k) in enumerate(u_pool + d_pool)]

        elif key == "hover":
            h_u = int(per_method * 0.7)
            pool = _sample(usages, h_u) + _sample(declarations, per_method - h_u)
            items = [{"name": f"hover_{i}_{n}", "location": [f, l, c], "target": n, "kind": k, "file": f}
                     for i, (f, l, c, n, k) in enumerate(pool)]

        elif key in ("completion", "signature_help", "on_type_formatting"):
            pool = _sample(call_usages, per_method)
            items = [{"name": f"{key}_{i}_{n}", "location": [f, l, c], "target": n, "kind": k, "file": f}
                     for i, (f, l, c, n, k) in enumerate(pool)]

        elif key in ("document_symbol", "folding_range", "code_lens", "document_diagnostic",
                     "document_link", "document_color", "color_presentation", "text_document_content",
                     "formatting", "range_formatting", "inlay_hint"):
            pool = _sample(files, per_method)
            items = [{"name": f"{key}_{i}_{Path(f).name}", "location": [f, 1, 1], "file": f}
                     for i, f in enumerate(pool)]

        elif key == "workspace_symbol":
            for i in range(per_method):
                if decl_names:
                    bname = random.choice(decl_names)
                    m = i % 5
                    q = bname if m == 0 else (bname[:max(2, len(bname)//2)] if m == 1 else (bname[1:-1] if m == 3 else f"nonexistent_{i}"))
                else:
                    q = f"query_{i}"
                items.append({"name": f"ws_sym_{i}_{q}", "query": q})

        elif key in ("prepare_call_hierarchy", "incoming_calls", "outgoing_calls"):
            pool = _sample(fn_decls, per_method)
            items = [{"name": f"{key}_{i}_{n}", "location": [f, l, c], "target": n, "file": f}
                     for i, (f, l, c, n, k) in enumerate(pool)]

        elif key in ("prepare_type_hierarchy", "supertypes", "subtypes"):
            pool = _sample(type_decls, per_method)
            items = [{"name": f"{key}_{i}_{n}", "location": [f, l, c], "target": n, "file": f}
                     for i, (f, l, c, n, k) in enumerate(pool)]

        elif key in ("semantic_tokens_full", "semantic_tokens_range", "semantic_tokens_delta"):
            pool = _sample(files, per_method)
            items = [{"name": f"{key}_{i}_{Path(f).name}", "location": [f, 1, 1], "file": f, "prev_id": f"id_{i}"}
                     for i, f in enumerate(pool)]

        elif key in ("prepare_rename", "rename", "linked_editing_range", "code_action"):
            pool = _sample(declarations, per_method)
            items = [{"name": f"{key}_{i}_{n}", "location": [f, l, c], "target": n, "new_name": f"{n}_renamed_{i}", "file": f}
                     for i, (f, l, c, n, k) in enumerate(pool)]

        elif key in ("did_open", "did_change_full", "did_change_inc", "will_save", "will_save_wait", "did_save", "did_close",
                     "did_change_watched_files", "did_create_files", "will_create_files", "did_rename_files", "will_rename_files",
                     "did_delete_files", "will_delete_files"):
            pool = _sample(files, per_method)
            items = [{"name": f"{key}_{i}_{Path(f).name}", "file": f}
                     for i, f in enumerate(pool)]

        elif key == "mutation_case_a":
            # Case A: Cross-file new symbol injection (file_a defines, file_b calls via use crate::)
            for i in range(per_method):
                fa = random.choice(files)
                fb = random.choice([f for f in files if f != fa] or files)
                items.append({
                    "name": f"mutation_a_{i}",
                    "file_a": fa, "file_b": fb,
                    "injected_fn": f"bench_probe_{i}",
                })

        elif key == "mutation_case_b":
            # Case B: Stale overlay detector (rename in file_a, query orig_name in file_b -> expect Not Found)
            decl_sample = _sample(fn_decls, per_method)
            for i, (f, l, c, n, k) in enumerate(decl_sample):
                fb = random.choice([f_other for f_other in files if f_other != f] or files)
                items.append({
                    "name": f"mutation_b_{i}_{n}",
                    "file_a": f, "file_b": fb,
                    "orig_name": n,
                    "stale_renamed": f"{n}_renamed_stale_{i}",
                    "location": [f, l, c],
                })

        elif key == "mutation_case_c":
            # Case C: Line displacement (+10 lines in file_a, verify targetSelectionRange shifted)
            decl_sample = _sample(declarations, per_method)
            for i, (f, l, c, n, k) in enumerate(decl_sample):
                items.append({
                    "name": f"mutation_c_{i}_{n}",
                    "file_a": f,
                    "target": n,
                    "orig_line": l,
                    "displacement": 10,
                    "location": [f, l, c],
                })

        elif key == "cancel_request":
            # String id guaranteed never to collide with numeric client next_id (int >= 1)
            items = [{"name": f"{key}_{i}", "iteration": i, "req_id": f"bench-cancel-{i}"} for i in range(per_method)]
        else:
            # Lifecycle, resolve, or workspace ops
            items = [{"name": f"{key}_{i}", "iteration": i} for i in range(per_method)]

        scenarios[key] = items

    return scenarios
