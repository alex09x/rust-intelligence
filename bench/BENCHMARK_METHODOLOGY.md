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

## 5. Comprehensive Neutral Sampling: 300 Variations per Method across LSP 3.17 (78 Methods and Scenarios)

The benchmark evaluates **all 78 distinct LSP 3.17 methods, notifications, and operational scenarios**. Each method is sampled across **300 diverse test cases** per repository.

Query targets and parameters are extracted using an AST scanner from source code:

### Domain 1: Lifecycle & Control
| # | LSP Method | RPC | Volume | Test Variations (300 per method) |
|:---:|---|:---:|:---:|---|
| 1 | **`initialize`** | Request | **300** | 300 capability combinations (dynamic registration, hierarchical symbols, snippets, inlay hints, pull diagnostics, workspace folders). |
| 2 | **`initialized`** | Notif | **300** | 300 connection establishment sessions with varying workspace root sets. |
| 3 | **`shutdown`** | Request | **300** | 300 server shutdown sequences from varied states (cold start, background build, active overlays). |
| 4 | **`exit`** | Notif | **300** | 300 exit validations (exit code 0 following shutdown, exit code 1 on abort without shutdown). |
| 5 | **`$/cancelRequest`** | Notif | **300** | 300 cancellation requests (cancelling heavy `references`, `workspace/symbol`, `semanticTokens` early and late). |
| 6 | **`$/setTrace`** | Notif | **300** | 300 dynamic trace level switches (`off`, `messages`, `verbose`) under load. |
| 7 | **`window/workDoneProgress/cancel`** | Notif | **300** | 300 client-side cancellations of background progress tokens during indexing. |

### Domain 2: Document Synchronization
| # | LSP Method | RPC | Volume | Test Variations (300 per method) |
|:---:|---|:---:|:---:|---|
| 8 | **`textDocument/didOpen`** | Notif | **300** | 300 diverse files: large monoliths (>3,000 lines), small modules, BOM files, emoji/surrogate pairs. |
| 9 | **`textDocument/didChange` (Full)** | Notif | **300** | 300 full document replacement scenarios across varied code structures. |
| 10 | **`textDocument/didChange` (Incremental)** | Notif | **300** | 300 incremental edits: token inserts, function deletes, multiline UTF-16 edits. |
| 11 | **`textDocument/willSave`** | Notif | **300** | 300 pre-save notifications with varying `TextDocumentSaveReason` (Manual, AfterDelay, FocusOut). |
| 12 | **`textDocument/willSaveWaitUntil`** | Request | **300** | 300 pre-save edit requests (trailing whitespace cleanup, import organization). |
| 13 | **`textDocument/didSave`** | Notif | **300** | 300 disk saves verifying manifest reload and incremental flycheck triggers. |
| 14 | **`textDocument/didClose`** | Notif | **300** | 300 file closures validating overlay eviction and return to compiled disk baseline. |

### Domain 3: Language Intelligence
| # | LSP Method | RPC | Volume | Test Variations (300 per method) |
|:---:|---|:---:|:---:|---|
| 15 | **`textDocument/definition`** | Request | **300** | 100 function calls, 60 paths (`Mod::item`), 40 type annotations, 30 macro calls, 30 struct fields, 40 constructors. |
| 16 | **`textDocument/declaration`** | Request | **300** | 120 trait method prototypes, 80 extern "C" functions, 100 associated types and constants. |
| 17 | **`textDocument/typeDefinition`** | Request | **300** | 100 composite structs, 80 enum variants, 60 type aliases (`type Result<T>`), 60 return types. |
| 18 | **`textDocument/implementation`** | Request | **300** | 100 standard traits (`Clone`, `Debug`), 100 domain traits, 100 generic structs with multiple `impl` blocks. |
| 19 | **`textDocument/references`** | Request | **300** | 150 usage sites, 150 declarations: private methods, public APIs, traits, fields, static constants. |
| 20 | **`textDocument/documentHighlight`** | Request | **300** | 120 local variables, 80 function arguments, 50 shadowing bindings, 50 struct fields. |
| 21 | **`textDocument/documentSymbol`** | Request | **300** | 300 files: flat modules, deep `impl` hierarchies, macros, structs with 100+ fields. |
| 22 | **`workspace/symbol`** | Request | **300** | 80 exact names, 80 prefixes, 60 fuzzy matches, 40 CamelCase, 20 short (1–2 chars), 20 non-existent. |
| 23 | **`workspaceSymbol/resolve`** | Request | **300** | 300 symbols verifying lazy resolution of doc-comments, signatures, and container paths. |
| 24 | **`textDocument/hover`** | Request | **300** | 90 structs and enums, 70 generic functions, 50 primitives (`u32`), 30 macros, 30 keywords (`match`, `impl`), 30 lifetimes (`'a`). |
| 25 | **`textDocument/completion`** | Request | **300** | 100 completions after `.`, 80 after `::`, 40 auto-imports, 40 local variables, 40 snippets and keywords. |
| 26 | **`completionItem/resolve`** | Request | **300** | 300 completion items for lazy documentation and extended text edit resolution. |
| 27 | **`textDocument/signatureHelp`** | Request | **300** | 100 functions with 1–2 args, 80 with 4+ args, 60 generic functions, 30 `&mut self` methods, 30 closures. |
| 28 | **`textDocument/inlayHint`** | Request | **300** | 120 `let x = ...` types, 100 parameter names, 50 `iter().map()` chains, 30 closure return types. |
| 29 | **`inlayHint/resolve`** | Request | **300** | 300 inlay hints verifying tooltip enrichment and source definition ranges. |
| 30 | **`textDocument/inlineValue`** | Request | **300** | 300 debugging probe positions to evaluate local variables and arguments. |
| 31 | **`textDocument/inlineCompletion`** | Request | **300** | 300 cursor positions for ghost text inline completion evaluation. |
| 32 | **`textDocument/moniker`** | Request | **300** | 300 symbol positions for exported LSIF/SCIP moniker verification. |
| 33 | **`moniker/resolve`** | Request | **300** | 300 monikers verifying package, scheme, and version metadata. |

