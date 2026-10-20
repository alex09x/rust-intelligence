# Benchmark Methodology: RIDX vs rust-analyzer

This document specifies the methodology behind the published benchmark data in `bench/published/`: the LSP matrix (sections 1 to 7), the selectionRange fidelity measurement (section 9) and the earlier pipelines whose data is kept but whose tooling has been removed (section 8). Every published number is produced by the scripts in `bench/` from recorded runs; `docs/BENCHMARK_MATRIX.md`, `LSP_BENCHMARK_COMPARISON_REPORT.md` and the site data are generated from the published files (`bench/README.md`).

---

## 1. Evaluated Systems

1. **`ridx lsp` (Rust Intelligence)**: the framed JSON-RPC LSP server over the content-addressed `.ridx` index (`mmap`, flat tables). The index of each corpus is built before the run by `ridx index` with the `ridx-driver` recorded in the provenance (`driver_hash`, `driver_build_commit`, `driver_source`); the sysroot shards are recorded by `sysroot_cas_key`.
2. **`rust-analyzer` (live LSP server)**: the release binary of the toolchain under test (`ra_version`; 1.98.1 in the published runs), started with `"checkOnSave": false` so that no `cargo check` runs during the measurement.

Both servers are driven by the same client (`bench/lsp_process.py`): framed JSON-RPC over stdio, a reader thread, a 2 s timeout per request after which the client sends `$/cancelRequest` and counts the request as cancelled (`ridx_cancelled_on_timeout`, `ra_cancelled_on_timeout`).

---

## 2. Corpus and Isolation

- Five repositories: `serde-rs/serde`, `BurntSushi/ripgrep`, `tokio-rs/tokio`, `rust-lang/rust-analyzer`, `bevyengine/bevy`. Each run records the corpus commit (`repo_commit`) and the sha256 of the `Cargo.lock` the index was built from (`corpus_lockfile_sha256`). `bench/matrix_provenance.py` checks that the corpus has a RIDX index with a sysroot CAS key before anything is measured (`check_ridx_index`, `bench/test_index_precondition.py`) and records whether the driver binary is newer than that index (`driver_newer_than_index`).
- One repository and one server at a time on the dedicated node (`node`); the runner is started under `taskset` and records the CPU set it inherited (`taskset`; `24-31` in the published runs). Nothing else runs on the node during a run.
- Before each method the client waits for every answer the server still owes (`drain`), then sends a request no server implements and waits for its `-32601` answer, so queued notifications cannot leak into the next method's latencies (`bench/lsp_barrier.py`). Waits above 1 ms are recorded per method (`method_drains`).
- Readiness is measured separately from query latency (section 6).

---

## 3. Determinism

- Items are sampled by `bench/matrix_sampling.py` from a line-based regex scan of the corpus sources (`bench/scan.py`) with seed 42; file lists are sorted, so the sample depends neither on directory order nor on `PYTHONHASHSEED` (`bench/test_sampling_determinism.py`). The sampled positions are published next to every run (`matrix_<repo>_booster.positions.json`).
- The method list, the request parameters and the request or notification type of every row are fixed in `bench/matrix_categories.py` (section 5).
- The documents are regenerated from the published files only and are deterministic for a given set of files (`bench/test_matrix_report.py`, `bench/test_comparison_report.py`, `bench/test_export_site.py`).

---

## 4. Resource Telemetry

After the items of each method the runner reads `/proc/<pid>/status` of the server (`VmRSS`, `Threads`) and counts the entries of `/proc/<pid>/fd` (`get_proc_metrics` in `bench/metrics.py`). The published run keeps the peaks over the whole suite per server: `peak_rss_mb`, `peak_threads`, `peak_fds`. `VmRSS` includes file-backed pages, so for `ridx` it counts the pages of the mapped `.ridx` shards the kernel currently holds; those pages can be dropped under memory pressure, anonymous heap cannot. The matrix does not publish the split into anonymous and file-backed memory.

---

## 5. The LSP Matrix: 78 Methods and Scenarios, 300 Sampled Items Each

