# Rust Intelligence (`ridx`)

Ultra-fast, zero-memory code intelligence for Rust inspired by `gopls` and powered by compiler-driven indexing.

---

## 1. The Problem: Why `rust-analyzer` Consumes Gigabytes

In large Rust workspaces (measured on BTCR with 698 crates), `rust-analyzer` consumes:
- **3.3 – 3.7 GB RSS** (2.8 GB live heap).
- **52% of the heap (1.14 GB)** is spent on Rowan syntax trees (green nodes and tokens).
- Another **11% (243 MB)** is raw file texts (`Arc<str>`).
- Over **14,986 files** are loaded into memory, yet only **203 files** belong to the project itself; the other 98.6% are third-party registry dependencies and the standard library that developers almost never edit.
- Lowering LRU cache caps does not reclaim this memory because trees are retained across Salsa/HIR queries.

### How `gopls` Solved This in Go
Go's language server (`gopls` v0.12+) avoids keeping dependency trees in memory:
1. Dependencies are never parsed into memory during editing.
2. Type signatures and cross-references are written to a content-addressed disk cache (`filecache`).
3. Only packages with currently open files live in memory.

---

## 2. The Solution: RIDX (Rust Intelligence Index)

`rustc` already computes all definition locations, reference resolutions, trait implementations, and expression types during `cargo check`.

Instead of running a persistent 3+ GB daemon that re-parses and re-analyzes every dependency in memory, **Rust Intelligence** introduces a zero-copy, binary on-disk index format (`.ridx`):
- **Compiler-Driven**: Generated via a lightweight `rustc_driver` callback during `cargo check` (~7% overhead, 300 ms for `hir_ty`).
- **Zero-Copy Memory Mapping**: Files are mapped directly into virtual memory via `mmap`. No heap allocations or deserialization overhead.
- **Instant Response**: Go to Definition, Find References, and Hover/Type-At respond in **under 50 microseconds** ($O(\log N)$ binary search).
- **Zero Daemon RSS**: 0.00 MB resident server footprint.

---

## 3. Binary Index Specification (`.ridx` v1)

All integers are stored in little-endian format with 8-byte boundary alignment.

```
┌────────────────────────────────────────────────────────┐
│ Header (Magic 'RIDX', Version, Table Offsets & Counts) │
├────────────────────────────────────────────────────────┤
│ String Intern Pool (Deduplicated UTF-8 Strings)       │
├────────────────────────────────────────────────────────┤
│ File Path Table (u32 String Pool Offsets)              │
├────────────────────────────────────────────────────────┤
│ Definitions Table (Array of DefEntry, 40 bytes each)   │
├────────────────────────────────────────────────────────┤
│ Definitions Hash Index (FNV-1a 64-bit Path Hashes, 16B)│
├────────────────────────────────────────────────────────┤
│ References Table (Sorted by Span, 44 bytes each)      │
├────────────────────────────────────────────────────────┤
│ References Target Hash Index (Target Hashes, 16B)      │
├────────────────────────────────────────────────────────┤
│ Types Table (Array of TypeEntry, 28 bytes each)        │
└────────────────────────────────────────────────────────┘
```

### Table Layout & Records

1. **`DefEntry` (40 bytes, 4-byte aligned)**:
   - `def_id`: `u32` (4 bytes)
   - `kind`: `u8` (`DefKind`: Fn, Struct, Enum, Trait, AssocFn, etc.)
   - `visibility`: `u8` (Public, Crate, Restricted, Private)
   - `_pad`: `[u8; 2]` (2 bytes)
   - `name_offset`: `u32` (String pool offset for identifier)
   - `path_offset`: `u32` (String pool offset for canonical path)
   - `sig_offset`: `u32` (String pool offset for function signature)
   - `span`: `Span` (file_id, start_line, start_col, end_line, end_col; 20 bytes)

2. **`RefEntry` (44 bytes, 4-byte aligned)**:
   - `span`: `Span` (Source location of reference usage; 20 bytes)
   - `target_def_id`: `u32` (Local definition ID or `u32::MAX`)
   - `target_path_offset`: `u32` (Target canonical symbol path)
   - `target_file_offset`: `u32` (Target definition file path)
   - `target_line`: `u32` (Target definition line)
   - `target_col`: `u32` (Target definition column)
   - `kind`: `u8`
   - `is_write`: `u8` (Read vs write access)
   - `_pad`: `[u8; 2]` (2 bytes)

3. **`TypeEntry` (28 bytes, 4-byte aligned)**:
   - `span`: `Span` (Expression source span; 20 bytes)
   - `type_offset`: `u32` (String pool offset for inferred type name)
   - `_pad`: `[u8; 4]` (4 bytes)

4. **`HashIndexEntry` (16 bytes, 8-byte aligned)**:
   - `hash`: `u64` (8 bytes, FNV-1a 64-bit hash)
   - `index`: `u32` (4 bytes, record index)
   - `_pad`: `[u8; 4]` (4 bytes)