### Domain 4: Hierarchies
| # | LSP Method | RPC | Volume | Test Variations (300 per method) |
|:---:|---|:---:|:---:|---|
| 34 | **`textDocument/prepareCallHierarchy`** | Request | **300** | 300 function positions: public APIs, private helpers, closures, trait methods. |
| 35 | **`callHierarchy/incomingCalls`** | Request | **300** | 300 functions (from isolated leaf functions to root dispatchers with 10+ callers). |
| 36 | **`callHierarchy/outgoingCalls`** | Request | **300** | 300 functions (from simple atomic functions to deep invocation chains). |
| 37 | **`textDocument/prepareTypeHierarchy`** | Request | **300** | 150 structs, 100 traits, 50 enums initializing hierarchy trees. |
| 38 | **`typeHierarchy/supertypes`** | Request | **300** | 300 queries for supertypes and base trait inheritance chains. |
| 39 | **`typeHierarchy/subtypes`** | Request | **300** | 300 queries for implementors and subtypes across all workspace crates. |

### Domain 5: Selection, Links & Colors
| # | LSP Method | RPC | Volume | Test Variations (300 per method) |
|:---:|---|:---:|:---:|---|
| 40 | **`textDocument/selectionRange`** | Request | **300** | 300 cursor positions for stepwise expansion (token $\to$ expression $\to$ block $\to$ fn $\to$ file). |
| 41 | **`textDocument/documentLink`** | Request | **300** | 300 doc comments with links to external URLs and local file paths. |
| 42 | **`documentLink/resolve`** | Request | **300** | 300 links validating destination URI normalization. |
| 43 | **`textDocument/documentColor`** | Request | **300** | 300 color definitions in code (HEX, RGB, RGBA constants in UI crates). |
| 44 | **`textDocument/colorPresentation`** | Request | **300** | 300 color format conversions (HEX $\to$ RGB, RGBA $\to$ HSL). |

### Domain 6: Editing & Refactoring
| # | LSP Method | RPC | Volume | Test Variations (300 per method) |
|:---:|---|:---:|:---:|---|
| 45 | **`textDocument/formatting`** | Request | **300** | 300 unformatted files evaluated for whole-document formatting via rustfmt. |
| 46 | **`textDocument/rangeFormatting`** | Request | **300** | 300 line ranges with misaligned indents, whitespace, and wrap anomalies. |
| 47 | **`textDocument/onTypeFormatting`** | Request | **300** | 300 trigger characters: typing `;`, `}`, and newlines within function bodies. |
| 48 | **`textDocument/prepareRename`** | Request | **300** | 150 valid identifiers, 100 keyword renames (`fn`, `match`), 50 macro invocations. |
| 49 | **`textDocument/rename`** | Request | **300** | 300 transactional cross-file renames verifying multi-file replacements. |
| 50 | **`textDocument/linkedEditingRange`**| Request | **300** | 300 synchronized paired range editing scenarios. |
| 51 | **`textDocument/codeAction`** | Request | **300** | 100 unused imports, 100 quick-fix diagnostics, 100 refactoring candidates. |
| 52 | **`codeAction/resolve`** | Request | **300** | 300 actions verifying accurate `WorkspaceEdit` diff generation. |
| 53 | **`workspace/executeCommand`** | Request | **300** | 300 custom LSP commands (including `ridx.applyEdits`, `cargo.reload`, test running). |
| 54 | **`workspace/applyEdit` (Client RPC)**| RPC | **300** | 300 two-phase edit scenarios verifying rollback on client rejection. |

