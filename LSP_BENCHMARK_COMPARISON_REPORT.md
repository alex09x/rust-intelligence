# Executive Summary: Pilot Benchmark (RIDX vs rust-analyzer)

- **Evaluated Corpus**: 5 repos (2100 scenarios, 1500 read-only queries)
- **Overall Strict Match Rate**: **1714/2100 (81.62%)** | **Read-Only**: **1318/1500 (87.87%)**
- **Recorded Discrepancies**: 426 lines (386 unique non-matching scenarios across all repositories)
- **Standard Library Sysroot Impact**: 113 of 426 recorded lines (26.5%), 99 of 386 unique scenarios (25.6%) target items in `std`/`core`/`alloc`
  - **Overall Scenarios**: expected effect of sysroot indexing is up to **+4.71 percentage points** (81.62% → 86.33%)
  - **Read-Only Queries**: expected effect of sysroot indexing is up to **+4.87 percentage points** (87.87% → 92.73%)
- **Standard Library Reference Resolution Note**: In `std`/`core`/`alloc`, go-to-definition succeeds on method calls/references (e.g. `iter.next()`, `atomic.fetch_sub()`) because `rustc` preserves target span in `rustlib/src` within `RawReference`. However, hover signatures, find-references, and workspace symbol searches require sysroot indexing.

---

## Benchmark Report: `serde-rs/serde` (commit `6693a89c`)
*Source Data:* `bench/results/pilot_results.jsonl`  
*Run Status:* `SUCCESS`  
*Driver SHA256:* `ac554e8f3b302af637109a0679adf95aff0d8d35937d6f6b4e99770044b720c8`  
*rust-analyzer Version:* `rust-analyzer 1.98.1 (48a229c 2026-09-01)`  
*Target Node:* `booster` (AMD Ryzen 9 7950X, 16 cores / 32 threads, 128 GB RAM, NVMe SSD)

### 1. Headline Performance & Resource Footprint

| Metric | `ridx lsp` | `rust-analyzer` (Live) | Comparison / Notes |
|---|---|---|---|
| **Cold Clean Index / Check** | **3.968 s** (`ridx index`) | 1.2 s (`cargo check`) | 3.2x of cargo check |
| **Warm Incremental Re-index (W1)** | **3160.3 ms** (`ridx index`) | **150.2 ms** (RA quiescent) | 21.0x slower |
| **Peak Resident Set Size (RSS)** | **47.9 MB** (Anon: **14.0 MB**, File: 34.4 MB) | **1324.2 MB** (Anon: **1292.2 MB**, File: 32.0 MB) | **92.3x lower Heap RAM** |
| **Active OS Threads** | **2 threads** | **37 threads** | 18.5x fewer threads |
| **Open File Descriptors** | **3 total** (0 open `.ridx` FDs) | **9 total** | Fixed mmap file handles |
| **Def Latency (Warm p50)** | **43.0 µs** | **131.0 µs** | 3.0x faster |
| **Def Latency (Warm p90)** | **204.3 µs** | **5449.9 µs** | **26.7x faster tail latency** |
| **Hover Latency (Warm p50)** | **68.0 µs** | **103.5 µs** | 1.5x faster |
| **Refs Latency (Warm p50)** | **93.5 µs** | **1534.0 µs** | **16.4x faster** |
| **Refs Latency (Warm p90)** | **1183.4 µs** | **24493.8 µs** | **20.7x faster tail latency** |
| **Read-Only Strict Match Rate (Class A)** | **262 / 300 (87.3%, 95% CI [83.1%, 90.6%]) (87.33%)** | Baseline | Wilson 95% CI |
| **Read-Only Line Diagnostic Match (Class B)** | **262 / 300 (87.3%, 95% CI [83.1%, 90.6%]) (87.33%)** | Baseline | Same-line resolution |
| **Semantic Parity (Class A + Class C)** | **269 / 300 (89.7%, 95% CI [85.7%, 92.6%]) (89.67%)** | Baseline | Strict + Class C default method equiv |
| **Compiler ICE & Caught Panics** | **0 ICE, 0 caught panics** | 0 ICE | Driver telemetry verified |
| **cargo Spawned During Queries** | **ridx: false, ra: false** | **Quiescent** | Verified via process monitoring |

### 2. Latency Analysis & Measured Architectural Differences

On a quiet, dedicated build node (`booster`, AMD Ryzen 9 7950X, 16 cores / 32 threads, 128 GB RAM), warmed in-memory query latencies at median (p50) are dynamically measured per capability: for Go to Definition, RIDX is **3.0x faster** (43.0 µs vs 131.0 µs); for Find References, RIDX is **16.4x faster** (93.5 µs vs 1534.0 µs); for Hover, RIDX is **1.5x faster** (68.0 µs vs 103.5 µs). The earlier 20–45x speedup figures were artifacts of a noisy test environment on `ram8`.

Measured architectural differences from the test run:
1. **Tail Latency**: `textDocument/definition` p90 is **26.7x faster** (204.3 µs vs 5449.9 µs / 5.45 ms).
2. **Multi-Token Reference Search**: `textDocument/references` is **16.4x faster at p50** (93.5 µs vs 1534.0 µs) and **20.7x faster at p90** (1183.4 µs vs 24493.8 µs).
3. **Process Resources**: RIDX maintains 2 OS threads and 3 open file descriptors, whereas rust-analyzer scales to 37 threads and 9 file descriptors.
4. **Memory Footprint**: RIDX uses **92.3x lower Heap RAM** (Anon Heap 14.0 MB vs 1292.2 MB).
5. **Multi-Worktree Scaling**: Across 10 concurrent worktrees, RIDX consumes **20.5 MB PSS** total (8.5 MB Anon) vs rust-analyzer's **9211.5 MB PSS** (9181.6 MB Anon) — a **449.3x lower PSS** footprint.
6. **Warm Incremental Re-index (W1)**: rust-analyzer reaches freshness in **150.2 ms**, which is **21.0x faster** than RIDX (**3160.3 ms**).

### 3. Semantic Parity & Discrepancies Breakdown

Across the sampled 300 read-only scenarios:
- **Class A Strict Matches**: 262/300 (87.3% [262 / 300 (87.3%, 95% CI [83.1%, 90.6%])])
- **Class A + Class C Semantic Parity**: 269/300 (89.7% [269 / 300 (89.7%, 95% CI [85.7%, 92.6%])])
- **Standard Library Sysroot Impact**: 31 of 38 non-matching cases (81.6%) target items in `std`/`core`/`alloc`. Expected effect of sysroot indexing: up to +10.33 percentage points strict-match (raising 87.3% to 97.7%).

The 38 non-matching cases were dynamically classified from telemetry:

- **Associated Type Bindings in Trait Bounds (18 cases)**: Identifiers in associated type equality constraints (`hir::TypeBinding`, e.g. `where V: Visitor<'de, Value = ()>`, `S::Error`, `Serializer<Ok = ()>`).

- **Class C: Trait Default Method vs Declaration (Semantically Equivalent) (7 cases)**: Same source file, default implementation body vs trait declaration header (e.g. `Error::invalid_type`). Valid location ('points to where defined'), NOT an error.

- **Missing in RIDX (6 cases)**: RIDX returned `Not found` while rust-analyzer found definition.

- **Missing in rust-analyzer (5 cases)**: rust-analyzer returned `null` / `Not found` while RIDX resolved target.

- **Divergent Target (2 cases)**: Different symbols or macro span offsets.

### 4. Live Mutation Benchmarks (Edit Loop & Overlay)
Evaluated under framed JSON-RPC with `textDocument/didChange` (Wilson 95% Confidence Intervals):

| Mutation Kind | `ridx lsp` Success (n/N) | `rust-analyzer` Success (n/N) | RIDX p50 | RA p50 | Architectural Note |
|---|---|---|---|---|---|
| `line_shift` | **30 / 30 (100.0%, 95% CI [88.6%, 100.0%])** | **30 / 30 (100.0%, 95% CI [88.6%, 100.0%])** | **0.1 ms** | **0.5 ms** | Overlay delta |
| `rename_stale` | **29 / 30 (96.7%, 95% CI [83.3%, 99.4%])** | **26 / 30 (86.7%, 95% CI [70.3%, 94.7%])** | **0.1 ms** | **0.3 ms** | Stale detection |
| `new_symbol` | **0 / 30 (0.0%, 95% CI [0.0%, 11.4%])** | **27 / 30 (90.0%, 95% CI [74.4%, 96.5%])** | **0.0 ms** | **0.2 ms** | Pre-save null by design |

