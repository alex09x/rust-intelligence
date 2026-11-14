# Rust Intelligence (`ridx`)

Compiler-backed, mmap-indexed code intelligence and Language Server Protocol (LSP 3.18) engine for Rust, inspired by `gopls` and powered by `rustc_driver`.

---

## 1. The Core Problem: Why `rust-analyzer` Consumes Gigabytes

In large Rust workspaces (e.g. Tokio, `rust-analyzer` itself, or large multi-crate monorepos), `rust-analyzer` consumes:
- **2.5 – 3.7 GB VmRSS** of resident memory (see [`LSP_BENCHMARK_COMPARISON_REPORT.md`](LSP_BENCHMARK_COMPARISON_REPORT.md)).
- **50%+ of memory** is spent holding Rowan green/red syntax trees and raw file text buffers for thousands of files.
- Over **98% of those files** belong to external registry dependencies (`crates.io`) and the standard library, which developers **never edit**.
- Cold start requires **10 to 60 seconds** of re-parsing, macro expansion, and type inference inside a separate database engine (Salsa) that duplicates the work `rustc` already performs.

---

## 2. The Solution: Compiler-Driven On-Disk Indexing (`.ridx` v3)

`rustc` already computes all definitions, references, trait resolutions, and inferred expression types during `cargo check`.

Instead of running a heavy persistent analysis database that duplicates compiler logic, **Rust Intelligence** introduces a binary on-disk index format (`.ridx` v3):

- **No Idle Daemon for CLI Navigation**: CLI queries (`ridx def`, `ridx refs`, `ridx hover`) execute on-demand without any persistent background daemon. Indices are paged directly from the OS page cache via `mmap(2)`.
- **Measured LSP Memory Footprint (`VmRSS`)**: Measured after `initialize` + one `hover` request using the same `VmRSS` metric as [`LSP_BENCHMARK_COMPARISON_REPORT.md`](LSP_BENCHMARK_COMPARISON_REPORT.md):
  - `serde` (31 shards): **39 MB VmRSS** (`11 MB` anonymous heap, `28 MB` file-backed `mmap`).
  - `bevy` (640 shards): **651 MB VmRSS** (`346 MB` anonymous heap including `PathIndex` and `BlastRadiusIndex`, `305 MB` file-backed `mmap`), compared to **2.5 – 3.7 GB VmRSS** for `rust-analyzer` ([`LSP_BENCHMARK_COMPARISON_REPORT.md`](LSP_BENCHMARK_COMPARISON_REPORT.md)).
- **Zero Open File Descriptors**: Under POSIX, memory mappings persist after file descriptor closure (`close(fd)`). `ridx` maintains zero open file descriptors for index shards, never exhausting `ulimit -n`.
- **Measured 5-Corpus Request Latencies (`c2aaec7` vs `rust-analyzer`, p50 / p95 in µs)**:
  Generated directly from the 5-corpus benchmark reports in [`bench/published/`](bench/published/) (`d7fbd78`, 300 sampled positions per method per corpus):

| Corpus | `definition` (`ridx` vs `ra` p50 / p95 µs) | `hover` (`ridx` vs `ra` p50 / p95 µs) | `references` (`ridx` vs `ra` p50 / p95 µs) | `completion` (`ridx` vs `ra` p50 / p95 µs) |
|---|---:|---:|---:|---:|
| `serde` | `53.9 / 612.4` vs `599.9 / 5281.1` | `70.4 / 422.2` vs `100.4 / 336.1` | `271.5 / 3086.1` vs `2774.5 / 24829.5` | `203.3 / 803.5` vs `3832.7 / 11312.7` |
| `ripgrep` | `63.7 / 178.9` vs `1155.5 / 8283.4` | `76.3 / 173.4` vs `145.7 / 355.2` | `219.2 / 2953.4` vs `2194.8 / 23468.2` | `214.7 / 665.6` vs `2795.1 / 7709.2` |
| `tokio` | `411.6 / 1734.7` vs `1604.1 / 15984.8` | `653.1 / 1428.7` vs `182.3 / 619.2` | `2134.8 / 6912.3` vs `20429.8 / 191175.6` | `2024.1 / 6947.0` vs `3661.5 / 9987.3` |
| `rust-analyzer` | `188.7 / 570.8` vs `2400.6 / 51550.2` | `279.6 / 578.7` vs `222.1 / 1861.9` | `1005.3 / 10608.4` vs `2118.6 / 221375.7` | `1398.2 / 3526.2` vs `5494.9 / 15835.4` |
| `bevy` | `410.1 / 897.5` vs `3106.5 / 49103.2` | `628.9 / 994.9` vs `281.4 / 3029.6` | `7716.5 / 25250.5` vs `24977.6 / 406300.2` | `2667.6 / 11740.5` vs `10338.6 / 48449.2` |

