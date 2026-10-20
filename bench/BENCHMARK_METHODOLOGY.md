# Benchmark Methodology: RIDX vs. rust-analyzer (Live) vs. rust-analyzer SCIP

This document specifies the validated, implemented methodology for comparative benchmarking of latency, resource consumption, and semantic accuracy: **Rust Intelligence (`ridx`)** vs. **`rust-analyzer 1.98.1` (live LSP daemon)** and **`rust-analyzer scip` (static index)**.

---

## 1. Architecture and Evaluated Systems

The benchmark evaluates three distinct systems:

1. **`ridx` (Rust Intelligence)**:
   - Pre-computed content-addressed CAS index (`.ridx`), flat `mmap` format.
   - Zero heap allocation during read queries (zero-memory cold and warm lookups).
   - Operates in CLI mode (`ridx def`, `ridx refs`, `ridx hover`) and as a framed JSON-RPC LSP daemon (`ridx lsp`).

2. **`rust-analyzer` (Live LSP daemon)**:
   - Full-featured LSP server version 1.98.1.
   - Incremental Salsa compiler engine, live semantic analysis, background checks.
   - Configured with `"checkOnSave": false` to prevent uncontrolled background `cargo check` execution.

3. **`rust-analyzer scip` (Static Index)**:
   - Ahead-of-time static index generation in SCIP Protobuf format (`rust-analyzer scip <dir> --output index.scip --exclude-vendored-libraries`).
   - Parsed into memory via the official `scip 0.10.0` crate by an in-memory loader (`ScipIndex`).
   - Enables direct "static index vs. static index" comparison isolated from Salsa and JSON-RPC overhead.

---

## 2. Test Corpus and Environment Isolation

### 2.1. Repository Corpus
- Reference dataset: top 100 open-source Rust repositories (`bench/corpus/rust-100.tsv`).
- Core benchmark suite (5 diverse projects):
  1. `serde-rs/serde` (macros, codegen, traits)
  2. `BurntSushi/ripgrep` (CLI, system APIs, complex types)
  3. `tokio-rs/tokio` (async runtime, concurrency)
  4. `rust-lang/rust-analyzer` (heavy monorepo, deep AST trees)
  5. `bevyengine/bevy` (large game engine, massive codebase)

### 2.2. Environmental Isolation
- Each repository is checked out into an isolated directory pinned to an exact commit SHA.
- **Sequential Execution**: Exactly one project runs at any time. Concurrent benchmark runs on the same node are prohibited.
- **Artifact Cleanup**: After completing measurements for each repository, `target/` directories and `index.scip` files are purged to prevent cache pollution.

---

## 3. Noise Elimination

To guarantee reproducibility and eliminate measurement artifacts:

1. **CPU Core Pinning**:
   - All benchmark processes (`cargo check`, `ridx index`, `rust-analyzer scip`, `ridx lsp`, `rust-analyzer lsp`) are pinned to 8 isolated cores on dedicated node `booster` via `taskset -c 24-31`.
2. **Compiler Background Process Guard (`pgrep`)**:
   - During query execution, a background monitor samples the process tree (`/proc/<pid>/stat`) every 5 ms.
   - If the server spawns `cargo` or `rustc` during query evaluation, the run is flagged with `cargo_spawned: true`.
3. **Prometheus Hardware Guard**:
   - At the beginning and end of each project run, node health metrics are inspected via Prometheus:
     - `CPU Steal Time > 1.0%`
     - `CPU Throttling > 0`
     - `Node CPU Load > 85%`
     - `Disk IO Wait` spikes
   - Any anomaly triggers a `NOISE_REJECTED` status and invalidates the run.

---

## 4. Resource Telemetry: Descriptors, Threads, and Disaggregated Memory

The runner polls Linux kernel `/proc/<pid>/` telemetry every 5 ms:

| Metric | Source | Purpose and Description |
| :--- | :--- | :--- |
| **`peak_open_fds`** | `count(/proc/<pid>/fd)` | Number of open file descriptors; tracks open file handles and inotify watchers to detect FD leaks. |
| **`peak_threads`** | `/proc/<pid>/status` (`Threads:`) | Peak OS thread count (rayon threadpools, tokio workers, Salsa workers). |
| **`peak_rss_anon_mb`** | `/proc/<pid>/status` (`RssAnon:`) | **Anonymous Physical RAM (Heap/Stack)** that cannot be paged to disk. Reflects true runtime heap footprint. |
| **`peak_rss_file_mb`** | `/proc/<pid>/status` (`RssFile:`) | **Page cache memory from `mmap` files**. For `ridx`, index pages can be evicted cleanly under OS memory pressure. |