### Domain 7: Presentation & Tokens
| # | LSP Method | RPC | Volume | Test Variations (300 per method) |
|:---:|---|:---:|:---:|---|
| 55 | **`textDocument/semanticTokens/full`** | Request | **300** | 300 files: full syntax tokenization (types, macros, constants, keywords). |
| 56 | **`textDocument/semanticTokens/full/delta`** | Request | **300** | 300 incremental delta token queries after localized buffer edits. |
| 57 | **`textDocument/semanticTokens/range`** | Request | **300** | 300 visible screen viewports (lines 1–50, 100–160, etc.) across diverse file sizes. |
| 58 | **`textDocument/foldingRange`** | Request | **300** | 300 files: function bodies, `match` arms, `unsafe` blocks, multiline doc-comments. |
| 59 | **`textDocument/codeLens`** | Request | **300** | 300 code lens probe sites: reference counts above functions, `#[test]` runners. |
| 60 | **`codeLens/resolve`** | Request | **300** | 300 lenses verifying lazy title and command parameter resolution. |

### Domain 8: Diagnostics
| # | LSP Method | RPC | Volume | Test Variations (300 per method) |
|:---:|---|:---:|:---:|---|
| 61 | **`textDocument/diagnostic`** | Request | **300** | 150 valid clean files, 150 files with injected syntax and semantic errors. |
| 62 | **`workspace/diagnostic`** | Request | **300** | 300 global pull-diagnostic queries across all workspace crates. |
| 63 | **`textDocument/publishDiagnostics`** | Push | **300** | 300 streamed compiler diagnostic checks from background Flycheck. |

### Domain 9: Workspace & File Operations
| # | LSP Method | RPC | Volume | Test Variations (300 per method) |
|:---:|---|:---:|:---:|---|
| 64 | **`workspace/workspaceFolders`** | Request | **300** | 300 folder enumeration scenarios across multi-root workspaces. |
| 65 | **`workspace/didChangeWorkspaceFolders`**| Notif | **300** | 300 dynamic workspace root additions and removals. |
| 66 | **`workspace/didChangeConfiguration`** | Notif | **300** | 300 live configuration updates without server restart. |
| 67 | **`workspace/configuration`** | Request | **300** | 300 client section configuration lookups (`rust-analyzer`, `ridx`). |
| 68 | **`workspace/didChangeWatchedFiles`** | Notif | **300** | 300 disk file change notifications verifying cache invalidation. |
| 69 | **`workspace/didCreateFiles`** | Notif | **300** | 300 file creation notifications with automatic indexing. |
| 70 | **`workspace/willCreateFiles`** | Request | **300** | 300 pre-creation file validations. |
| 71 | **`workspace/didRenameFiles`** | Notif | **300** | 300 file rename notifications with overlay rebinding. |
| 72 | **`workspace/willRenameFiles`** | Request | **300** | 300 pre-rename file validations. |
| 73 | **`workspace/didDeleteFiles`** | Notif | **300** | 300 file deletion notifications with overlay eviction. |
| 74 | **`workspace/willDeleteFiles`** | Request | **300** | 300 pre-deletion file validations. |
| 75 | **`textDocument/content`** | Request | **300** | 300 virtual document buffer inspections. |

### Domain 10: Memory Mutation Scenarios
| # | Mutation Scenario | Type | Volume | Test Variations (300 per method) |
|:---:|---|:---:|:---:|---|
| 76 | **Case A: New Symbol (Cross-file in-scope)** | Mutation | **300** | 300 cross-file links: declare new function in file A, invoke with qualified path in file B without saving to disk. |
| 77 | **Case B: Stale Overlay Detector** | Mutation | **300** | 300 renames in file A verifying that querying the old name in file B returns `Not Found` rather than stale disk data. |
| 78 | **Case C: Line Displacement** | Mutation | **300** | 300 coordinate shift verifications: insert 10 empty lines in file A with target range in file B shifting by exactly +10 lines. |

---

### Total Benchmark Scale:
- **78 methods and scenarios x 300 variations = 23,400 tests per repository**.
- **117,000 measurements** across the 5-project reference corpus (`serde`, `ripgrep`, `tokio`, `rust-analyzer`, `bevy`).
- Comprehensive Error Attribution Matrix: on discrepancies, the runner categorizes the syntax class (e.g. unexpanded macro, generic, cyclic import, closure).

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