### 5. Worktree Scenarios (W1, W2, W3)

**Scenario W1: Real LSP Commit Walk (99 commits)**
- **Immediate Stale (before re-index completes)**: RIDX = **35 / 853 (4.1%, 95% CI [3.0%, 5.7%])**, RA = **19 / 913 (2.1%, 95% CI [1.3%, 3.2%])**
- **Post-Reindex Stale (after re-index completes)**: RIDX = **0 / 853 (0.0%, 95% CI [0.0%, 0.4%])**, RA = **0 / 913 (0.0%, 95% CI [0.0%, 0.4%])**
- **Time to Freshness**: RIDX median = **3160.3 ms** (`ridx index`), RA quiescent median = **150.2 ms**

**Scenario W2: Large Git Jumps (10 iterations)**
- Total disk growth: **57.1 MB** (bounded CAS cache)

**Scenario W3: Multi-Worktree Concurrency & PSS Scaling**

> [!NOTE]
> Workload caveat: Each live server process was actively probed with 50 sequential definition requests (compared to 300 queries + live mutations in the single-process main run). Headlines and scaling comparisons use PSS multipliers (proportional share of physical RAM).

| Worktrees | `ridx` PSS (MB) | `ridx` Anon (MB) | RA PSS (MB) | RA Anon (MB) | CAS Delta (MB) | Cargo Check (ms) | CAS Index (ms) |
|---|---|---|---|---|---|---|---|
| **1** | **6.5** | **1.1** | **966.9** | **938.5** | **5.7** | 1235.7 | **4039.4** |
| **2** | **8.8** | **2.2** | **1905.0** | **1875.9** | **11.4** | 1273.4 | **4086.9** |
| **5** | **15.1** | **5.2** | **4595.5** | **4566.1** | **28.6** | 1371.3 | **4126.7** |
| **10** | **20.5** | **8.5** | **9211.5** | **9181.6** | **52.2** | 1233.7 | **3612.1** |

*CAS & Indexing Breakdown:*
- **CAS Growth (+5.2 MB per worktree)**: Represents first-party workspace crate objects compiled for differing source states across commit offsets (`HEAD~0`, `HEAD~5`, ...). Third-party dependencies are 100% deduplicated.
- **Index Duration**: Corresponds to cargo executing `ridx-driver` across all workspace crate targets. Once cached in CAS, index assembly and subsequent LSP lookups are sub-millisecond.


---

## Benchmark Report: `BurntSushi/ripgrep` (commit `3fce3b5b`)
*Source Data:* `bench/results/pilot_results.jsonl`  
*Run Status:* `SUCCESS`  
*Driver SHA256:* `ac554e8f3b302af637109a0679adf95aff0d8d35937d6f6b4e99770044b720c8`  
*rust-analyzer Version:* `rust-analyzer 1.98.1 (48a229c 2026-09-01)`  
*Target Node:* `booster` (AMD Ryzen 9 7950X, 16 cores / 32 threads, 128 GB RAM, NVMe SSD)

### 1. Headline Performance & Resource Footprint

| Metric | `ridx lsp` | `rust-analyzer` (Live) | Comparison / Notes |
|---|---|---|---|
| **Cold Clean Index / Check** | **3.610 s** (`ridx index`) | 1.2 s (`cargo check`) | 3.0x of cargo check |
| **Warm Incremental Re-index** | **1292.5 ms** (`ridx index`) | N/A | Cached CAS |
| **Peak Resident Set Size (RSS)** | **51.2 MB** (Anon: **18.4 MB**, File: 32.8 MB) | **1274.3 MB** (Anon: **1236.8 MB**, File: 37.5 MB) | **67.0x lower Heap RAM** |
| **Active OS Threads** | **2 threads** | **37 threads** | 18.5x fewer threads |
| **Open File Descriptors** | **3 total** (0 open `.ridx` FDs) | **9 total** | Fixed mmap file handles |
| **Def Latency (Warm p50)** | **50.0 µs** | **185.5 µs** | 3.7x faster |
| **Def Latency (Warm p90)** | **293.3 µs** | **5057.3 µs** | **17.2x faster tail latency** |
| **Hover Latency (Warm p50)** | **78.5 µs** | **132.5 µs** | 1.7x faster |
| **Refs Latency (Warm p50)** | **103.0 µs** | **1014.5 µs** | **9.8x faster** |
| **Refs Latency (Warm p90)** | **2988.6 µs** | **9693.1 µs** | **3.2x faster tail latency** |
| **Read-Only Strict Match Rate (Class A)** | **288 / 300 (96.0%, 95% CI [93.1%, 97.7%]) (96.00%)** | Baseline | Wilson 95% CI |
| **Read-Only Line Diagnostic Match (Class B)** | **288 / 300 (96.0%, 95% CI [93.1%, 97.7%]) (96.00%)** | Baseline | Same-line resolution |
| **Semantic Parity (Class A + Class C)** | **289 / 300 (96.3%, 95% CI [93.6%, 97.9%]) (96.33%)** | Baseline | Strict + Class C default method equiv |
| **Compiler ICE & Caught Panics** | **0 ICE, 0 caught panics** | 0 ICE | Driver telemetry verified |
| **cargo Spawned During Queries** | **ridx: false, ra: false** | **Quiescent** | Verified via process monitoring |

### 2. Latency Analysis & Measured Architectural Differences

On a quiet, dedicated build node (`booster`, AMD Ryzen 9 7950X, 16 cores / 32 threads, 128 GB RAM), warmed in-memory query latencies at median (p50) are dynamically measured per capability: for Go to Definition, RIDX is **3.7x faster** (50.0 µs vs 185.5 µs); for Find References, RIDX is **9.8x faster** (103.0 µs vs 1014.5 µs); for Hover, RIDX is **1.7x faster** (78.5 µs vs 132.5 µs). The earlier 20–45x speedup figures were artifacts of a noisy test environment on `ram8`.

Measured architectural differences from the test run:
1. **Tail Latency**: `textDocument/definition` p90 is **17.2x faster** (293.3 µs vs 5057.3 µs / 5.06 ms).
2. **Multi-Token Reference Search**: `textDocument/references` is **9.8x faster at p50** (103.0 µs vs 1014.5 µs) and **3.2x faster at p90** (2988.6 µs vs 9693.1 µs).
3. **Process Resources**: RIDX maintains 2 OS threads and 3 open file descriptors, whereas rust-analyzer scales to 37 threads and 9 file descriptors.
4. **Memory Footprint**: RIDX uses **67.0x lower Heap RAM** (Anon Heap 18.4 MB vs 1236.8 MB).
5. **Multi-Worktree Scaling**: Across 10 concurrent worktrees, RIDX consumes **26.2 MB PSS** total (13.5 MB Anon) vs rust-analyzer's **9526.1 MB PSS** (9491.6 MB Anon) — a **363.2x lower PSS** footprint.
6. **Warm Incremental Re-index (W1)**: rust-analyzer reaches freshness in **0.0 ms**, which is **32649.0x faster** than RIDX (**3264.9 ms**).

### 3. Semantic Parity & Discrepancies Breakdown

Across the sampled 300 read-only scenarios:
- **Class A Strict Matches**: 288/300 (96.0% [288 / 300 (96.0%, 95% CI [93.1%, 97.7%])])
- **Class A + Class C Semantic Parity**: 289/300 (96.3% [289 / 300 (96.3%, 95% CI [93.6%, 97.9%])])
- **Standard Library Sysroot Impact**: 5 of 12 non-matching cases (41.7%) target items in `std`/`core`/`alloc`. Expected effect of sysroot indexing: up to +1.67 percentage points strict-match (raising 96.0% to 97.7%).

The 12 non-matching cases were dynamically classified from telemetry:

- **Missing in rust-analyzer (6 cases)**: rust-analyzer returned `null` / `Not found` while RIDX resolved target.

- **Divergent Target (3 cases)**: Different symbols or macro span offsets.