- **Two-Tier Content-Addressed Architecture**: External dependencies are indexed once into a global cache and shared across all workspaces and worktrees without re-compilation.
- **Live Memory Overlay**: Real-time code changes in editors are tracked by an in-memory `OverlayManager` with sub-millisecond response and AST mapping before disk saves.

---

## 3. High-Level Architecture

```
                             ┌────────────────────────────────┐
                             │       LSP Client / Editor      │
                             │  (VS Code, Neovim, Zed, Helix) │
                             └───────────────┬────────────────┘
                                             │ JSON-RPC (stdio)
                                             ▼
                             ┌────────────────────────────────┐
                             │           ridx lsp             │
                             │ (Dispatcher & Lifecycle Mgr)   │
                             └───────┬────────────────┬───────┘
                                     │                │
            ┌────────────────────────┘                └────────────────────────┐
            ▼                                                                  ▼
┌───────────────────────────────┐                          ┌───────────────────────────────────────┐
│     OverlayManager (RAM)      │                          │         WorkspaceIndex (mmap)         │
│  - Active open buffers        │                          │  - MultiRoot workspace routing        │
│  - Live incremental edits     │                          │  - Strict Cargo.lock scope isolation  │
│  - Lexical scoping & rename   │                          │  - On-demand mmap shard paging        │
└───────────────────────────────┘                          └───────────────────┬───────────────────┘
                                                                               │
                                                   ┌───────────────────────────┴───────────────────────────┐
                                                   ▼                                                       ▼
                                ┌─────────────────────────────────────┐         ┌─────────────────────────────────────┐
                                │   Tier 1: Global Cache (CAS)        │         │   Tier 2: Workspace Variant         │
                                │   ~/.cache/ridx/v3/objects/         │         │   <project>/target/ridx/variants/   │
                                │   - 680+ immutable crates (crates.io│         │   - Local project member shards     │
                                │   - 100% cache hit (1-5 ms link)    │         │   - ridx_manifest.json (whitelist)  │
                                └─────────────────────────────────────┘         └─────────────────────────────────────┘
```

### Two-Tier Indexing Explained
1. **Tier 1 — Global Immutable Object Store (`~/.cache/ridx/v3/objects/`)**:
   External dependencies from `crates.io` and git repositories are marked as immutable packages. They are indexed **once** per unique source hash / compilation target tuple. Any workspace or worktree on the machine using that crate reuses the precompiled shard instantly.
2. **Tier 2 — Workspace Variant Store (`target/ridx/variants/<variant-id>/`)**:
   Contains shards for the project's own workspace member crates, plus a strict whitelist manifest (`ridx_manifest.json`). Every project only accesses the crates declared in its `Cargo.lock` — preventing any cross-project symbol pollution.

### Distributed Compilation with Mitts
When combined with `mitts` (distributed compilation and build artifact store):
- Precompiled `.ridx` index shards are published alongside compilation artifacts in CAS.
- Fresh checkouts and new Git Worktrees achieve **0-second initial indexing**: `.ridx` shards are linked directly from cache without running local `cargo check`.

---

## 4. Full LSP 3.18 Capability Matrix

`ridx lsp` implements core LSP 3.18 protocol capabilities backed by mmap index lookup and live buffer overlays (see [ROADMAP.md](ROADMAP.md) for detailed per-method status):

| Category | LSP Methods |
|---|---|
| **Navigation** | `textDocument/definition`<br>`textDocument/declaration`<br>`textDocument/implementation`<br>`textDocument/typeDefinition` |
| **References** | `textDocument/references`<br>`textDocument/documentHighlight` |
| **Intelligence** | `textDocument/hover`<br>`textDocument/completion`<br>`textDocument/signatureHelp`<br>`textDocument/inlayHint` |
| **Hierarchies** | `textDocument/prepareCallHierarchy`<br>`callHierarchy/incomingCalls`<br>`callHierarchy/outgoingCalls`<br>`typeHierarchy/supertypes`<br>`typeHierarchy/subtypes` |
| **Refactoring** | `textDocument/prepareRename`<br>`textDocument/rename`<br>`textDocument/codeAction`<br>`workspace/executeCommand` |
| **Formatting** | `textDocument/formatting`<br>`textDocument/rangeFormatting`<br>`textDocument/onTypeFormatting` |
| **Symbols** | `textDocument/documentSymbol`<br>`workspace/symbol`<br>`workspaceSymbol/resolve` |
| **Diagnostics** | `textDocument/publishDiagnostics`<br>`textDocument/diagnostic`<br>`workspace/diagnostic` |
| **Workspaces** | `workspace/didChangeWorkspaceFolders`<br>`workspace/didChangeWatchedFiles` |