The matrix covers 78 LSP 3.17 methods, notifications and operational scenarios (`bench/matrix_categories.py`). For every method the harness draws 300 items per repository with a fixed seed (42) from a line-based regex scan of the corpus sources (`bench/scan.py`, `bench/matrix_sampling.py`); the sampled positions are published next to every run. The scanner is not a parser: it records method calls (`.name(`), path segments (`::name`), type annotations (`: Name`) and `fn`/`struct`/`enum`/`trait`/`type` declarations, skipping comment lines and the bodies of `quote!`/`macro_rules!` blocks. Items are drawn uniformly from these pools, without stratification by syntactic category, and the request parameters are built mechanically from the sampled position (`build_params` in `bench/matrix_categories.py`). What is sent, by group:

| Methods | Items and parameters |
|---|---|
| `initialize`, `initialized`, `shutdown`, `exit` | Run once each on a fresh server process (`tests_run` = 1); `docs/BENCHMARK_MATRIX.md` prints them as "1 run". |
| `$/cancelRequest`, `$/setTrace`, `window/workDoneProgress/cancel`, `workspace/didChangeWorkspaceFolders`, `workspace/didChangeConfiguration`, `workspace/didChangeWatchedFiles`, `workspace/didCreateFiles`, `workspace/didRenameFiles`, `workspace/didDeleteFiles` | 300 notifications with fixed payloads (cancel ids `bench-cancel-<i>` that no request uses, trace `off`, token `token_1`, the repository root as the added folder, empty settings, one change for a random file). `a` means the notification was written to the server. |
| `textDocument/didOpen`, `didChange` (both rows), `willSave`, `willSaveWaitUntil`, `didSave`, `didClose` | 300 random source files, each opened with its disk text and closed again. Both `didChange` rows send a full-text change (the disk text plus one appended comment line) and revert it; no incremental range edits are sent. `willSave` and `willSaveWaitUntil` always use reason 1 (manual). |
| `definition`, `typeDefinition`, `documentHighlight`, `moniker`, `selectionRange`, `inlineValue`, `inlineCompletion` | 300 random usage positions (method call, path segment or type annotation). |
| `declaration` | 300 random declaration positions. |
| `implementation`, `prepareTypeHierarchy`, `typeHierarchy/supertypes`, `typeHierarchy/subtypes` | 300 random `struct`/`enum`/`trait` declarations; the `supertypes`/`subtypes` item is synthesised from the sampled position (name, zero-width range), not taken from a `prepareTypeHierarchy` answer. |
| `references` | 150 random usages and 150 random declarations, `includeDeclaration: true`. |
| `hover` | 210 random usages and 90 random declarations. |
| `completion`, `signatureHelp`, `onTypeFormatting` | 300 random method-call positions (`onTypeFormatting` with `ch: ";"`). |
| `prepareCallHierarchy`, `callHierarchy/incomingCalls`, `callHierarchy/outgoingCalls` | 300 random `fn` declarations; the `incomingCalls`/`outgoingCalls` item is synthesised from the position, not taken from a `prepareCallHierarchy` answer. |
| `prepareRename`, `rename`, `linkedEditingRange`, `codeAction` | 300 random declarations; `rename` uses `<name>_renamed_<i>`; `codeAction` sends the zero-width range at the declaration with an empty diagnostics context. |
| `documentSymbol`, `foldingRange`, `codeLens`, `textDocument/diagnostic`, `documentLink`, `documentColor`, `formatting`, `semanticTokens/full`, `semanticTokens/full/delta` | 300 random files, whole-document requests. Files are taken as they are on disk: nothing is injected, unformatted or edited on purpose. |
| `textDocument/content` | 300 random files, sent as `{"uri"}`. Runs measured before the harness fix listed under known harness defects in `docs/BENCHMARK_MATRIX.md` sent `{"textDocument": {"uri"}}`, which ridx read as an empty uri and answered with an empty text that still counted as `a`; row 75 of those runs is not a measurement of the method. |
| `inlayHint`, `semanticTokens/range`, `rangeFormatting`, `colorPresentation` | 300 random files with a fixed range (lines 0 to 2; a zero-width range at 0:0 and a fixed grey colour for `colorPresentation`). |
| `workspace/symbol` | 300 queries derived from random declaration names in a five-way rotation: 60 exact names, 60 first halves, 60 inner substrings, 120 `nonexistent_<i>`. |
| `workspaceSymbol/resolve`, `completionItem/resolve`, `inlayHint/resolve`, `moniker/resolve`, `documentLink/resolve`, `codeAction/resolve`, `codeLens/resolve` | 300 synthetic items built by the harness (a placeholder symbol or label, a `noop` command), not items returned by a previous request. These rows show whether a server accepts a well-formed resolve request and how fast it answers; a server that echoes the item back counts as `a`. |
| `workspace/executeCommand` | One fixed command repeated 300 times: `ridx.resolveWorkspace` for ridx, `rust-analyzer.reloadWorkspace` for rust-analyzer. |
| `workspace/diagnostic` | One fixed pull request (`identifier: "workspace"`) repeated 300 times. |
| `workspace/workspaceFolders`, `workspace/configuration`, `workspace/applyEdit`, `textDocument/publishDiagnostics` | Server-to-client RPCs: reported as supported, no latency. |
| `workspace/willCreateFiles`, `willRenameFiles`, `willDeleteFiles` | 300 random files, one file operation each. |
| Mutation cases A, B, C (section 7) | 300 random file pairs or declarations each, all through unsaved overlays. Case B treats a found definition as a failure (`STALE_INDEX_LEAK_DETECTED`); cases A and C count an error-free answer as `a` without checking its target. |