- **Associated Type Bindings in Trait Bounds (1 cases)**: Identifiers in associated type equality constraints (`hir::TypeBinding`, e.g. `where V: Visitor<'de, Value = ()>`, `S::Error`, `Serializer<Ok = ()>`).

- **Class C: Trait Default Method vs Declaration (Semantically Equivalent) (1 cases)**: Same source file, default implementation body vs trait declaration header (e.g. `Error::invalid_type`). Valid location ('points to where defined'), NOT an error.

- **Missing in RIDX (1 cases)**: RIDX returned `Not found` while rust-analyzer found definition.

### 4. Live Mutation Benchmarks (Edit Loop & Overlay)
Evaluated under framed JSON-RPC with `textDocument/didChange` (Wilson 95% Confidence Intervals):

| Mutation Kind | `ridx lsp` Success (n/N) | `rust-analyzer` Success (n/N) | RIDX p50 | RA p50 | Architectural Note |
|---|---|---|---|---|---|
| `line_shift` | **30 / 30 (100.0%, 95% CI [88.6%, 100.0%])** | **29 / 30 (96.7%, 95% CI [83.3%, 99.4%])** | **0.1 ms** | **3.4 ms** | Overlay delta |
| `rename_stale` | **28 / 30 (93.3%, 95% CI [78.7%, 98.2%])** | **24 / 30 (80.0%, 95% CI [62.7%, 90.5%])** | **0.1 ms** | **3.1 ms** | Stale detection |
| `new_symbol` | **24 / 30 (80.0%, 95% CI [62.7%, 90.5%])** | **29 / 30 (96.7%, 95% CI [83.3%, 99.4%])** | **0.1 ms** | **4.8 ms** | Pre-save null by design |

### 5. Worktree Scenarios (W1, W2, W3)

**Scenario W1: Real LSP Commit Walk (99 commits)**
- **Immediate Stale (before re-index completes)**: RIDX = **671 / 719 (93.3%, 95% CI [91.3%, 94.9%])**, RA = **542 / 808 (67.1%, 95% CI [63.8%, 70.2%])**
- **Post-Reindex Stale (after re-index completes)**: RIDX = **0 / 719 (0.0%, 95% CI [0.0%, 0.5%])**, RA = **0 / 808 (0.0%, 95% CI [0.0%, 0.5%])**
- **Time to Freshness**: RIDX median = **3264.9 ms** (`ridx index`), RA quiescent median = **0.0 ms**

**Scenario W2: Large Git Jumps (10 iterations)**
- Total disk growth: **-0.1 MB** (bounded CAS cache)

**Scenario W3: Multi-Worktree Concurrency & PSS Scaling**

> [!NOTE]
> Workload caveat: Each live server process was actively probed with 50 sequential definition requests (compared to 300 queries + live mutations in the single-process main run). Headlines and scaling comparisons use PSS multipliers (proportional share of physical RAM).

| Worktrees | `ridx` PSS (MB) | `ridx` Anon (MB) | RA PSS (MB) | RA Anon (MB) | CAS Delta (MB) | Cargo Check (ms) | CAS Index (ms) |
|---|---|---|---|---|---|---|---|
| **1** | **8.1** | **1.4** | **997.0** | **964.2** | **0.0** | 1170.5 | **3648.2** |
| **2** | **10.7** | **2.8** | **1956.1** | **1923.0** | **2.6** | 2451.1 | **3647.6** |
| **5** | **16.4** | **6.8** | **4750.5** | **4716.5** | **10.2** | 1909.9 | **3662.7** |
| **10** | **26.2** | **13.5** | **9526.1** | **9491.6** | **23.0** | 1858.4 | **3571.5** |

*CAS & Indexing Breakdown:*
- **CAS Growth (+2.3 MB per worktree)**: Represents first-party workspace crate objects compiled for differing source states across commit offsets (`HEAD~0`, `HEAD~5`, ...). Third-party dependencies are 100% deduplicated.
- **Index Duration**: Corresponds to cargo executing `ridx-driver` across all workspace crate targets. Once cached in CAS, index assembly and subsequent LSP lookups are sub-millisecond.


---

## Benchmark Report: `tokio-rs/tokio` (commit `b2636752`)
*Source Data:* `bench/results/pilot_results.jsonl`  
*Run Status:* `SUCCESS`  
*Driver SHA256:* `ac554e8f3b302af637109a0679adf95aff0d8d35937d6f6b4e99770044b720c8`  
*rust-analyzer Version:* `rust-analyzer 1.98.1 (48a229c 2026-09-01)`  
*Target Node:* `booster` (AMD Ryzen 9 7950X, 16 cores / 32 threads, 128 GB RAM, NVMe SSD)

### 1. Headline Performance & Resource Footprint

| Metric | `ridx lsp` | `rust-analyzer` (Live) | Comparison / Notes |
|---|---|---|---|
| **Cold Clean Index / Check** | **12.018 s** (`ridx index`) | 2.8 s (`cargo check`) | 4.3x of cargo check |
| **Warm Incremental Re-index (W1)** | **60002.8 ms** (`ridx index`) | **50.1 ms** (RA quiescent) | 1197.7x slower |
| **Peak Resident Set Size (RSS)** | **103.4 MB** (Anon: **37.9 MB**, File: 68.0 MB) | **2642.1 MB** (Anon: **2604.1 MB**, File: 38.0 MB) | **68.8x lower Heap RAM** |
| **Active OS Threads** | **2 threads** | **37 threads** | 18.5x fewer threads |
| **Open File Descriptors** | **4 total** (0 open `.ridx` FDs) | **9 total** | Fixed mmap file handles |
| **Def Latency (Warm p50)** | **95.0 µs** | **181.0 µs** | 1.9x faster |
| **Def Latency (Warm p90)** | **172.1 µs** | **28592.0 µs** | **166.1x faster tail latency** |
| **Hover Latency (Warm p50)** | **1295.5 µs** | **149.5 µs** | 8.7x slower |
| **Hover (Post-844eac0, Warm p50)** | **16.0 µs** | **161.5 µs** | **10.1x faster** (n=100) |
| **Refs Latency (Warm p50)** | **573.0 µs** | **5079.5 µs** | **8.9x faster** |
| **Refs Latency (Warm p90)** | **2018.7 µs** | **59270.8 µs** | **29.4x faster tail latency** |
| **Read-Only Strict Match Rate (Class A)** | **245 / 300 (81.7%, 95% CI [76.9%, 85.6%]) (81.67%)** | Baseline | Wilson 95% CI |
| **Read-Only Line Diagnostic Match (Class B)** | **245 / 300 (81.7%, 95% CI [76.9%, 85.6%]) (81.67%)** | Baseline | Same-line resolution |
| **Semantic Parity (Class A + Class C)** | **246 / 300 (82.0%, 95% CI [77.3%, 85.9%]) (82.00%)** | Baseline | Strict + Class C default method equiv |
| **Compiler ICE & Caught Panics** | **0 ICE, 0 caught panics** | 0 ICE | Driver telemetry verified |
| **cargo Spawned During Queries** | **ridx: false, ra: false** | **Quiescent** | Verified via process monitoring |

### 2. Latency Analysis & Measured Architectural Differences

On a quiet, dedicated build node (`booster`, AMD Ryzen 9 7950X, 16 cores / 32 threads, 128 GB RAM), warmed in-memory query latencies at median (p50) are dynamically measured per capability: for Go to Definition, RIDX is **1.9x faster** (95.0 µs vs 181.0 µs); for Find References, RIDX is **8.9x faster** (573.0 µs vs 5079.5 µs); for Hover, RIDX is **8.7x slower** (1295.5 µs vs 149.5 µs). The earlier 20–45x speedup figures were artifacts of a noisy test environment on `ram8`.