Measured latencies and answer counters for all 78 methods on five repositories are in [`docs/BENCHMARK_MATRIX.md`](docs/BENCHMARK_MATRIX.md).

---

## 5. Benchmark Results & Content Parity (`v0.2.0`)

> [!NOTE]
> The LSP matrix benchmark runs all 78 methods against `ridx` and `rust-analyzer` on five open-source repositories (`serde`, `ripgrep`, `tokio`, `rust-analyzer`, `bevy`), 300 sampled items per method and 22,204 requests per server and repository. Results: [`LSP_BENCHMARK_COMPARISON_REPORT.md`](LSP_BENCHMARK_COMPARISON_REPORT.md) (headline methods, resources, coverage differences) and [`docs/BENCHMARK_MATRIX.md`](docs/BENCHMARK_MATRIX.md) (every method, provenance, known harness defects). Methodology: [`bench/BENCHMARK_METHODOLOGY.md`](bench/BENCHMARK_METHODOLOGY.md).

### 5-Corpus Content-Parity Summary (`c2aaec7` vs `rust-analyzer`)

Derived directly from [`bench/published/position_ab_parity_c2aaec7_vs_ra_ram9_<corpus>.differing.json`](bench/published/) (`d7fbd78`, 300 positions per method per repository, denominator = non-empty `rust-analyzer` responses):

| Corpus | `definition` | `hover` | `references` (line prec / rec) | `completion: target label in list (recall, not parity) / top-20` |
|---|---:|---:|---:|---:|
| `serde` | 259/282 (91.8%) | 280/288 (97.2%) | 208/283 (73.5%, 96.7% / 77.1%) | 245/299 (81.9% / 66.6%, exact parity 12/299 = 4.0%) |
| `ripgrep` | 290/294 (98.6%) | 290/293 (99.0%) | 274/295 (92.9%, 99.8% / 97.5%) | 256/298 (85.9% / 71.1%, exact parity 6/298 = 2.0%) |
| `tokio` | 219/229 (95.6%) | 242/243 (99.6%) | 158/236 (66.9%, 68.8% / 74.0%) | 191/261 (73.2% / 61.7%, exact parity 8/261 = 3.1%) |
| `rust-analyzer` | 261/276 (94.6%) | 242/278 (87.1%) | 165/267 (61.8%, 99.5% / 88.8%) | 219/296 (74.0% / 55.4%, exact parity 5/296 = 1.7%) |
| `bevy` | 264/283 (93.3%) | 278/284 (97.9%) | 163/271 (60.1%, 89.7% / 81.4%) | 197/289 (68.2% / 49.1%, exact parity 6/289 = 2.1%) |
| **Total (5 corpora)** | **1293/1364 (94.8%)** | **1332/1386 (96.1%)** | **968/1352 (71.6%, 90.4% / 83.5%)** | **1108/1443 (76.8% / 60.8%, exact parity = 37/1443 (2.6%))** |

- **Enriched Hover (Beyond `rust-analyzer`)**: `textDocument/hover` returns the full multi-line Rust item signature (`where` clauses, bounds), extracted doc comments (`///`, `#[doc = "..."]`, `#[cfg_attr(..., doc = "...")]`), canonical crate/file **Provenance**, and precomputed $O(1)$ **Blast Radius** counters (`N direct references across M files in K crates`, deduplicated across compilation targets and Cargo packages).

## 6. Binary Format Specification (`.ridx` v3)

All records are little-endian with strict 8-byte alignment:

```
┌────────────────────────────────────────────────────────┐
│ Header (Magic 'RIDX', Version 3, Offsets & Counts)     │
├────────────────────────────────────────────────────────┤
│ String Intern Pool (Null-terminated Deduplicated UTF-8)│
├────────────────────────────────────────────────────────┤
│ File Path Table (Array of FileEntry, 16 bytes each)    │
├────────────────────────────────────────────────────────┤
│ Definitions Table (Array of DefEntry, 44 bytes each)   │
├────────────────────────────────────────────────────────┤
│ Definitions Hash Index (FNV-1a 64-bit Path Hashes, 16B)│
├────────────────────────────────────────────────────────┤
│ Owner Hash Index & Deref Table (Receiver Type Chains)  │
├────────────────────────────────────────────────────────┤
│ References Table (Sorted by Span, 48 bytes each)       │
├────────────────────────────────────────────────────────┤
│ References Target Hash Index (Target Hashes, 16B)      │
├────────────────────────────────────────────────────────┤
│ Types Table (Array of TypeEntry, 28 bytes each)        │
└────────────────────────────────────────────────────────┘
```

