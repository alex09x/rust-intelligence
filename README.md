# Rust Intelligence (`ridx`)

Ultra-fast, zero-memory, compiler-driven code intelligence and Language Server Protocol (LSP 3.18) engine for Rust, inspired by `gopls` and powered by `rustc_driver`.

---

## 1. The Core Problem: Why `rust-analyzer` Consumes Gigabytes

In large Rust workspaces (e.g. Tokio, `rust-analyzer` itself, or BTCR with hundreds of crates), `rust-analyzer` consumes:
- **2.5 – 3.7 GB RSS** of active heap memory.
- **50%+ of memory** is spent holding Rowan green/red syntax trees and raw file text buffers for thousands of files.
- Over **98% of those files** belong to external registry dependencies (`crates.io`) and the standard library, which developers **never edit**.
- Cold start requires **10 to 60 seconds** of re-parsing, macro expansion, and type inference inside a separate database engine (Salsa) that duplicates the work `rustc` already performs.

---

## 2. The Solution: Compiler-Driven Zero-Copy Indexing

`rustc` already computes all definitions, references, trait resolutions, and inferred expression types during `cargo check`.

Instead of running a heavy persistent daemon that duplicates compiler logic, **Rust Intelligence** introduces a zero-copy, binary on-disk index format (`.ridx` v2):

- **No Idle Daemon for CLI Navigation**: CLI queries (`ridx def`, `ridx refs`, `ridx hover`) execute on-demand without any running background daemon (0.00 MB idle resident memory). Indices are paged directly from the OS page cache via zero-copy `mmap(2)`.
- **Lightweight LSP Server**: `ridx lsp` runs with a compact resident memory footprint (~12–15 MB RSS) storing only active editor buffers and lexical overlays, without holding multi-gigabyte compiler syntax trees in RAM.
- **Zero Open File Descriptors**: Under POSIX, memory mappings persist after file descriptor closure (`close(fd)`). `ridx` maintains zero open file descriptors for index shards, never exhausting `ulimit -n`.
- **Instant Response Latencies**:
  - `goto definition`: **10 – 30 µs** ($O(\log N)$ binary search).
  - `hover` & `type-at`: **20 – 50 µs**.
  - `find references`: **0.4 – 4 ms** across millions of references in hundreds of crates.