Measured architectural differences from the test run:
1. **Tail Latency**: `textDocument/definition` p90 is **166.1x faster** (172.1 µs vs 28592.0 µs / 28.59 ms).
2. **Multi-Token Reference Search**: `textDocument/references` is **8.9x faster at p50** (573.0 µs vs 5079.5 µs) and **29.4x faster at p90** (2018.7 µs vs 59270.8 µs).
3. **Process Resources**: RIDX maintains 2 OS threads and 4 open file descriptors, whereas rust-analyzer scales to 37 threads and 9 file descriptors.
4. **Memory Footprint**: RIDX uses **68.8x lower Heap RAM** (Anon Heap 37.9 MB vs 2604.1 MB).
5. **Multi-Worktree Scaling**: Across 10 concurrent worktrees, RIDX consumes **196.5 MB PSS** total (149.1 MB Anon) vs rust-analyzer's **17528.4 MB PSS** (17493.9 MB Anon) — a **89.2x lower PSS** footprint.
6. **Warm Incremental Re-index (W1)**: rust-analyzer reaches freshness in **50.1 ms**, which is **1197.7x faster** than RIDX (**60002.8 ms**).
7. **Hover Optimization (Commit 844eac0)**: In a dedicated symmetric rerun on `booster` (taskset 24-31, n=100 hover queries alongside RA 1.98.1), warm p50 is **10.1x faster** (16.0 µs vs 161.5 µs).

### 3. Semantic Parity & Discrepancies Breakdown

Across the sampled 300 read-only scenarios:
- **Class A Strict Matches**: 245/300 (81.7% [245 / 300 (81.7%, 95% CI [76.9%, 85.6%])])
- **Class A + Class C Semantic Parity**: 246/300 (82.0% [246 / 300 (82.0%, 95% CI [77.3%, 85.9%])])
- **Standard Library Sysroot Impact**: 10 of 55 non-matching cases (18.2%) target items in `std`/`core`/`alloc`. Expected effect of sysroot indexing: up to +3.33 percentage points strict-match (raising 81.7% to 85.0%).

The 55 non-matching cases were dynamically classified from telemetry:

- **Missing in rust-analyzer (35 cases)**: rust-analyzer returned `null` / `Not found` while RIDX resolved target.

- **Missing in RIDX (10 cases)**: RIDX returned `Not found` while rust-analyzer found definition.

- **Divergent Target (9 cases)**: Different symbols or macro span offsets.

- **Class C: Trait Default Method vs Declaration (Semantically Equivalent) (1 cases)**: Same source file, default implementation body vs trait declaration header (e.g. `Error::invalid_type`). Valid location ('points to where defined'), NOT an error.

### 4. Live Mutation Benchmarks (Edit Loop & Overlay)
Evaluated under framed JSON-RPC with `textDocument/didChange` (Wilson 95% Confidence Intervals):

| Mutation Kind | `ridx lsp` Success (n/N) | `rust-analyzer` Success (n/N) | RIDX p50 | RA p50 | Architectural Note |
|---|---|---|---|---|---|
| `line_shift` | **24 / 30 (80.0%, 95% CI [62.7%, 90.5%])** | **21 / 30 (70.0%, 95% CI [52.1%, 83.3%])** | **0.1 ms** | **0.4 ms** | Overlay delta |
| `rename_stale` | **25 / 30 (83.3%, 95% CI [66.4%, 92.7%])** | **24 / 30 (80.0%, 95% CI [62.7%, 90.5%])** | **0.1 ms** | **26.4 ms** | Stale detection |
| `new_symbol` | **0 / 30 (0.0%, 95% CI [0.0%, 11.4%])** | **16 / 30 (53.3%, 95% CI [36.1%, 69.8%])** | **0.1 ms** | **27.0 ms** | Pre-save null by design |

### 5. Worktree Scenarios (W1, W2, W3)

**Scenario W1: Real LSP Commit Walk (99 commits)**
- **Immediate Stale (before re-index completes)**: RIDX = **501 / 637 (78.6%, 95% CI [75.3%, 81.7%])**, RA = **234 / 638 (36.7%, 95% CI [33.0%, 40.5%])**
- **Post-Reindex Stale (after re-index completes)**: RIDX = **0 / 637 (0.0%, 95% CI [0.0%, 0.6%])**, RA = **0 / 638 (0.0%, 95% CI [0.0%, 0.6%])**
- **Time to Freshness**: RIDX median = **60002.8 ms** (`ridx index`), RA quiescent median = **50.1 ms**

**Scenario W2: Large Git Jumps (0 iterations)**
- Total disk growth: **0.0 MB** (bounded CAS cache)

**Scenario W3: Multi-Worktree Concurrency & PSS Scaling**

> [!NOTE]
> Workload caveat: Each live server process was actively probed with 50 sequential definition requests (compared to 300 queries + live mutations in the single-process main run). Headlines and scaling comparisons use PSS multipliers (proportional share of physical RAM).

| Worktrees | `ridx` PSS (MB) | `ridx` Anon (MB) | RA PSS (MB) | RA Anon (MB) | CAS Delta (MB) | Cargo Check (ms) | CAS Index (ms) |
|---|---|---|---|---|---|---|---|
| **1** | **29.2** | **15.0** | **1784.3** | **1751.4** | **0.0** | 3059.0 | **12641.8** |
| **2** | **48.7** | **30.1** | **3503.1** | **3469.3** | **9.4** | 3055.0 | **12375.5** |
| **5** | **104.9** | **75.1** | **8735.4** | **8701.2** | **37.6** | 3071.2 | **13610.3** |
| **10** | **196.5** | **149.1** | **17528.4** | **17493.9** | **84.6** | 3036.6 | **13615.8** |

*CAS & Indexing Breakdown:*
- **CAS Growth (+8.5 MB per worktree)**: Represents first-party workspace crate objects compiled for differing source states across commit offsets (`HEAD~0`, `HEAD~5`, ...). Third-party dependencies are 100% deduplicated.
- **Index Duration**: Corresponds to cargo executing `ridx-driver` across all workspace crate targets. Once cached in CAS, index assembly and subsequent LSP lookups are sub-millisecond.


---

## Benchmark Report: `bevyengine/bevy` (commit `68c80f3b`)
*Source Data:* `bench/results/pilot_results.jsonl`  
*Run Status:* `SUCCESS`  
*Driver SHA256:* `ac554e8f3b302af637109a0679adf95aff0d8d35937d6f6b4e99770044b720c8`  
*rust-analyzer Version:* `rust-analyzer 1.98.1 (48a229c 2026-09-01)`  
*Target Node:* `booster` (AMD Ryzen 9 7950X, 16 cores / 32 threads, 128 GB RAM, NVMe SSD)

### 1. Headline Performance & Resource Footprint

| Metric | `ridx lsp` | `rust-analyzer` (Live) | Comparison / Notes |
|---|---|---|---|
| **Cold Clean Index / Check** | **67.609 s** (`ridx index`) | 34.8 s (`cargo check`) | 1.9x of cargo check |
| **Warm Incremental Re-index (W1)** | **39531.2 ms** (`ridx index`) | **550.9 ms** (RA quiescent) | 71.8x slower |
| **Peak Resident Set Size (RSS)** | **536.3 MB** (Anon: **178.0 MB**, File: 358.3 MB) | **10304.9 MB** (Anon: **10266.9 MB**, File: 38.0 MB) | **57.7x lower Heap RAM** |
| **Active OS Threads** | **2 threads** | **37 threads** | 18.5x fewer threads |
| **Open File Descriptors** | **4 total** (0 open `.ridx` FDs) | **9 total** | Fixed mmap file handles |
| **Def Latency (Warm p50)** | **245.0 µs** | **280.5 µs** | on par (±20%) |
| **Def Latency (Warm p90)** | **504.3 µs** | **125641.2 µs** | **249.1x faster tail latency** |
| **Hover Latency (Warm p50)** | **3957.5 µs** | **215.0 µs** | 18.4x slower |
| **Hover (Post-844eac0, Warm p50)** | **353.5 µs** | **230.5 µs** | **1.5x slower** (n=100) |
| **Refs Latency (Warm p50)** | **3880.0 µs** | **13022.0 µs** | **3.4x faster** |
| **Refs Latency (Warm p90)** | **7931.9 µs** | **117158.9 µs** | **14.8x faster tail latency** |
| **Read-Only Strict Match Rate (Class A)** | **281 / 300 (93.7%, 95% CI [90.3%, 95.9%]) (93.67%)** | Baseline | Wilson 95% CI |
| **Read-Only Line Diagnostic Match (Class B)** | **281 / 300 (93.7%, 95% CI [90.3%, 95.9%]) (93.67%)** | Baseline | Same-line resolution |
| **Semantic Parity (Class A + Class C)** | **281 / 300 (93.7%, 95% CI [90.3%, 95.9%]) (93.67%)** | Baseline | Strict + Class C default method equiv |
| **Compiler ICE & Caught Panics** | **0 ICE, 0 caught panics** | 0 ICE | Driver telemetry verified |
| **cargo Spawned During Queries** | **ridx: false, ra: false** | **Quiescent** | Verified via process monitoring |

