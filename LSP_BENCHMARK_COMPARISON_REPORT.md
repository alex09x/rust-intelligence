## Benchmark Report: `serde-rs/serde` (commit `6693a89c`) — Preliminary (Serde only)
*Source Data:* `bench/results/pilot_results.jsonl`  
*Run Status:* `SUCCESS`  
*Driver SHA256:* `ac554e8f3b302af637109a0679adf95aff0d8d35937d6f6b4e99770044b720c8`  
*rust-analyzer Version:* `rust-analyzer 1.98.1 (48a229c 2026-09-01)`  
*Target Node:* `booster` (AMD Ryzen 9 7950X, 32 cores, 128 GB RAM, NVMe SSD, isolated on cores 24–31)

### 1. Headline Performance & Resource Footprint

| Metric | `ridx lsp` | `rust-analyzer` (Live) | Comparison / Notes |
|---|---|---|---|
| **Cold Clean Index / Check** | **3.968 s** (`ridx index`) | 1.2 s (`cargo check`) | 3.2x of cargo check |
| **Warm Incremental Re-index (W1)** | **3160.3 ms** (`ridx index`) | **150.2 ms** (RA quiescent) | 21.0x slower |
| **Peak Resident Set Size (RSS)** | **47.9 MB** (Anon: **14.0 MB**, File: 34.4 MB) | **1324.2 MB** (Anon: **1292.2 MB**, File: 32.0 MB) | **92.3x lower Heap RAM** |
| **Active OS Threads** | **2 threads** | **37 threads** | 18.5x fewer threads |
| **Open File Descriptors** | **3 total** (0 open `.ridx` FDs) | **9 total** | Fixed mmap file handles |
| **Def Latency (Warm p50)** | **43.0 µs** | **131.0 µs** | Comparable on quiet node (3.0x) |
| **Def Latency (Warm p90)** | **204.3 µs** | **5449.9 µs** | **26.7x faster tail latency** |
| **Hover Latency (Warm p50)** | **68.0 µs** | **103.5 µs** | Comparable on quiet node (1.5x) |
| **Refs Latency (Warm p50)** | **93.5 µs** | **1534.0 µs** | **16.4x faster** |
| **Refs Latency (Warm p90)** | **1183.4 µs** | **24493.8 µs** | **20.7x faster tail latency** |
| **Read-Only Strict Match Rate (Class A)** | **262 / 300 (87.3%, 95% CI [83.1%, 90.6%]) (87.33%)** | Baseline | Wilson 95% CI |
| **Read-Only Line Diagnostic Match (Class B)** | **262 / 300 (87.3%, 95% CI [83.1%, 90.6%]) (87.33%)** | Baseline | Same-line resolution |
| **Semantic Parity (Class A + Class C)** | **269 / 300 (89.7%, 95% CI [85.7%, 92.6%]) (89.67%)** | Baseline | Strict + Class C default method equiv |
| **Compiler ICE & Caught Panics** | **0 ICE, 0 caught panics** | 0 ICE | Driver telemetry verified |
| **cargo Spawned During Queries** | **ridx: false, ra: false** | **Quiescent** | Verified via process monitoring |

### 2. Latency Analysis & Measured Architectural Differences

On a quiet, dedicated build node (`booster`, AMD Ryzen 9 7950X, 32 cores, 128 GB RAM, isolated on cores 24–31), warmed in-memory query latencies for single-token lookups at median (p50) are: rust-analyzer achieves **131.0 µs** (def) and **103.5 µs** (hover), while RIDX achieves **43.0 µs** (def, 3.0x faster) and **68.0 µs** (hover, 1.5x faster).
*Latency Shift Explanation (148 µs → 43 µs in commit `f40e6d9`):*
In previous revisions, cross-crate reference lookups lacked exact `target_unit` binding for external crates when sidecars used raw cargo hashes, causing `find_cross_definition` to fall through into an unconstrained search across all 65 readers in the workspace. With `f40e6d9` introducing `{cname}_{stable_crate_id:016x}.json` sidecar indexing and scoping fallback readers strictly to the matching `target_unit`/`target_crate`, the linear 65-reader scan was eliminated. Concrete evidence: on scenario `def_15_Struct` (`test_enum_untagged.rs:471:20`), latency dropped from **819 µs** (which misresolved to `syn-3.0.6`) to **40 µs** (resolving accurately to `serde_test-1.0.177`). Prior to `f40e6d9`, 'parity' was the accurate characterization (~148 µs vs ~145 µs); post-`f40e6d9`, RIDX's p50 definition latency is measured at **43.0 µs** (3.0x faster than rust-analyzer's 131.0 µs) with verified root cause, alongside RIDX's primary architectural strengths: tail latency (p90), reference search, and zero-heap scaling.