---

## 7. Installation, Editor Setup & CLI Quick Start

### Installation

1. **Pinned Rust Toolchain**:
   `ridx-driver` links against `rustc_driver` and requires the pinned `1.98.1` toolchain with compiler internal components declared in [`rust-toolchain.toml`](rust-toolchain.toml) (`channel = "1.98.1"`, `components = ["rustfmt", "clippy", "rustc-dev", "llvm-tools-preview", "rust-src"]`) and `RUSTC_BOOTSTRAP=1` (configured in [`.cargo/config.toml`](.cargo/config.toml)):
   ```bash
   rustup toolchain install 1.98.1 --component rustfmt clippy rustc-dev llvm-tools-preview rust-src
   ```

2. **Install `ridx` and `ridx-driver`**:
   Both binaries (`ridx` for CLI/LSP and `ridx-driver` for `RUSTC_WORKSPACE_WRAPPER` extraction during `ridx index`) must be installed on `PATH`:
   ```bash
   RUSTC_BOOTSTRAP=1 cargo +1.98.1 install --path crates/ridx --locked
   RUSTC_BOOTSTRAP=1 cargo +1.98.1 install --path crates/ridx-driver --locked
   ```
   Because `ridx-driver` links dynamically against `librustc_driver` (`-C prefer-dynamic`), ensure the toolchain sysroot library directory is in your dynamic loader path if your environment does not embed rpath:
   ```bash
   # Linux
   export LD_LIBRARY_PATH="$(rustup run 1.98.1 rustc --print sysroot)/lib:${LD_LIBRARY_PATH:-}"
   # macOS
   export DYLD_LIBRARY_PATH="$(rustup run 1.98.1 rustc --print sysroot)/lib:${DYLD_LIBRARY_PATH:-}"
   ```

3. **Index Your Workspace Before Opening the Editor**:
   Run `ridx index .` in the target Cargo workspace root before launching your editor so `.ridx` v3 shards are generated under `target/ridx/` and `~/.cache/ridx/v3/objects/`:
   ```bash
   cd /path/to/your/rust/workspace
   ridx index .
   ```

4. **Behavior When No Index Exists in the Workspace**:
   If `ridx lsp` is started in a workspace before `ridx index .` has been run (no `target/ridx/` shards exist), `ridx lsp` starts cleanly with `0` loaded index shards, serves open-buffer lexical overlay features (`didOpen`/`didChange` document symbols, folding ranges, selection ranges, formatting, and single-file lexical hover/definitions/references/highlights/completions), and returns `null` / `[]` for cross-crate index-backed queries without crashing or failing initialization. Once `ridx index .` completes (or when `.ridx` file creation/update events arrive via `workspace/didChangeWatchedFiles`), `ridx lsp` loads the new shards automatically.

### Drop-in `rust-analyzer` Editor Integration

`ridx` can be invoked directly without a subcommand as a drop-in `rust-analyzer` replacement (supporting `--version`, `-v`/`-q`, `--log-file <path>`, and `--print-config-schema`). Ready-to-use editor configurations are provided in [`docs/editors/`](docs/editors/):
- **VS Code**: [`docs/editors/vscode-settings.json`](docs/editors/vscode-settings.json) (`"rust-analyzer.server.path": "ridx"`)
- **Zed**: the [`editors/zed`](editors/zed) extension registers `ridx lsp` as the `rust-intelligence` language server (`zed: install dev extension`, then [`docs/editors/zed-settings.json`](docs/editors/zed-settings.json) selects it for Rust)
- **Neovim (`nvim-lspconfig`)**: [`docs/editors/neovim-lspconfig.lua`](docs/editors/neovim-lspconfig.lua) (`cmd = { "ridx" }`)

### CLI Usage

```bash
# Index a Cargo workspace
ridx index .

# Inspect index structure, crate breakdown, and record counts
ridx inspect .

# Go to definition by symbol or file:line:col
ridx def . my_crate::run
ridx def . src/main.rs:12:15

# Find references across entire workspace and dependencies
ridx refs . my_crate::run
ridx refs . src/main.rs:12:15

# Query hover or expression type
ridx hover . src/main.rs:12:15
ridx type-at . src/main.rs:12:15

# Search workspace symbols
ridx symbols . Config

# Run side-by-side benchmark comparison against rust-analyzer
ridx compare -c all . my_crate::run

# Run as Language Server Protocol (LSP) server (or invoke `ridx` with no subcommand)
ridx lsp
```