### 2. Latency Analysis & Measured Architectural Differences

On a quiet, dedicated build node (`booster`, AMD Ryzen 9 7950X, 16 cores / 32 threads, 128 GB RAM), warmed in-memory query latencies at median (p50) are dynamically measured per capability: for Go to Definition, RIDX is **on par (±20%)** (245.0 µs vs 280.5 µs); for Find References, RIDX is **3.4x faster** (3880.0 µs vs 13022.0 µs); for Hover, RIDX is **18.4x slower** (3957.5 µs vs 215.0 µs). The earlier 20–45x speedup figures were artifacts of a noisy test environment on `ram8`.

Measured architectural differences from the test run:
1. **Tail Latency**: `textDocument/definition` p90 is **249.1x faster** (504.3 µs vs 125641.2 µs / 125.64 ms).
2. **Multi-Token Reference Search**: `textDocument/references` is **3.4x faster at p50** (3880.0 µs vs 13022.0 µs) and **14.8x faster at p90** (7931.9 µs vs 117158.9 µs).
3. **Process Resources**: RIDX maintains 2 OS threads and 4 open file descriptors, whereas rust-analyzer scales to 37 threads and 9 file descriptors.
4. **Memory Footprint**: RIDX uses **57.7x lower Heap RAM** (Anon Heap 178.0 MB vs 10266.9 MB).
5. **Multi-Worktree Scaling**: Across 10 concurrent worktrees, RIDX consumes **1725.3 MB PSS** total (1583.5 MB Anon) vs rust-analyzer's **58549.2 MB PSS** (58522.7 MB Anon) — a **33.9x lower PSS** footprint.
6. **Warm Incremental Re-index (W1)**: rust-analyzer reaches freshness in **550.9 ms**, which is **71.8x faster** than RIDX (**39531.2 ms**).
7. **Hover Optimization (Commit 844eac0)**: In a dedicated symmetric rerun on `booster` (taskset 24-31, n=100 hover queries alongside RA 1.98.1), warm p50 is **1.5x slower** (353.5 µs vs 230.5 µs).

### 3. Semantic Parity & Discrepancies Breakdown

Across the sampled 300 read-only scenarios:
- **Class A Strict Matches**: 281/300 (93.7% [281 / 300 (93.7%, 95% CI [90.3%, 95.9%])])
- **Class A + Class C Semantic Parity**: 281/300 (93.7% [281 / 300 (93.7%, 95% CI [90.3%, 95.9%])])
- **Standard Library Sysroot Impact**: 5 of 19 non-matching cases (26.3%) target items in `std`/`core`/`alloc`. Expected effect of sysroot indexing: up to +1.67 percentage points strict-match (raising 93.7% to 95.3%).

The 19 non-matching cases were dynamically classified from telemetry:

- **Missing in RIDX (11 cases)**: RIDX returned `Not found` while rust-analyzer found definition.

- **Divergent Target (6 cases)**: Different symbols or macro span offsets.

- **Associated Type Bindings in Trait Bounds (1 cases)**: Identifiers in associated type equality constraints (`hir::TypeBinding`, e.g. `where V: Visitor<'de, Value = ()>`, `S::Error`, `Serializer<Ok = ()>`).

- **Missing in rust-analyzer (1 cases)**: rust-analyzer returned `null` / `Not found` while RIDX resolved target.

### 4. Live Mutation Benchmarks (Edit Loop & Overlay)
Evaluated under framed JSON-RPC with `textDocument/didChange` (Wilson 95% Confidence Intervals):

| Mutation Kind | `ridx lsp` Success (n/N) | `rust-analyzer` Success (n/N) | RIDX p50 | RA p50 | Architectural Note |
|---|---|---|---|---|---|
| `line_shift` | **30 / 30 (100.0%, 95% CI [88.6%, 100.0%])** | **30 / 30 (100.0%, 95% CI [88.6%, 100.0%])** | **0.3 ms** | **50.6 ms** | Overlay delta |
| `rename_stale` | **26 / 30 (86.7%, 95% CI [70.3%, 94.7%])** | **26 / 30 (86.7%, 95% CI [70.3%, 94.7%])** | **0.4 ms** | **0.2 ms** | Stale detection |
| `new_symbol` | **4 / 30 (13.3%, 95% CI [5.3%, 29.7%])** | **30 / 30 (100.0%, 95% CI [88.6%, 100.0%])** | **0.2 ms** | **112.5 ms** | Pre-save null by design |

### 5. Worktree Scenarios (W1, W2, W3)

**Scenario W1: Real LSP Commit Walk (99 commits)**
- **Immediate Stale (before re-index completes)**: RIDX = **1304 / 1432 (91.1%, 95% CI [89.5%, 92.4%])**, RA = **1561 / 3517 (44.4%, 95% CI [42.7%, 46.0%])**
- **Post-Reindex Stale (after re-index completes)**: RIDX = **0 / 1432 (0.0%, 95% CI [0.0%, 0.3%])**, RA = **0 / 3517 (0.0%, 95% CI [0.0%, 0.1%])**
- **Time to Freshness**: RIDX median = **39531.2 ms** (`ridx index`), RA quiescent median = **550.9 ms**

**Scenario W2: Large Git Jumps (10 iterations)**
- Total disk growth: **788.1 MB** (bounded CAS cache)

**Scenario W3: Multi-Worktree Concurrency & PSS Scaling**

> [!NOTE]
> Workload caveat: Each live server process was actively probed with 50 sequential definition requests (compared to 300 queries + live mutations in the single-process main run). Headlines and scaling comparisons use PSS multipliers (proportional share of physical RAM).

| Worktrees | `ridx` PSS (MB) | `ridx` Anon (MB) | RA PSS (MB) | RA Anon (MB) | CAS Delta (MB) | Cargo Check (ms) | CAS Index (ms) |
|---|---|---|---|---|---|---|---|
| **1** | **216.5** | **176.1** | **5777.6** | **5752.0** | **216.3** | 36619.6 | **68340.6** |
| **2** | **409.8** | **352.1** | **11759.4** | **11733.4** | **432.6** | 36039.3 | **66930.4** |
| **5** | **789.8** | **704.5** | **29266.8** | **29240.5** | **1070.1** | 42418.5 | **66423.8** |
| **10** | **1725.3** | **1583.5** | **58549.2** | **58522.7** | **2132.7** | 35604.1 | **67198.0** |

*CAS & Indexing Breakdown:*
- **CAS Growth (+213.3 MB per worktree)**: Represents first-party workspace crate objects compiled for differing source states across commit offsets (`HEAD~0`, `HEAD~5`, ...). Third-party dependencies are 100% deduplicated.
- **Index Duration**: Corresponds to cargo executing `ridx-driver` across all workspace crate targets. Once cached in CAS, index assembly and subsequent LSP lookups are sub-millisecond.


---

## Benchmark Report: `rust-lang/rust-analyzer` (commit `021bfc15`)
*Source Data:* `bench/results/pilot_results.jsonl`  
*Run Status:* `SUCCESS`  
*Driver SHA256:* `3954bd16bf13c8eb9566d25f1f5de24af710abf7001687c7f47088e1931a5dbf`  
*rust-analyzer Version:* `rust-analyzer 1.98.1 (48a229c 2026-09-01)`  
*Target Node:* `booster` (AMD Ryzen 9 7950X, 16 cores / 32 threads, 128 GB RAM, NVMe SSD)

### 1. Headline Performance & Resource Footprint