Telemetry runs across all phases:
- Indexing: `cargo check`, `ridx index`, `rust-analyzer scip`.
- Serving: `ridx lsp`, `rust-analyzer`, and `scip-loader` sessions.

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
| `textDocument/content` | 300 random files, but sent as `{"textDocument": {"uri"}}`; the method takes `{"uri"}`, so ridx answers every item with an empty text that still counts as `a`. The row is not a measurement of the method until the parameter shape is fixed. |
| `inlayHint`, `semanticTokens/range`, `rangeFormatting`, `colorPresentation` | 300 random files with a fixed range (lines 0 to 2; a zero-width range at 0:0 and a fixed grey colour for `colorPresentation`). |
| `workspace/symbol` | 300 queries derived from random declaration names in a five-way rotation: 60 exact names, 60 first halves, 60 inner substrings, 120 `nonexistent_<i>`. |
| `workspaceSymbol/resolve`, `completionItem/resolve`, `inlayHint/resolve`, `moniker/resolve`, `documentLink/resolve`, `codeAction/resolve`, `codeLens/resolve` | 300 synthetic items built by the harness (a placeholder symbol or label, a `noop` command), not items returned by a previous request. These rows show whether a server accepts a well-formed resolve request and how fast it answers; a server that echoes the item back counts as `a`. |
| `workspace/executeCommand` | One fixed command repeated 300 times: `ridx.resolveWorkspace` for ridx, `rust-analyzer.reloadWorkspace` for rust-analyzer. |
| `workspace/diagnostic` | One fixed pull request (`identifier: "workspace"`) repeated 300 times. |
| `workspace/workspaceFolders`, `workspace/configuration`, `workspace/applyEdit`, `textDocument/publishDiagnostics` | Server-to-client RPCs: reported as supported, no latency. |
| `workspace/willCreateFiles`, `willRenameFiles`, `willDeleteFiles` | 300 random files, one file operation each. |
| Mutation cases A, B, C (section 7) | 300 random file pairs or declarations each, all through unsaved overlays. Case B treats a found definition as a failure (`STALE_INDEX_LEAK_DETECTED`); cases A and C count an error-free answer as `a` without checking its target. |

For requests the harness asks three times per item: the first answer is the cold measurement, the mean of the second and third the warm one (section 6). `a` counts non-empty answers, `b` empty ones (`null`, `[]`, `{}`, a hover without contents), `c` error responses and client timeouts (2 s per request, cancelled with `$/cancelRequest`). Outside case B nothing in the matrix checks that an answer is correct; correctness is measured by the test suite and by the per-method fidelity scripts (section 9).

### Total Benchmark Scale
- 74 methods x 300 items + 4 lifecycle methods x 1 run = **22,204 tests per repository per server** (`total_tests_per_server` in every published run).
- 111,020 tests per server across the five-repository corpus (`serde`, `ripgrep`, `tokio`, `rust-analyzer`, `bevy`).

### Execution and Publication:
- The matrix is run by `bench/matrix_runner.py` on the dedicated `booster` node, one repository at a time, both servers pinned to the same CPU set (`taskset`). Sampling is deterministic for a given seed (`bench/test_sampling_determinism.py`).
- Each run is published by `bench/publish_results.py` as `bench/published/matrix_<repo>_booster.json` together with `matrix_<repo>_booster.positions.json` (the sampled positions) and `matrix_<repo>_booster.provenance.json` (ridx commit, driver hash of the binary that built the index, corpus commit, sysroot CAS key, shard count, toolchain versions, node, CPU set, seed, timestamps and readiness waits, all copied from the runner output).
- `docs/BENCHMARK_MATRIX.md`, generated by `bench/matrix_report.py` from the published files, lists per-method results for both servers, the harness defects found during execution with the commits that fixed them, and the repositories whose runs are still pending.

---

## 6. Latency Measurements: Warmup and Cold vs. Warm Separation