> **Note on updates**: After updating `ridx` (or modifying compiler driver flags), run `ridx index .` (or `ridx index . --all-features`) to re-generate workspace indices with updated canonical symbol definitions and paths.

### Known limitations
- **First-request synchronous index load on large workspaces (`bevy`, 640 shards)**: On the first index-backed LSP request in a workspace with $N$ shards, `ridx lsp` synchronously builds `PathIndex` and `BlastRadiusIndex` across all loaded shards (`~14.9 s` on `bevy` with 640 shards in `c2aaec7`, vs `13.3 s` on `462fe99`). Background warmup at `initialized` and lazy index construction are planned for `v0.3.0`.
- **`hover` p50 latency slower than `rust-analyzer` on 3 of 5 corpora**: In the `c2aaec7` 5-corpus run, `hover` p50 is slower than `rust-analyzer` on `tokio` (`653.1` vs `182.3 µs`), `rust-analyzer` (`279.6` vs `222.1 µs`), and `bevy` (`628.9` vs `281.4 µs`), while faster on `serde` (`70.4` vs `100.4 µs`) and `ripgrep` (`76.3` vs `145.7 µs`).
- **`references` (`968/1352 = 71.6%` full-set parity, `90.4%` workspace line precision, `83.5%` line recall)**: Across all 1,500 sampled rows in `d7fbd78` (`1,352` non-empty `rust-analyzer` responses), the script-computed non-exact / supplementary class breakdown is: `p_refs_partial_overlap = 177`, `c_refs_active_ws_superset = 67`, `p_b_to_a_raw_string_fixture = 63`, `c_ridx_answers_ra_empty_or_err = 52`, `p_refs_partial_mixed_decl_and_usages = 50`, `c_refs_active_ws_lines_equal = 44`, `p_refs_ra_only_self_keyword = 32`, `p_refs_ridx_excess = 25`, `p_refs_module_path_segment = 13`, `p_refs_trait_impl_sibling_decl = 8`, `p_target_custom_build = 3`, `p_b_to_a_type_relative_assoc_path = 3`, `p_b_to_a_unstored_workspace_file = 3`, `p_refs_external_decl_only = 2`, `p_refs_ra_only_macro_def = 2`, `p_refs_path_segment_cursor_line = 2`, `p_b_to_a_cfg_gated_item = 1`.
- **`completion` (`1108/1443 = 76.8%` target label recall, `878/1443 = 60.8%` in top-20, `exact parity = 37/1443 (2.6%)`)**: Scope-aware and type-directed completion covers `self.`/`Type::` receiver members and local bindings (`1311/1443` partial matches), while chained generic receivers (`p_comp_assoc_or_unresolved_receiver`, `95` rows) and full trait-method import candidate sets are planned for `v0.3.0`.
- **Custom `build.rs` generated code & inactive `#[cfg(...)]` targets**: Items guarded by target-conditional compilation (`#[cfg(windows)]`, `#[cfg(unix)]`, feature gates) or custom `OUT_DIR` bindings (`p_target_custom_build = 3`) are indexed for the active compilation target and feature set specified during `ridx index` (`--all-features`, `--target <triple>`).
- **Standard library (`std`, `core`, `alloc`)**: Definition navigation and hover for standard library items are supported via pre-indexed sysroot shards. Finding references to standard library items is scoped to workspace crates, and renaming standard library symbols is prohibited.
- **`Self` keywords**: `Self` is not counted as a type reference in references queries (`p_refs_ra_only_self_keyword = 32`).
- **Macro invocations (`vec![]`)**: `vec![]` invocations are reported as references to `Vec` (`rust-analyzer` does not report them).
- **Incoming calls (`callHierarchy/incomingCalls`)**: A caller is listed only where the reference to the function is followed by a call (`name(` or `name::<...>(`). `rust-analyzer` also lists functions passed as values, such as a system handed to `add_systems` or a function given to `map`.

---

## 8. Development & Verification

All code changes adhere to strict modularity guidelines (every source file $\le 400$ lines):

```bash
# Format check
cargo fmt --all -- --check

# Strict lint check
cargo clippy --workspace -- -D warnings

# Run all workspace unit and integration tests
cargo test --workspace
```

---

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.