| Metric | `ridx lsp` | `rust-analyzer` (Live) | Comparison / Notes |
|---|---|---|---|
| **Cold Clean Index / Check** | **29.268 s** (`ridx index`) | 20.8 s (`cargo check`) | 1.4x of cargo check |
| **Warm Incremental Re-index (W1)** | **6461.5 ms** (`ridx index`) | **100.1 ms** (RA quiescent) | 64.6x slower |
| **Peak Resident Set Size (RSS)** | **272.6 MB** (Anon: **78.6 MB**, File: 196.4 MB) | **3806.8 MB** (Anon: **3766.8 MB**, File: 40.2 MB) | **47.9x lower Heap RAM** |
| **Active OS Threads** | **2 threads** | **37 threads** | 18.5x fewer threads |
| **Open File Descriptors** | **4 total** (0 open `.ridx` FDs) | **9 total** | Fixed mmap file handles |
| **Def Latency (Warm p50)** | **174.0 µs** | **314.5 µs** | 1.8x faster |
| **Def Latency (Warm p90)** | **394.1 µs** | **51288.0 µs** | **130.1x faster tail latency** |
| **Hover Latency (Warm p50)** | **436.0 µs** | **228.0 µs** | 1.9x slower |
| **Refs Latency (Warm p50)** | **654.5 µs** | **1571.5 µs** | **2.4x faster** |
| **Refs Latency (Warm p90)** | **2265.9 µs** | **110309.6 µs** | **48.7x faster tail latency** |
| **Read-Only Strict Match Rate (Class A)** | **242 / 300 (80.7%, 95% CI [75.8%, 84.7%]) (80.67%)** | Baseline | Wilson 95% CI |
| **Read-Only Line Diagnostic Match (Class B)** | **242 / 300 (80.7%, 95% CI [75.8%, 84.7%]) (80.67%)** | Baseline | Same-line resolution |
| **Semantic Parity (Class A + Class C)** | **249 / 300 (83.0%, 95% CI [78.3%, 86.8%]) (83.00%)** | Baseline | Strict + Class C default method equiv |
| **Compiler ICE & Caught Panics** | **0 ICE, 0 caught panics** | 0 ICE | Driver telemetry verified |
| **cargo Spawned During Queries** | **ridx: false, ra: false** | **Quiescent** | Verified via process monitoring |

### 2. Latency Analysis & Measured Architectural Differences

On a quiet, dedicated build node (`booster`, AMD Ryzen 9 7950X, 16 cores / 32 threads, 128 GB RAM), warmed in-memory query latencies at median (p50) are dynamically measured per capability: for Go to Definition, RIDX is **1.8x faster** (174.0 µs vs 314.5 µs); for Find References, RIDX is **2.4x faster** (654.5 µs vs 1571.5 µs); for Hover, RIDX is **1.9x slower** (436.0 µs vs 228.0 µs). The earlier 20–45x speedup figures were artifacts of a noisy test environment on `ram8`.

Measured architectural differences from the test run:
1. **Tail Latency**: `textDocument/definition` p90 is **130.1x faster** (394.1 µs vs 51288.0 µs / 51.29 ms).
2. **Multi-Token Reference Search**: `textDocument/references` is **2.4x faster at p50** (654.5 µs vs 1571.5 µs) and **48.7x faster at p90** (2265.9 µs vs 110309.6 µs).
3. **Process Resources**: RIDX maintains 2 OS threads and 4 open file descriptors, whereas rust-analyzer scales to 37 threads and 9 file descriptors.
4. **Memory Footprint**: RIDX uses **47.9x lower Heap RAM** (Anon Heap 78.6 MB vs 3766.8 MB).
5. **Multi-Worktree Scaling**: Across 10 concurrent worktrees, RIDX consumes **122.2 MB PSS** total (59.4 MB Anon) vs rust-analyzer's **21120.2 MB PSS** (21082.4 MB Anon) — a **172.8x lower PSS** footprint.
6. **Warm Incremental Re-index (W1)**: rust-analyzer reaches freshness in **100.1 ms**, which is **64.6x faster** than RIDX (**6461.5 ms**).

### 3. Semantic Parity & Discrepancies Breakdown

Across the sampled 300 read-only scenarios:
- **Class A Strict Matches**: 242/300 (80.7% [242 / 300 (80.7%, 95% CI [75.8%, 84.7%])])
- **Class A + Class C Semantic Parity**: 249/300 (83.0% [249 / 300 (83.0%, 95% CI [78.3%, 86.8%])])
- **Standard Library Sysroot Impact**: 22 of 58 non-matching cases (37.9%) target items in `std`/`core`/`alloc`. Expected effect of sysroot indexing: up to +7.33 percentage points strict-match (raising 80.7% to 88.0%).

The 58 non-matching cases were dynamically classified from telemetry:

- **Divergent Target (24 cases)**: Different symbols or macro span offsets.

- **Missing in rust-analyzer (19 cases)**: rust-analyzer returned `null` / `Not found` while RIDX resolved target.

- **Class C: Trait Default Method vs Declaration (Semantically Equivalent) (7 cases)**: Same source file, default implementation body vs trait declaration header (e.g. `Error::invalid_type`). Valid location ('points to where defined'), NOT an error.

- **Missing in RIDX (7 cases)**: RIDX returned `Not found` while rust-analyzer found definition.

- **Associated Type Bindings in Trait Bounds (1 cases)**: Identifiers in associated type equality constraints (`hir::TypeBinding`, e.g. `where V: Visitor<'de, Value = ()>`, `S::Error`, `Serializer<Ok = ()>`).

### 4. Live Mutation Benchmarks (Edit Loop & Overlay)
Evaluated under framed JSON-RPC with `textDocument/didChange` (Wilson 95% Confidence Intervals):

| Mutation Kind | `ridx lsp` Success (n/N) | `rust-analyzer` Success (n/N) | RIDX p50 | RA p50 | Architectural Note |
|---|---|---|---|---|---|
| `line_shift` | **30 / 30 (100.0%, 95% CI [88.6%, 100.0%])** | **30 / 30 (100.0%, 95% CI [88.6%, 100.0%])** | **0.2 ms** | **46.0 ms** | Overlay delta |
| `rename_stale` | **24 / 30 (80.0%, 95% CI [62.7%, 90.5%])** | **20 / 30 (66.7%, 95% CI [48.8%, 80.8%])** | **0.3 ms** | **47.7 ms** | Stale detection |
| `new_symbol` | **2 / 30 (6.7%, 95% CI [1.8%, 21.3%])** | **29 / 30 (96.7%, 95% CI [83.3%, 99.4%])** | **0.2 ms** | **46.9 ms** | Pre-save null by design |

### 5. Worktree Scenarios (W1, W2, W3)

**Scenario W1: Real LSP Commit Walk (99 commits)**
- **Immediate Stale (before re-index completes)**: RIDX = **357 / 471 (75.8%, 95% CI [71.7%, 79.4%])**, RA = **33 / 581 (5.7%, 95% CI [4.1%, 7.9%])**
- **Post-Reindex Stale (after re-index completes)**: RIDX = **0 / 471 (0.0%, 95% CI [0.0%, 0.8%])**, RA = **0 / 581 (0.0%, 95% CI [0.0%, 0.7%])**
- **Time to Freshness**: RIDX median = **6461.5 ms** (`ridx index`), RA quiescent median = **100.1 ms**

**Scenario W2: Large Git Jumps (10 iterations)**
- Total disk growth: **-0.1 MB** (bounded CAS cache)

**Scenario W3: Multi-Worktree Concurrency & PSS Scaling**

> [!NOTE]
> Workload caveat: Each live server process was actively probed with 50 sequential definition requests (compared to 300 queries + live mutations in the single-process main run). Headlines and scaling comparisons use PSS multipliers (proportional share of physical RAM).

| Worktrees | `ridx` PSS (MB) | `ridx` Anon (MB) | RA PSS (MB) | RA Anon (MB) | CAS Delta (MB) | Cargo Check (ms) | CAS Index (ms) |
|---|---|---|---|---|---|---|---|
| **1** | **23.5** | **5.9** | **2171.4** | **2134.9** | **56.4** | 129.5 | **30089.7** |
| **2** | **35.1** | **11.9** | **4233.8** | **4196.6** | **112.9** | 121.4 | **29661.5** |
| **5** | **68.2** | **29.6** | **10569.9** | **10532.2** | **282.2** | 127.8 | **29518.8** |
| **10** | **122.2** | **59.4** | **21120.2** | **21082.4** | **564.4** | 127.9 | **29199.7** |

*CAS & Indexing Breakdown:*
- **CAS Growth (+56.4 MB per worktree)**: Represents first-party workspace crate objects compiled for differing source states across commit offsets (`HEAD~0`, `HEAD~5`, ...). Third-party dependencies are 100% deduplicated.
- **Index Duration**: Corresponds to cargo executing `ridx-driver` across all workspace crate targets. Once cached in CAS, index assembly and subsequent LSP lookups are sub-millisecond.

