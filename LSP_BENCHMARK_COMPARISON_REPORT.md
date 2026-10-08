## Benchmark Report: `serde-rs/serde` (commit `6693a89c`) — Preliminary (Serde only)
*Source Data:* `bench/results/pilot_results.jsonl`  
*Run Status:* `SUCCESS`  
*Driver SHA256:* `fa9a79e81bdfbcf14f27345ec895b0f52c2a99b34a9fa95845e05eee891cf76f`  
*rust-analyzer Version:* `rust-analyzer 1.98.1 (48a229c 2026-09-01)`  
*Target Node:* `booster` (AMD Ryzen 9 7950X, 32 cores, 128 GB RAM, NVMe SSD, isolated on cores 24–31)

### 1. Headline Performance & Resource Footprint

| Metric | `ridx lsp` | `rust-analyzer` (Live) | Comparison / Notes |
|---|---|---|---|
| **Cold Clean Index / Check** | **4.157 s** (`ridx index`) | 1.2 s (`cargo check`) | 3.4x of cargo check |
| **Warm Incremental Re-index (W1)** | **2707.5 ms** (`ridx index`) | **150.2 ms** (RA quiescent) | 18.0x slower |
| **Peak Resident Set Size (RSS)** | **49.5 MB** (Anon: **13.7 MB**, File: 35.9 MB) | **1314.9 MB** (Anon: **1283.4 MB**, File: 31.5 MB) | **94.0x lower Heap RAM** |
| **Active OS Threads** | **2 threads** | **37 threads** | 18.5x fewer threads |
| **Open File Descriptors** | **3 total** (0 open `.ridx` FDs) | **9 total** | Fixed mmap file handles |
| **Def Latency (Warm p50)** | **120.5 µs** | **134.5 µs** | Comparable on quiet node (1.1x) |
| **Def Latency (Warm p90)** | **959.1 µs** | **5168.6 µs** | **5.4x faster tail latency** |
| **Hover Latency (Warm p50)** | **149.5 µs** | **104.0 µs** | Comparable on quiet node (0.7x) |
| **Refs Latency (Warm p50)** | **141.0 µs** | **1617.0 µs** | **11.5x faster** |
| **Refs Latency (Warm p90)** | **2053.8 µs** | **27886.1 µs** | **13.6x faster tail latency** |
| **Read-Only Strict Match Rate** | **259 / 300 (86.3%, 95% CI [82.0%, 89.8%]) (86.33%)** | Baseline | Wilson 95% CI |
| **Read-Only Line Diagnostic Match** | **259 / 300 (86.3%, 95% CI [82.0%, 89.8%]) (86.33%)** | Baseline | Same-line resolution |
| **cargo Spawned During Queries** | **ridx: false, ra: false** | **Quiescent** | Verified via process monitoring |

### 2. Latency Analysis & Measured Architectural Differences

On a quiet, dedicated build node (`booster`, AMD Ryzen 9 7950X, 32 cores, 128 GB RAM, isolated on cores 24–31), warmed in-memory query latencies for single-token lookups at median (p50) are roughly on par: rust-analyzer achieves **134.5 µs** (def) and **104.0 µs** (hover), while RIDX achieves **120.5 µs** (def) and **149.5 µs** (hover). The earlier 20–45x speedup figures were artifacts of a noisy test environment on `ram8`.

Measured architectural differences from the test run:
1. **Tail Latency**: `textDocument/definition` p90 is **5.4x faster** (959.1 µs vs 5168.6 µs / 5.17 ms).
2. **Multi-Token Reference Search**: `textDocument/references` is **11.5x faster at p50** (141.0 µs vs 1617.0 µs) and **13.6x faster at p90** (2053.8 µs vs 27886.1 µs).
3. **Process Resources**: RIDX maintains 2 OS threads and 3 open file descriptors, whereas rust-analyzer scales to 37 threads and 9 file descriptors.
4. **Memory Footprint**: RIDX uses **94.0x lower Heap RAM** (Anon Heap 13.7 MB vs 1283.4 MB).
5. **Multi-Worktree Scaling**: Across 10 concurrent worktrees, RIDX consumes **20.6 MB PSS** total (7.9 MB Anon) vs rust-analyzer's **9211.0 MB PSS** (9181.1 MB Anon) — a **447.6x lower PSS** footprint (workload: 50 def queries per process vs 300 queries + mutations in single-process main run).
6. **Warm Incremental Re-index (W1)**: rust-analyzer reaches freshness in **150.2 ms**, which is **18.0x faster** than RIDX (**2707.5 ms**).