For requests the harness asks three times per item: the first answer is the cold measurement, the mean of the second and third the warm one (section 6). `a` counts non-empty answers, `b` empty ones (`null`, `[]`, `{}`, a hover without contents), `c` error responses and client timeouts (2 s per request, cancelled with `$/cancelRequest`). For the rows that send an item built by the harness (the seven `*/resolve` methods, `workspace/executeCommand`, `callHierarchy/incomingCalls`, `callHierarchy/outgoingCalls`, `typeHierarchy/supertypes`, `typeHierarchy/subtypes`) `docs/BENCHMARK_MATRIX.md` shows `n/a (synthetic item)` instead of a/b/c and only latency and error codes; the counters stay in the published JSON. Outside case B nothing in the matrix checks that an answer is correct; correctness is measured by the test suite and by the per-method fidelity scripts (section 9).

### Total Benchmark Scale
- 74 methods x 300 items + 4 lifecycle methods x 1 run = **22,204 tests per repository per server** (`total_tests_per_server` in every published run).
- 111,020 tests per server across the five-repository corpus (`serde`, `ripgrep`, `tokio`, `rust-analyzer`, `bevy`).

### Execution and Publication:
- The matrix is run by `bench/matrix_runner.py` on the dedicated `booster` node, one repository at a time, both servers pinned to the same CPU set (`taskset`). Sampling is deterministic for a given seed (`bench/test_sampling_determinism.py`).
- Each run is published by `bench/publish_results.py` as `bench/published/matrix_<repo>_booster.json` together with `matrix_<repo>_booster.positions.json` (the sampled positions) and `matrix_<repo>_booster.provenance.json` (ridx commit, driver hash of the binary that built the index, corpus commit, sysroot CAS key, shard count, toolchain versions, node, CPU set, seed, timestamps and readiness waits, all copied from the runner output).
- `docs/BENCHMARK_MATRIX.md`, generated by `bench/matrix_report.py` from the published files, lists per-method results for both servers, the harness defects found during execution with the commits that fixed them, and the repositories whose runs are still pending.

---

## 6. Latency Measurements: Readiness and Cold vs. Warm

1. **Readiness**, measured and recorded separately from query latency:
   - `ridx`: time from process start until the server reports quiescence after `initialize` (`ridx_initial_wait_ms`).
   - `rust-analyzer`: three recorded steps: the server reports `quiescent` (`ra_initial_wait_ms`), every `$/progress` token has ended, and a `textDocument/definition` canary on the first three sampled positions answers non-empty twice in a row (`ra_canary_wait_ms`, `ra_canary_attempts`). The synchronization block (ids 8 to 14) makes rust-analyzer reload the workspace, so readiness is re-established before the first query method (`ra_quiescent_pre_query_wait_ms`, `ra_canary_pre_query_wait_ms`, `ra_canary_pre_query_attempts`).
2. **Query latency, three asks per item**: the first answer is the cold measurement, the mean of the second and third the warm one. Percentiles p50, p90, p95 and p99 of both are kept per method (`cold_*_us`, `warm_*_us`); the documents show warm p50 and p99.

---

## 7. Cross-File Semantic Mutations (Unsaved Editor Buffers)