1. **Readiness / Warmup Time**:
   - `ridx`: time to open CAS manifest and mount `mmap` segments (`readiness_warmup_micros`).
   - `rust-analyzer`: time from process start to background analysis completion and ping response (`readiness_warmup_micros`).
   - `rust-analyzer` in the matrix runs: readiness is established in three steps and recorded separately: the server reports `quiescent` (`ra_initial_wait_ms`), every `$/progress` token has ended, and a `textDocument/definition` canary on the first three sampled positions returns two consecutive non-empty answers (`ra_canary_wait_ms`, `ra_canary_attempts`). The synchronization block (methods 8-14) makes rust-analyzer reload the workspace, so the same quiescence and canary check is repeated before the first query method (`ra_quiescent_pre_query_wait_ms`, `ra_canary_pre_query_wait_ms`, `ra_canary_pre_query_attempts`).
   - `scip`: time to parse Protobuf index into memory (`scip_load_ms`).
   - *Readiness time is strictly separated from query evaluation latency.*

2. **Query Latency (3 iterations per probe)**:
   - **Iteration 1**: `Cold latency` (first read, cold page cache and CPU cache).
   - **Iterations 2 and 3**: `Warm latency` (median of subsequent queries).
   - Percentile calculations: **p50, p90, p95, p99** per method (`def`, `hover`, `refs`).

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

*(For SCIP, mutation tests are marked `N/A` as SCIP is a purely static offline index format).*

---

## 8. Discrepancy Arbitration and Reporting

1. **Discrepancy Logging**:
   - All response discrepancies (one server found a target, the other returned null, or differing ranges) are recorded in `bench/results/discrepancies.jsonl` with full JSON-RPC payloads.

2. **Attribution Categories**:
   - `missing_in_ra`: Baseline returned `null` / `Not found` while RIDX resolved target (RIDX advantage).
   - `missing_in_ridx`: RIDX returned `null` / `Not found` while baseline located definition (e.g. proc-macros without Salsa expansion or unindexed crates).
   - `associated_type_binding`: Identifiers in associated type equality constraints (`hir::TypeBinding`, e.g. `where V: Visitor<'de, Value = ()>`, `S::Error`, `Serializer<Ok = ()>`).
   - `same_file_decl_vs_impl` (Class C): Trait default method body vs declaration header (semantically equivalent locations, both valid).
   - `line_level_match` (Class B): Coordinates within 2 lines tolerance in the same file.
   - `divergent_target`: Different symbols, files, or macro expansion span offsets.

3. **Standard Library (Sysroot) Discrepancy Accounting & Expected Impact**:
   - **Distinction between Discrepancy Share and Strict-Match Percentage Points**:
     When analyzing non-matching queries targeting standard library items (`std`/`core`/`alloc`), the proportion of *discrepancies* targeting sysroot must not be conflated with percentage points ($\text{pp}$) of overall accuracy.
     The mathematical formula for projected improvement is:
     $$\Delta_{\text{strict-match}} = \frac{N_{\text{sysroot\_discrepancies}}}{N_{\text{total\_scenarios}}} \times 100\%$$
     In the 5-repository pilot benchmark:
     - Out of 426 recorded discrepancy lines (386 unique scenarios across 2,100 evaluated scenarios), **113 records (26.5% of recorded discrepancies; 99 unique scenarios or 25.6%)** target items residing in the standard library sysroot (`std`/`core`/`alloc`).
     - Across all 2,100 evaluated scenarios, the expected impact of standard library sysroot indexing is up to **+4.71 percentage points** (raising aggregate strict match from 81.62% to 86.33%).
     - Across the 1,500 read-only queries (300 per repository), 73 of the 182 non-matching queries target sysroot (40.1% of read-only discrepancies). The expected impact on read-only strict match is up to **+4.87 percentage points** (raising read-only strict match from 87.87% to 92.73%).
   - **Architectural Behavior for Standard Library References**:
     In the standard library (`std`/`core`/`alloc`), go-to-definition (`textDocument/definition`) on method calls and field accesses (e.g. `iter.next()`, `atomic.fetch_sub()`) succeeds even without a pre-indexed sysroot shard because `rustc` preserves the `target_file` path pointing into `rustlib/src` within the `RawReference` payload.
     However, operations that require index definition shards—including hover signatures and documentation (`textDocument/hover`), find references (`textDocument/references`), and workspace symbol searches (`workspace/symbol`)—do not resolve for standard library items until sysroot crates are pre-indexed into CAS shards.

4. **Structured Results**:
   - Each entry in `bench/results/pilot_results.jsonl` contains complete percentiles, RSS breakdown (Anon/File), open file descriptors (FDs), thread counts, build durations, and Prometheus node status.

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