### 3. Semantic Parity & Discrepancies Breakdown

Across the sampled 300 read-only scenarios (259 strict matches, 86.3% [259 / 300 (86.3%, 95% CI [82.0%, 89.8%])]),
the 41 non-matching cases were dynamically classified from telemetry:

- **Associated Type Bindings in Trait Bounds (24 cases)**: Identifiers in associated type equality constraints (`hir::TypeBinding`, e.g. `where V: Visitor<'de, Value = ()>`, `S::Error`, `Serializer<Ok = ()>`).

- **Trait Default Method vs Declaration (6 cases)**: Same source file, default implementation body vs trait declaration header (e.g. `Error::invalid_type`).

- **Divergent Target (4 cases)**: Different symbols or macro span offsets.

- **Missing in rust-analyzer (4 cases)**: rust-analyzer returned `null` / `Not found` while RIDX resolved target.

- **Missing in RIDX (3 cases)**: RIDX returned `Not found` while rust-analyzer found definition.

### 4. Live Mutation Benchmarks (Edit Loop & Overlay)
Evaluated under framed JSON-RPC with `textDocument/didChange` (Wilson 95% Confidence Intervals):

| Mutation Kind | `ridx lsp` Success (n/N) | `rust-analyzer` Success (n/N) | RIDX p50 | RA p50 | Architectural Note |
|---|---|---|---|---|---|
| `line_shift` | **30 / 30 (100.0%, 95% CI [88.6%, 100.0%])** | **30 / 30 (100.0%, 95% CI [88.6%, 100.0%])** | **0.1 ms** | **3.9 ms** | Overlay delta |
| `rename_stale` | **29 / 30 (96.7%, 95% CI [83.3%, 99.4%])** | **27 / 30 (90.0%, 95% CI [74.4%, 96.5%])** | **0.1 ms** | **0.5 ms** | Stale detection |
| `new_symbol` | **0 / 30 (0.0%, 95% CI [0.0%, 11.4%])** | **27 / 30 (90.0%, 95% CI [74.4%, 96.5%])** | **0.0 ms** | **0.2 ms** | Pre-save null by design |

### 5. Worktree Scenarios (W1, W2, W3)

**Scenario W1: Real LSP Commit Walk (99 commits)**
- **Immediate Stale (before re-index completes)**: RIDX = **35 / 853 (4.1%, 95% CI [3.0%, 5.7%])**, RA = **23 / 913 (2.5%, 95% CI [1.7%, 3.8%])**
- **Post-Reindex Stale (after re-index completes)**: RIDX = **0 / 853 (0.0%, 95% CI [0.0%, 0.4%])**, RA = **0 / 913 (0.0%, 95% CI [0.0%, 0.4%])**
- **Time to Freshness**: RIDX median = **2707.5 ms** (`ridx index`), RA quiescent median = **150.2 ms**

**Scenario W2: Large Git Jumps (10 iterations)**
- Total disk growth: **57.1 MB** (bounded CAS cache)

**Scenario W3: Multi-Worktree Concurrency & PSS Scaling**

> [!NOTE]
> Workload caveat: Each live server process was actively probed with 50 sequential definition requests (compared to 300 queries + live mutations in the single-process main run). Headlines and scaling comparisons use PSS multipliers (proportional share of physical RAM).

| Worktrees | `ridx` PSS (MB) | `ridx` Anon (MB) | RA PSS (MB) | RA Anon (MB) | CAS Delta (MB) | Cargo Check (ms) | CAS Index (ms) |
|---|---|---|---|---|---|---|---|
| **1** | **6.8** | **1.0** | **970.0** | **941.3** | **5.7** | 1223.7 | **4131.0** |
| **2** | **9.0** | **2.0** | **1903.7** | **1874.5** | **11.4** | 1254.1 | **4049.8** |
| **5** | **15.3** | **4.9** | **4641.9** | **4612.2** | **28.6** | 1208.6 | **4047.1** |
| **10** | **20.6** | **7.9** | **9211.0** | **9181.1** | **52.2** | 1257.5 | **3606.3** |

*CAS & Indexing Breakdown:*
- **CAS Growth (+5.2 MB per worktree)**: Represents first-party workspace crate objects compiled for differing source states across commit offsets (`HEAD~0`, `HEAD~5`, ...). Third-party dependencies are 100% deduplicated.
- **Index Duration**: Corresponds to cargo executing `ridx-driver` across all workspace crate targets. Once cached in CAS, index assembly and subsequent LSP lookups are sub-millisecond.