5. **`Header` (128 bytes, 8-byte aligned)**:
   - Magic `b"RIDX"` (4B), version `1` (4B), offsets and counts for all tables with explicit 8-byte boundary alignment.

6. **String Intern Pool**:
   - Stores paths, signatures, and types once with complete deduplication.
   - Reduces index text size from tens of megabytes to a few hundred kilobytes.

---

## 4. CLI Usage (`ridx`)

### Inspecting an Index File
```bash
ridx inspect target/ridx/my_crate.ridx
```
Displays header metadata, string pool compression ratio, and record counts.

### Go to Definition
By symbol path:
```bash
ridx def target/ridx/my_crate.ridx my_crate::math::calculate
```
By source cursor position (`file:line:col`):
```bash
ridx def target/ridx/my_crate.ridx src/main.rs:15:20
```

### Find References
```bash
ridx refs target/ridx/my_crate.ridx my_crate::math::calculate
```

### Inferred Expression Type (Hover)
```bash
ridx type-at target/ridx/my_crate.ridx src/main.rs:15:22
```

### Symbol Search
```bash
ridx symbols target/ridx/my_crate.ridx calculate
```

### Side-by-Side Comparison with `prod-code`
```bash
ridx compare target/ridx/my_crate.ridx --symbol my_crate::math::calculate
```

---

## 5. Preliminary Performance Comparison

| Metric | `ridx` (Zero-Copy Mmap) | `prod-code` (In-Memory rust-analyzer) |
|---|---|---|
| **Query Latency (Def)** | **< 30 µs** | **~ 30 – 50 ms** (~1,000x difference) |
| **Daemon RSS Memory** | **0.00 MB** (no daemon) | **~ 2,700 MB** (warm remote gateway) |
| **Index Size** | **~ 3 – 8 MB** per crate | N/A (300+ MB per crate in RAM) |
| **Cold Start** | **Instant (0 ms)** | **10 – 30 seconds** (cache priming) |

---

## 6. Architecture & Division of Labor

```
                    ┌────────────────────────┐
                    │     Code Request       │
                    └───────────┬────────────┘
                                │
               ┌────────────────┴────────────────┐
               │                                 │
     [Read-Only Navigation]              [Active Editing]
  (Go to Def, Refs, Hover, Types)     (Completion, Refactor, Assists)
               │                                 │
               ▼                                 ▼
   ┌───────────────────────┐         ┌───────────────────────┐
   │ RIDX Disk Index Engine│         │ Scoped rust-analyzer  │
   │ (<50 µs, 0 MB Daemon) │         │ (Open files/crate only│
   └───────────────────────┘         └───────────────────────┘
```

---

## 7. Engineering Roadmap

### Phase 1: Core Zero-Copy Binary Format & Reader/Writer ✅
- [x] Header specification with 8-byte alignment.
- [x] String Intern Pool with null-terminated UTF-8 strings.
- [x] File table deduplication.
- [x] `DefEntry`, `RefEntry`, `TypeEntry` compact binary layouts.
- [x] Fast FNV-1a 64-bit hash indices for symbols and targets.
- [x] `IndexWriter` implementation.
- [x] Memory-mapped zero-copy `IndexReader` with sub-50 µs lookups.
- [x] Comprehensive round-trip unit and integration tests.

### Phase 2: Compiler Driver (`ridx-driver`) ✅
- [x] `rustc_driver` callback architecture (`after_analysis`).
- [x] HIR visitor capturing definitions, function signatures, and method resolutions.
- [x] Expression type extraction from `TypeckResults`.
- [x] Direct serialization to `.ridx` binary format.

### Phase 3: CLI & Benchmarking Harness (`ridx`) ✅
- [x] `ridx inspect`: Binary index introspection and memory layout statistics.
- [x] `ridx def`: Location and symbol resolution.
- [x] `ridx refs`: Instant cross-reference lookup.
- [x] `ridx type-at`: Expression hover resolution with innermost span selection.
- [x] `ridx symbols`: Fast substring symbol search.
- [x] `ridx compare`: Automated latency, accuracy, and memory benchmarking against `prod-code`.

### Phase 4: Macro Span Mapping & Cross-Crate Graph
- [ ] Call-site remapping for macro expansions (`span.from_expansion()` translation).
- [ ] Cross-crate global reference resolution (linking local uses to dependency `.ridx` files).
- [ ] Content-addressed shared caching for `crates.io` dependencies.

### Phase 5: Differential Updates & `prod-code` Gateway Integration
- [ ] Incremental differential index updates on file changes without full rebuilds.
- [ ] Gateway routing: direct read-only LSP requests (`textDocument/definition`, `textDocument/references`, `textDocument/hover`) to RIDX, reserving rust-analyzer only for active edits.