---

## 6. Scoped Completion Telemetry (Stage 2: Owner-Index & Autoderef)

Commit `8fa53fa` routed dot- (`receiver.`) and colon- (`path::`) completion requests through the dedicated format v3 `defs_by_owner` index and autoderef chain (`derefs`), bypassing full-workspace symbol scanning. Commit `26a4ddb` subsequently eliminated quadratic fallback symbol cloning via streaming zero-allocation traversal and crate-scoped module index lookup.

Evaluated against `rust-analyzer 1.98.1 (48a229c)` on `booster` (100 sampled scenarios: 50 dot, 50 colon per repository; published in `bench/published/rerun_completion_8fa53fa.jsonl`, `bench/published/rerun_completion_26a4ddb.jsonl`, and `bench/published/completion_regression_8fa53fa_vs_26a4ddb.json`):

### Latency Breakdown: Matching Owner-Path vs Fallback Subsets (`26a4ddb`)

| Repository | Matching Owner Positions (n) | Owner RIDX warm p50 | RA warm p50 (matching owner) | Fair Owner Speedup | Fallback Share % | Fallback RIDX warm p50 | RA warm p50 (matching fallback) | Overall RIDX warm p50 | Overall RA warm p50 | Overall Speedup | Recall vs RA 1.98.1 |
|---|---|---|---|---|---|---|---|---|---|---|---|
| **`bevyengine/bevy`** (`bevy_ecs`) | 32 (32%) | **179.0 µs** | **6,457.5 µs** | **36.1x** | **68.0%** (n=68) | **11,099.5 µs** | **2,492.0 µs** | **3,154.5 µs** | **4,470.5 µs** (4.22 ms mean) | **1.34x – 1.42x** | **23.8%** |
| **`serde-rs/serde`** | 5 (5%) | **49.0 µs** | **7,023.0 µs** | **143.3x** | **95.0%** (n=95) | **809.0 µs** | **4,345.0 µs** | **800.0 µs** | **4,348.0 µs** | **5.43x** | **7.3%** |

*(Baseline `8fa53fa` reference: Bevy owner 186.5 µs [35.8x], fallback 44,390.5 µs [18.5x slower]; Serde owner 100.0 µs [64.8x], fallback 2,522.0 µs [1.7x faster].)*

### Empirical Regression Check: `8fa53fa` vs `26a4ddb` (100+100 Identical Positions)

| Repository | `8fa53fa` Empty Responses | `26a4ddb` Empty Responses | Regressions to Empty | Total Items (`8fa53fa` -> `26a4ddb`) | Recall vs RA 1.98.1 (`8fa53fa` -> `26a4ddb`) | Fallback Speedup (`8fa53fa` -> `26a4ddb`) | Overall Speedup (`8fa53fa` -> `26a4ddb`) |
|---|---|---|---|---|---|---|---|
| **`serde-rs/serde`** | 82 / 100 | **82 / 100** | **0** | 656 -> 656 | 7.3% -> **7.3%** | **2.2x** (1.80 ms -> 0.81 ms) | **2.2x** (1.79 ms -> 0.80 ms) |
| **`bevyengine/bevy`** | 33 / 100 | **33 / 100** | **0** | 3,739 -> 3,739 | 23.8% -> **23.8%** | **4.1x** (45.7 ms -> 11.1 ms) | **13.2x** (41.8 ms -> 3.15 ms) |

*Historical Baseline Check (`bb4aa75` -> `8fa53fa`):* Zero regressions to empty responses, with empty responses dropping 84 -> 82 on Serde and 43 -> 33 on Bevy, resolving 12 previously empty scenarios while narrowing blind prefix candidate spam.

### Stage 3.1: Generic Trait Bounds via `predicates_of` (`b05240a`)

Commit `b05240a` resolved generic parameters (`T: Trait`, e.g., `visitor.`, `serializer.`, `seed.`) to primary domain traits via `param_env.caller_bounds()` and indexed supertrait inheritance into `.ridx` autoderef chains. Published in `bench/published/rerun_completion_b05240a.jsonl` and `bench/published/completion_regression_26a4ddb_vs_b05240a.json`:

| Repository | `26a4ddb` Empty Responses | `b05240a` Empty Responses | Closed Empty Responses | Regressions to Empty | Total Items (`26a4ddb` -> `b05240a`) | Recall vs RA 1.98.1 (`26a4ddb` -> `b05240a`) | Owner Share (`26a4ddb` -> `b05240a`) | Overall Speedup vs RA 1.98.1 |
|---|---|---|---|---|---|---|---|---|
| **`serde-rs/serde`** | 82 / 100 | **62 / 100** | **20 / 82** (-24.4%) | **0** | 656 -> **1,158** (+76.5%) | 7.3% -> **17.2%** (+9.9%) | 5.0% -> **25.0%** (5x boost) | **5.82x** (750 µs vs 4.37 ms) |
| **`bevyengine/bevy`** | 33 / 100 | **33 / 100** | 0 / 33 (sysroot std) | **0** | 3,739 -> 3,739 (100%) | 23.8% -> 23.8% (100%) | 32.0% -> 32.0% | **1.34x** (3.33 ms vs 4.46 ms) |

### Key Architectural Findings:
1. **Ultra-Fast Owner-Path Execution**: On concrete domain types (`World`, `Schedule`, `Entity`, `Commands`, `QueryState` in Bevy; `Visitor`, `ContentVisitor`, `Deserializer` in Serde), the owner path completes in **49–179 µs** warm p50, delivering a **36.1x – 143.3x speedup** over rust-analyzer on those exact matching positions.
2. **Honest Scope on Fallback & Overall Performance**: Following streaming zero-allocation traversal in commit `26a4ddb`, fallback completion on large workspaces (`bevy`) improved from 44.4 ms (~18x slower) to **11.1 ms vs 2.4 ms RA (~4.6x slower)** on that subset. When combined with the ultra-fast owner path, overall completion across all 100 positions on `bevy` is now **1.34x faster than rust-analyzer (3.15 ms vs 4.22 ms)**, and on `serde` is **5.43x – 5.82x faster (750–800 µs vs 4.35 ms)**.
3. **Dramatic Generic Recall Boost without Sysroot**: In commit `b05240a`, resolving generic bounds via `param_env.caller_bounds()` closed 20 previously empty scenarios on Serde (`visitor.visit_u16`, `serializer.serialize_u8`, `seq_access.next_element`, `map_access.next_key`), more than doubling Serde recall from 7.3% to 17.2% and expanding owner-path routing from 5% to 25% with zero regressions. The remaining 62 empty Serde and 33 empty Bevy scenarios are standard library sysroot types (`Option`, `Result`, `Iterator`, `Formatter`, `Path`), reserved for Stage 3.2 sysroot indexing.
4. **Strict Zero-Regression Guarantee**: Across all transitions (`bb4aa75` -> `8fa53fa` -> `26a4ddb` -> `b05240a`), RIDX maintains exactly 0 regressions from non-empty to empty responses across all 200 scenarios.

---

## 7. Stage 3.2.1: Sysroot Metadata Indexing & Parity Verification (`c308792` vs `b05240a`)

*Target Node:* `booster` (AMD Ryzen 9 7950X, 16 cores / 32 threads, 128 GB RAM)  
*Toolchain Parity:* `rustc 1.98.1 (48a229cea 2026-09-01)` matching `rust-analyzer 1.98.1 (48a229c 2026-09-01)`  
*Sysroot CAS Key:* `ad5e1980a489d231` (deterministic shared cache for `alloc.ridx`, `core.ridx`, `std.ridx`)  
*Evaluation:* 200 total scenarios across `serde-rs/serde` (n=100) and `bevyengine/bevy` (n=100)  
*Published Reports:*
- Grounded v3 Report: `bench/published/completion_regression_b05240a_vs_c308792_v3.json`
- Historical Reports: `bench/published/completion_regression_b05240a_vs_c308792_v2.json` and `completion_regression_b05240a_vs_c308792.json`
- Raw Telemetry: `bench/published/rerun_completion_c308792.jsonl` and `rerun_completion_b05240a.jsonl`

### Headline Comparison Table (`b05240a` -> `c308792`)