To validate behavior on unsaved editor buffers via `textDocument/didChange`:

### Case A: New Symbol (Cross-File Overlay Linkage)
- In file A (`src/lib.rs` / `src/main.rs` or public module), a new function is declared:
  ```rust
  pub fn bench_probe_<idx>() -> usize { 42 }
  ```
- In file B of the same crate, an invocation is injected into an existing function body:
  ```rust
  let _probe = crate::<mod_path>::bench_probe_<idx>();
  ```
- A `goto_definition` request targets `bench_probe_<idx>` in file B.
- **Criterion**: The server must resolve the definition to file A, confirming semantic cross-file linkage between unsaved overlays without touching disk.

### Case B: Stale Overlay Detector
- In file A, a function is renamed (`func` $\to$ `func_renamed_stale`).
- File B remains unchanged.
- In file B, `goto_definition` is requested for the old identifier `func`.
- **Criterion**: The server must return `Not Found`. If the server returns the old definition, it fails for returning stale disk index data.

### Case C: Line Displacement
- 10 empty lines are prepended to file A.
- From file B, definition of a symbol in file A is queried.
- **Criterion**: The returned `targetSelectionRange` must shift by exactly +10 lines.

---

## 8. Earlier Pipelines (Historical Note)

The published files of the earlier pipelines stay in `bench/published/`; their tooling was removed from the tree after commit `9a75047`, the last commit that still contains it. `bench/README.md` lists the files and the `git checkout 9a75047 -- bench/<file>` commands that restore them.

1. **Five-repository pilot** (`pilot_5repos_2026-10-08.jsonl` with its `.provenance.json`): 300 read-only positions per repository plus mutation and worktree scenarios; answers compared with rust-analyzer as strict matches, line-level matches and semantically equivalent locations; discrepancies attributed to categories (missing in one server, associated type bindings, declaration vs implementation, divergent target); a `rust-analyzer scip` static index as a third system; process telemetry split into anonymous and file-backed RSS; Prometheus node-health guards and a `cargo`/`rustc` spawn guard around every run.
2. **Completion recall and precision reruns** (`rerun_completion_*.jsonl`, `completion_regression_*.json`): 100 completion positions on serde and bevy; candidates compared with rust-analyzer per scenario (recall, non-empty precision, empty answers, regressions to empty); paired baseline and current binaries with recorded driver and sysroot provenance.
3. **Hover rerun** after the type-table binary search (`rerun_hover_844eac0.jsonl`) and the **rename oracle** on tokio (`rename_oracle.json`: `cargo check --all-features` after the generated `WorkspaceEdit`).

The strict-match rates, sysroot discrepancy shares and ratios formerly quoted in this document and in `LSP_BENCHMARK_COMPARISON_REPORT.md` belong to the pilot and remain in the git history of both files.

---

## 9. selectionRange Fidelity against rust-analyzer

`bench/selection_range_fidelity.py --repo DIR --ra BIN --ridx before=BIN --ridx after=BIN --out FILE`
takes the 300 `selection_range` positions of the matrix sample (Section 5, seed 42), asks
rust-analyzer and every ridx binary given for the full `textDocument/selectionRange` chain
(twice per position: cold, then warm) and stores every chain in the output file together with
the binaries' `--version` output, the corpus commit and the node. Per ridx binary it reports:

- `invariant_violations`: innermost node not containing the cursor, or a parent not strictly
  containing its child;
- `innermost_in_ra`, `all_in_ra`: positions whose innermost node, or every node, occurs in the
  rust-analyzer chain; `nodes_in_ra / nodes` over all positions;
- `innermost_eq_ra_first_nonempty`: rust-analyzer's first node is always the empty range at the
  cursor, so the comparison is with its first non-empty node;
- `fn_item_start`: for every ridx `fn` item node (the parent of a `{..}` body node ending at the
  same place) whether rust-analyzer has the same range, would have it only with the start moved
  to the first `fn`/`pub`/... keyword line, or neither (fns written inside macro calls, where
  rust-analyzer has token trees and no FN node);
- cold and warm latency percentiles (p50, p90, p99, in microseconds) measured by the client.

Published runs: `bench/published/selection_range_fidelity_<corpus>_ram9.json` (ram9, rust-analyzer
1.98.1, ridx 59c9402 vs 26c6aef, five corpora, 300 positions each).