- **Two-Tier Content-Addressed Architecture**: External dependencies are indexed once into a global cache and shared across all workspaces and worktrees without re-compilation.
- **Live Memory Overlay**: Real-time code changes in editors are tracked by an in-memory `OverlayManager` with sub-millisecond response and full AST mapping before disk saves.

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
│  - Lexical scoping & rename   │                          │  - Zero background daemon RSS         │
└───────────────────────────────┘                          └───────────────────┬───────────────────┘
                                                                               │
                                                   ┌───────────────────────────┴───────────────────────────┐
                                                   ▼                                                       ▼
                                ┌─────────────────────────────────────┐         ┌─────────────────────────────────────┐
                                │   Tier 1: Global Cache (CAS)        │         │   Tier 2: Workspace Variant         │
                                │   ~/.cache/ridx/v2/objects/         │         │   <project>/target/ridx/variants/   │
                                │   - 680+ immutable crates (crates.io│         │   - Local project member shards     │
                                │   - 100% cache hit (1-5 ms link)    │         │   - ridx_manifest.json (whitelist)  │
                                └─────────────────────────────────────┘         └─────────────────────────────────────┘
```

### Two-Tier Indexing Explained
1. **Tier 1 — Global Immutable Object Store (`~/.cache/ridx/v2/objects/`)**:
   External dependencies from `crates.io` and git repositories are marked as immutable packages. They are indexed **once** per unique source hash / compilation target tuple. Any workspace or worktree on the machine using that crate reuses the precompiled shard instantly.
2. **Tier 2 — Workspace Variant Store (`target/ridx/variants/<variant-id>/`)**:
   Contains shards for the project's own workspace member crates, plus a strict whitelist manifest (`ridx_manifest.json`). Every project only accesses the crates declared in its `Cargo.lock` — preventing any cross-project symbol pollution.

### Distributed Compilation with Mitts
When combined with `mitts` (distributed compilation and build artifact store):
- Precompiled `.ridx` index shards are published alongside compilation artifacts in CAS.
- Fresh checkouts and new Git Worktrees achieve **0-second initial indexing**: `.ridx` shards are linked directly from cache without running local `cargo check`.

---

## 4. Full LSP 3.18 Capability Matrix

`ridx lsp` implements core LSP 3.18 protocol capabilities backed by zero-copy index lookup and live buffer overlays (see [ROADMAP.md](ROADMAP.md) for detailed per-method status):

| Category | LSP Methods | Latency (`ridx`) | Latency (`rust-analyzer`) |
|---|---|---|---|
| **Navigation** | `textDocument/definition`<br>`textDocument/declaration`<br>`textDocument/implementation`<br>`textDocument/typeDefinition` | **10 – 30 µs** | 30 – 10,000 ms |
| **References** | `textDocument/references`<br>`textDocument/documentHighlight` | **0.4 – 4 ms** | 100 – 1,500 ms |
| **Intelligence** | `textDocument/hover`<br>`textDocument/completion`<br>`textDocument/signatureHelp`<br>`textDocument/inlayHint` | **20 – 150 µs** | 20 – 350 ms |
| **Hierarchies** | `textDocument/prepareCallHierarchy`<br>`callHierarchy/incomingCalls`<br>`callHierarchy/outgoingCalls`<br>`typeHierarchy/supertypes`<br>`typeHierarchy/subtypes` | **0.1 – 1.6 ms** | 50 – 400 ms |
| **Refactoring** | `textDocument/prepareRename`<br>`textDocument/rename`<br>`textDocument/codeAction`<br>`workspace/executeCommand` | **0.2 – 2 ms** | 100 – 800 ms |
| **Formatting** | `textDocument/formatting`<br>`textDocument/rangeFormatting`<br>`textDocument/onTypeFormatting` | **5 – 25 ms** | 10 – 40 ms |
| **Symbols** | `textDocument/documentSymbol`<br>`workspace/symbol`<br>`workspaceSymbol/resolve` | **0.1 – 15 ms** | 20 – 350 ms |
| **Diagnostics** | `textDocument/publishDiagnostics`<br>`textDocument/diagnostic`<br>`workspace/diagnostic` | **Background async (flycheck)** | 500 – 4,000 ms |
| **Workspaces** | `workspace/didChangeWorkspaceFolders`<br>`workspace/didChangeWatchedFiles` | **< 1 ms** | 500 – 5,000 ms |

---

## 5. Real-World Benchmark Results

Measurements comparing local `ridx` index lookups against a warm, pinned `rust-analyzer 1.97.1` language server instance on a 32-core Linux server with warm OS page caches (detailed methodology, command pins, and reproduction instructions in [LSP_BENCHMARK_COMPARISON_REPORT.md](LSP_BENCHMARK_COMPARISON_REPORT.md)):

### A. Tokio Workspace (42 Crates, 880 Files, 298k References)
| Capability | Target Symbol | `ridx` Latency (CLI) | `rust-analyzer` Latency | Speedup | Verified Target |
|---|---|---|---|---|---|
| **Definition** | `runtime::context::blocking::try_enter_blocking_region` | **24 µs** | 10,554 ms | **439,785x** | Identical definition span |
| **References** | `runtime::context::blocking::try_enter_blocking_region` | **655 µs** | 1,273 ms | **1,944x** | Full reference set |
| **Hover** | `runtime::context::blocking::try_enter_blocking_region` | **42 µs** | 344 ms | **8,189x** | Matching signature & docs |
| **Callers** | `runtime::context::blocking::try_enter_blocking_region` | **262 µs** | 363 ms | **1,389x** | Matching call hierarchy |
| **Symbols** | `Runtime` | **16.9 ms** | 323 ms | **19.1x** | Matching symbol list |
| **Idle Daemon RSS**| Workspace Idle | **0.00 MB (CLI)** / **~12 MB (LSP)** | **1,420 MB** | **> 118x** | Resident set size |

### B. `rust-analyzer` Workspace (261 Crates, 3,050 Files, 2.3M References)
| Capability | Target Symbol | `ridx` Latency (CLI) | `rust-analyzer` Latency | Speedup | Verified Target |
|---|---|---|---|---|---|
| **Definition** | `ide::AnalysisHost` | **704 µs** | 10,905 ms | **15,490x** | Identical definition span |
| **References** | `ide::AnalysisHost` | **4.06 ms** | 1,487 ms | **365x** | Full reference set |
| **Hover** | `ide::AnalysisHost` | **410 µs** | 381 ms | **931x** | Matching signature & docs |
| **Callers** | `ide::AnalysisHost` | **1.61 ms** | 382 ms | **236x** | Matching call hierarchy |
| **Idle Daemon RSS**| Workspace Idle | **0.00 MB (CLI)** / **~15 MB (LSP)** | **3,240 MB** | **> 216x** | Resident set size |

---

## 6. Binary Format Specification (`.ridx` v2)

All records are little-endian with strict 8-byte alignment:

```
┌────────────────────────────────────────────────────────┐
│ Header (Magic 'RIDX', Version 2, Offsets & Counts)     │
├────────────────────────────────────────────────────────┤
│ String Intern Pool (Null-terminated Deduplicated UTF-8)│
├────────────────────────────────────────────────────────┤
│ File Path Table (Array of FileEntry, 16 bytes each)    │
├────────────────────────────────────────────────────────┤
│ Definitions Table (Array of DefEntry, 40 bytes each)   │
├────────────────────────────────────────────────────────┤
│ Definitions Hash Index (FNV-1a 64-bit Path Hashes, 16B)│
├────────────────────────────────────────────────────────┤
│ References Table (Sorted by Span, 48 bytes each)       │
├────────────────────────────────────────────────────────┤
│ References Target Hash Index (Target Hashes, 16B)      │
├────────────────────────────────────────────────────────┤
│ Types Table (Array of TypeEntry, 28 bytes each)        │
└────────────────────────────────────────────────────────┘
```

---

## 7. CLI Quick Start

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

# Run as Language Server Protocol (LSP) server
ridx lsp
```

---

## 8. Development & Verification

All code changes adhere to strict modularity guidelines (every source file $\le 400$ lines):

```bash
# Format check
cargo fmt --check

# Strict lint check
cargo clippy --workspace --all-targets -- -D warnings

# Run all workspace unit and integration tests (217 tests)
cargo test --workspace
```

---

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.