| Metric | `serde-rs/serde` (`b05240a` → `c308792`) | `bevyengine/bevy` (`b05240a` → `c308792`) | Combined Total |
|---|---|---|---|
| **Empty Responses** | **62 → 34** (28 closed) | **33 → 16** (17 closed) | **95 → 50** (**45 closed**) |
| **Regressions to Empty** | **0** | **0** | **0** |
| **Total Candidates Returned** | 1,158 → 2,293 (RA: 4,675) | 3,739 → 4,657 (RA: 6,832) | 4,897 → 6,950 (RA: 11,507) |
| **Grounded Recall vs RA** (`ra_count > 0`) | **16.5% → 31.1%** (+14.6 pp, n=98) | **22.1% → 32.2%** (+10.1 pp, n=94) | **19.2% → 31.6%** (+12.4 pp, n=192) |
| *Undefined Oracle Scenarios (`ra_count == 0`)* | 2 excluded (`ra_zero_oracle`) | 6 excluded (`ra_zero_oracle`) | 8 excluded overall |
| **Grounded Precision vs RA** (`ridx_count > 0`, `ra_count > 0`) | **80.6% → 77.7%** (-2.9 pp, n: 37 → 65) | **60.9% → 56.2%** (-4.7 pp, n: 64 → 80) | **68.1% → 65.8%** (-2.3 pp, n: 101 → 145) |
| *Precision Excluded Scenarios (`ra_count == 0`)* | 1 in base, 1 in curr | 3 in base, 4 in curr | 4 in base, 5 in curr |
| **Owner-Path Share** | 25% → **48%** | 32% → **48%** | **48% overall** |
| **Owner-Path Speedup (p50 / p95)** | **28.77x** (214 µs vs 6,143 µs) / **17.80x** (714 µs vs 12.71 ms) | **17.97x** (407 µs vs 7,314 µs) / **18.35x** (1.06 ms vs 19.49 ms) | **18x – 29x** (p50) / **18x** (p95) |
| **Fallback Speedup (p50 / p95)** | **1.21x** (1,486 µs vs 1,796 µs) / **3.64x** (2.05 ms vs 7.47 ms) | **1.14x** (1,591 µs vs 1,819 µs) / **3.17x slower** (20.39 ms vs 6.43 ms) | **1.14x – 1.21x** (p50) / tail varies by dependency count |
| **Overall Speedup (p50 / p95)** | **6.64x** (662 µs vs 4,398 µs) / **6.06x** (2.01 ms vs 12.15 ms) | **10.33x** (420 µs vs 4,340 µs) / **1.19x slower** (20.30 ms vs 17.08 ms) | **6.6x – 10.3x** (p50) / **1.19x slower – 6.1x faster** (p95) |

### Detailed Latency Breakdown (p50 and p95)

```
┌─────────────────────────────────────────────────────────────────────────────────────────────┐
│                             RIDX vs rust-analyzer 1.98.1 Latency                            │
├──────────────────────┬───────────────────────────────┬──────────────────────────────────────┤
│ Route                │ serde-rs/serde (p50 / p95)    │ bevyengine/bevy (p50 / p95)          │
├──────────────────────┼───────────────────────────────┼──────────────────────────────────────┤
│ Owner Path (48%)     │ RIDX:   214 µs /   714 µs     │ RIDX:   407 µs /  1.06 ms            │
│                      │ RA:   6,143 µs / 12.71 ms     │ RA:   7,314 µs / 19.49 ms            │
│                      │ Speedup: 28.77x (p50) / 17.8x │ Speedup: 17.97x (p50) / 18.35x (p95) │
├──────────────────────┼───────────────────────────────┼──────────────────────────────────────┤
│ Fallback Path (52%)  │ RIDX: 1,486 µs /  2.05 ms     │ RIDX: 1,591 µs / 20.39 ms            │
│                      │ RA:   1,796 µs /  7.47 ms     │ RA:   1,819 µs /  6.43 ms            │
│                      │ Speedup:  1.21x (p50) / 3.64x │ Speedup:  1.14x (p50) / 3.17x slower │
├──────────────────────┼───────────────────────────────┼──────────────────────────────────────┤
│ Overall Combined     │ RIDX:   662 µs /  2.01 ms     │ RIDX:   420 µs / 20.30 ms            │
│                      │ RA:   4,398 µs / 12.15 ms     │ RA:   4,340 µs / 17.08 ms            │
│                      │ Speedup:  6.64x (p50) / 6.06x │ Speedup: 10.33x (p50) / 1.19x slower │
└──────────────────────┴───────────────────────────────┴──────────────────────────────────────┘
```

### Explanatory Findings & Root Causes

#### 1. Precision & Recall Grounding
Evaluating precision and recall strictly over grounded non-empty sets where `ra_count > 0`:
- **Serde Recall**: `b05240a` = 16.5% (n=98) → `c308792` = 31.1% (n=98), +14.6 pp. (2 scenarios excluded where `ra_count == 0`).
- **Bevy Recall**: `b05240a` = 22.1% (n=94) → `c308792` = 32.2% (n=94), +10.1 pp. (6 scenarios excluded where `ra_count == 0`).
- **Serde Precision**: `b05240a` = 80.6% (n=37) → `c308792` = 77.7% (n=65), -2.9 pp.
- **Bevy Precision**: `b05240a` = 60.9% (n=64) → `c308792` = 56.2% (n=80), -4.7 pp.
- **Empty Responses**: reduced from 95 to 50 (28 closed in Serde, 17 closed in Bevy, 0 regressions to empty).

#### 2. Root Cause of Recall Regressions (> 5.0 pp)
Across all 200 scenarios, exactly 2 experienced a recall drop > 5.0 pp (both in Serde, 0 in Bevy):
1. **`comp_colon_41_RangeVisitor` (`serde_core/src/de/impls.rs:2434:20`)**:
   - *Observation*: Recall dropped from 100.0% (33 items) to 0.0% (5 items, RA returned 2).
   - *Root Cause*: The cursor qualifier was `range`. In `b05240a` (no sysroot), `find_definition_by_path("range")` failed, falling back to workspace symbol search which located `RangeVisitor` and `FIELDS`. In `c308792`, `core.ridx` was attached; `find_definition_by_path("range")` matched `core::ops::range` before checking the local submodule `mod range { ... }` defined at line 2463. Owner completion returned 5 sysroot range items (`legacy`, `Range`, `RangeInclusive`, `RangeFrom`, `RangeToInclusive`), shadowing local submodule items.
2. **`comp_colon_37_SocketAddr` (`serde_core/src/ser/impls.rs:853:22`)**:
   - *Observation*: Recall dropped from 15.4% (24 items) to 7.7% (1 item, RA returned 13).
   - *Root Cause*: The cursor qualifier was `net`. In Rust 1.98, `SocketAddr` lives in `core::net` and is re-exported by `std::net`. During `ridx-driver` sysroot compilation, items re-exported across crates were excluded (`did.krate != cnum`), leaving `std::net` with only `Shutdown`. When owner completion matched `net` to `std::net`, it returned only 1 candidate (`Shutdown`). In `b05240a`, it fell back to workspace symbol search, finding 24 items.

#### 3. Route Migration & Tail Latency (p95)
- **Bevy Fallback Median Shift (11.6 ms → 1.59 ms)**: With sysroot metadata attached, exactly **16 heavy sysroot queries** (e.g., `Result::`, `Option::`, `String::`) that previously ran as slow linear scans across all 69 workspace members migrated to the owner path (warm p50 dropped from 13.1 ms to 773 µs). Removing these 16 queries relieved the fallback pool, leaving 52 lightweight queries with median 1.59 ms.
- **Tail Latency Reality (p95)**: On small repos (Serde, 2 workspace members), RIDX is faster across p50 (6.64x, 662 µs vs 4.40 ms) and p95 (6.06x, 2.01 ms vs 12.15 ms). On large multi-crate workspaces (Bevy, 69 workspace members, ~1,248 total dependency shards in index), RIDX owner-path p95 is 1.06 ms (18.35x faster than RA's 19.49 ms), but fallback p95 is 20.39 ms (vs RA's 6.43 ms, 3.17x slower), bringing overall Bevy p95 to 20.30 ms (vs RA's 17.08 ms, 1.19x slower).