Measured architectural differences from the test run:
1. **Tail Latency**: `textDocument/definition` p90 is **26.7x faster** (204.3 µs vs 5449.9 µs / 5.45 ms).
2. **Multi-Token Reference Search**: `textDocument/references` is **16.4x faster at p50** (93.5 µs vs 1534.0 µs) and **20.7x faster at p90** (1183.4 µs vs 24493.8 µs).
3. **Process Resources**: RIDX maintains 2 OS threads and 3 open file descriptors, whereas rust-analyzer scales to 37 threads and 9 file descriptors.
4. **Memory Footprint**: RIDX uses **92.3x lower Heap RAM** (Anon Heap 14.0 MB vs 1292.2 MB).
5. **Multi-Worktree Scaling**: Across 10 concurrent worktrees, RIDX consumes **20.5 MB PSS** total (8.5 MB Anon) vs rust-analyzer's **9211.5 MB PSS** (9181.6 MB Anon) — a **449.3x lower PSS** footprint.
6. **Warm Incremental Re-index (W1)**: rust-analyzer reaches freshness in **150.2 ms**, which is **21.0x faster** than RIDX (**3160.3 ms**).

### 2.1 Compiler Stability & Panic Telemetry (Heavy Generic Repositories, Clean CAS)

To verify compiler stability and validate that `catch_unwind` does not silently swallow errors, `ridx-driver` was evaluated with a completely empty CAS cache across 11 heavy generic repositories on `booster` (isolated cores 24–31). Stderr was monitored for `internal compiler error`, and `caught_panics` telemetry was aggregated across all produced sidecars.

Eight repositories completed full indexing without errors. Three repositories (`diesel`, `sqlx`, `wasmtime`) halted during `cargo check` due to external build-time requirements (database dev headers, online query macro database credentials, and wasm32 compilation target) before reaching index completion; these are reported below as unindexed with causes identified, and excluded from the completed totals:

| Repository | Units / Sidecars | Indexed Crates | Definitions | References | Types | ICE Detected | Caught Panics | Status |
|---|---|---|---|---|---|---|---|---|
| **serde** | 33 | 32 | 31,783 | 163,698 | 460,332 | **0 ICE** | **0** | Complete |
| **axum** | 198 | 11 | 14,756 | 108,091 | 303,851 | **0 ICE** | **0** | Complete |
| **tantivy** | 921 | 74 | 56,763 | 499,035 | 907,960 | **0 ICE** | **0** | Complete |
| **gitoxide** | 1,707 | 270 | 128,534 | 1,337,365 | 2,378,313 | **0 ICE** | **0** | Complete |
| **burn** | 3,906 | 275 | 213,148 | 1,591,201 | 3,666,785 | **0 ICE** | **0** | Complete |
| **datafusion** | 2,397 | 283 | 274,712 | 2,898,340 | 5,444,895 | **0 ICE** | **0** | Complete |
| **polars** | 2,009 | 50 | 54,625 | 514,835 | 912,341 | **0 ICE** | **0** | Complete |
| **bevy** | 871 | 616 | 430,865 | 1,883,927 | 4,060,358 | **0 ICE** | **0** | Complete |
| **TOTAL (8 complete)** | **12,042** | **1,611** | **1,205,186** | **8,996,492** | **18,124,835** | **0 ICE** | **0** | **100% Success** |
| *diesel* | 823 | — | — | — | — | — | — | *Not indexed: integration_tests require libpq/libmysqlclient/sqlite3-dev* |
| *sqlx* | 1,471 | — | — | — | — | — | — | *Not indexed: examples require DATABASE_URL (query!)* |
| *wasmtime* | 1,404 | — | — | — | — | — | — | *Not indexed: build.rs requires wasm32-unknown-unknown target* |


### 3. Semantic Parity & Discrepancies Breakdown

Across the sampled 300 read-only scenarios:
- **Class A Strict Matches**: 262/300 (87.3% [262 / 300 (87.3%, 95% CI [83.1%, 90.6%])])
- **Class A + Class C Semantic Parity**: 269/300 (89.7% [269 / 300 (89.7%, 95% CI [85.7%, 92.6%])])

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
