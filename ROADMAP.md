# RIDX roadmap: complete Rust language-server support

## Goal and starting point

Make RIDX usable for everyday Rust editing: reliable navigation, useful diagnostics,
completion on incomplete code, and safe changes across a workspace. Keep compiler
snapshots on disk and bound the additional memory used for active editing.

The prototype already has handlers for definition, references, hover, document
symbols, workspace symbols, and a live buffer overlay. These are implementation
milestones, not a claim of complete or verified LSP support. Their correctness,
especially across compilation units and unsaved edits, is the first release gate.

Use [LSP 3.18](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.18/specification/)
as the capability inventory. The agreed scope is **all LSP functionality**, including
optional capability families. Compatibility is negotiated per capability, and only
implemented behavior is advertised. Protocol coverage and Rust semantic correctness
must be tracked separately. **rust-analyzer is the comparison baseline.**

## Agreed scope

- Implement the full LSP surface in stages, including lifecycle, synchronization,
  language features, workspace operations, and client/server capability negotiation.
- Keep RIDX storage, compiler extraction, Cargo integration, and live analysis as
  internal components supporting that LSP surface.
- Exclude LSIF/SCIP export, BSP, DAP, separate Base Protocol 0.9 support, and custom
  editor protocol extensions from this roadmap. The framing and lifecycle required
  by LSP itself remain in scope.
- Compare RIDX directly with rust-analyzer through the same LSP harness. Tokio and
  the rust-analyzer source workspace are test corpora; rust-analyzer is also the
  server against which behavior and performance are compared.

The first daily-use milestone is an intermediate release. It does not reduce the
final full-LSP scope or turn the remaining capabilities into optional future work.

## Architecture constraints

1. **Compiler snapshots.** Definitions, types, references, implementations, and macro
   provenance describe an identified source snapshot and build configuration. Extend
   the on-disk schema for missing relations as features need them; version the format
   and extractor together.
2. **Composed shard storage.** A workspace manifest selects local shards and shared
   immutable dependency shards. Compilation-unit identities include source identity,
   resolved features, target/host roles, compiler/extractor versions, effective flags,
   dependency identities, and relevant generated inputs. Never merge every cached
   variant. Stable keys must survive relocation without overlooking changed inputs.
3. **Live syntax.** Keep text versions and an error-tolerant Rust syntax tree for open
   buffers. Provide local scopes, document structure, syntax diagnostics, and position
   mappings. Preserve newer edits when a build completes. Establish a parser-backed
   implementation rather than expanding an ad hoc lexer into a Rust grammar.
4. **Live semantics.** Type-dependent completion, trait resolution, signature help,
   and refactorings need more than syntax. Evaluate reusable rust-analyzer components
   versus a focused semantic engine using incomplete-code fixtures and measured
   memory. Do not assume semantic analysis can be limited to one open file: imports,
   macros, and types may require a wider dependency slice. The distinction between
   syntax, HIR, and IDE features is described in the
   [rust-analyzer architecture](https://rust-analyzer.github.io/book/contributing/architecture.html).
5. **Snapshot-safe answers.** Carry source and target unit identities, source hashes,
   document versions, and index generations through queries. Unchanged line text
   alone cannot validate old semantics after an import or configuration change.
   Return no result when correctness cannot be established, and recompute when possible.
6. **Bounded resources.** Use cancellation, on-demand analysis, cache eviction, and
   explicit memory accounting. Mmap pages, source buffers, result allocations, and
   the live semantic working set all count toward actual memory use.

## Delivery stages and acceptance gates

### 0. Correctness of the existing foundation

- Resolve references using the target compilation unit, including callers in other
  crates, dependency aliases, and simultaneous versions of one package.
- Tie overlay baselines to the source snapshot that produced the published index.
  Remap both incoming cursors and outgoing ranges; handle edits to imports and scopes.
- Publish complete index generations atomically. Preserve the previous usable
  generation on build failure, interrupted publication, or invalid cache data.
- Make cache repair preserve the requested workspace, variant, and custom output path.
- Establish error-tolerant parsing and lexical visibility before extending the overlay.

**Gate:** targeted fixtures reproduce edit/save/build/reload sequences without wrong
symbol identity, stale ranges, crashes, or mixed variants. This gate blocks declaring
the current navigation implementation production-ready.

### 1. Protocol and workspace lifecycle

- Robust stdio framing; initialization/shutdown; request IDs and errors; capability
  negotiation; cancellation; progress; tracing; request scheduling and backpressure.
- Versioned incremental document synchronization, correct URI and position handling,
  and negotiated save notifications. Keep compiler progress off protocol stdout.
- Multi-root workspaces, configuration changes, watched files, and file operations.
  Rebuild the relevant Cargo graph when manifests, features, or configuration change.

**Gate:** real editor message sequences cover Unicode, partial reads, concurrent
requests, cancellation, restart, file rename/delete, and workspace changes.

### 2. Complete navigation

- Definition, declaration, type definition, implementation, references, highlights,
  documented hover, hierarchical document symbols, and workspace symbol resolution.
- Extend the index for missing type/implementation relations and documentation.
- Cover aliases, reexports, trait methods, generics, macro-generated code, dependency
  versions, and standard-library sources. Preserve macro provenance and meaningful
  source locations instead of presenting every expansion as one ambiguous call site.

**Gate:** expected targets and ranges match curated semantic fixtures in both saved
and edited documents; no cross-version reference pollution.

### 3. Diagnostics and everyday editing

- Immediate syntax diagnostics; compiler and optional Clippy diagnostics after a
  scheduled check. Publish only matching versions and clear obsolete diagnostics.
- Completion with deferred details, imports and snippets; signature help with active
  parameter tracking. Start with lexical candidates, then add type-directed results.
- Rustfmt-backed document formatting; separately specify range/on-type behavior.
- Choose and validate a live semantic backend before claiming receiver-type,
  generic, trait, or macro-aware completion on broken code.

**Gate:** editing incomplete Rust is useful without waiting for a successful build;
diagnostics and suggested edits never refer to an obsolete document version.

**First daily-use milestone:** stages 0–3, with correctness and resource evidence.

### 4. Structure and presentation

- Semantic tokens with negotiated full/range/delta support; inlay hints and lazy
  resolution; folding and selection ranges; document links.
- Separate syntax-derived presentation from type-dependent results. Refresh only
  when the underlying document or semantic snapshot changes.

**Gate:** highlights, hints, and ranges remain consistent during edits, including
non-ASCII identifiers, macro calls, and multiline strings.

### 5. Safe code changes

- Prepare rename and semantic rename; conflict detection; versioned workspace edits.
- Code actions and deferred resolution: compiler fixes, missing imports, basic
  extracts, and progressively broader Rust refactorings.
- Respect file-operation and edit capabilities, preview boundaries, cancellation,
  and unsaved changes across all affected files. Abstain on ambiguous macro mappings.

**Gate:** edits affect exactly the intended symbol, detect conflicts, and cannot
silently apply to a newer buffer. Compile relevant fixtures when the edit promises
to preserve valid code.

### 6. Relationships and commands

- Call/type hierarchies with explicit limitations for dynamic dispatch; CodeLens
  and commands for references, tests, and runnable targets.
- Cargo task scheduling, progress and cancellation behind LSP commands and CodeLens.
  Keep command behavior and build configuration explicit.

**Gate:** each relation and command has a defined semantic meaning, selected build
variant, and verified editor behavior.

### 7. Remaining LSP capabilities and scale

- Maintain the capability matrix below with implementation status, negotiation
  flags, fixture links, known limits, and client evidence as work lands.
- Complete linked editing, colors, monikers, notebooks, inline values, inline
  completion, and document-content requests. Each needs useful Rust behavior and
  tests of its full lifecycle, including capability negotiation and updates.
- Exercise VS Code and Neovim against pinned Tokio and rust-analyzer revisions,
  covering feature configurations and multiple dependency versions.
- Run the same LSP scenarios against RIDX and a pinned rust-analyzer server. Record
  correctness differences and performance for equivalent operations.

**Gate:** every LSP method has a recorded implementation and conformance result;
reproducible correctness and lifecycle measurements accompany releases. A capability
missing from rust-analyzer is still in scope and requires independent validation.

## Capability inventory

“Prototype” means a handler exists and still needs the gates above. “Planned” is work
to implement and validate. Every family below is in scope. The table groups methods;
stage 1 must expand it into a method-level checklist from a pinned specification,
tracking direction, capability flags, result variants, tests, and rust-analyzer
comparison status. An absent handler or placeholder response is not completion.

| Capability family | Planning status | Stage |
| --- | --- | --- |
| Base framing, lifecycle, errors, cancellation, progress and logging | Prototype; harden and complete | 1 |
| Document open/change/save/close, incremental sync, save hooks | Prototype; harden and complete | 1 |
| Workspace folders, configuration, watchers, create/rename/delete events and edits | Planned | 1, 5 |
| Definition, references, hover, document/workspace symbols | Prototype; correctness first | 0, 2 |
| Declaration, type definition, implementation, document highlights | Planned | 2 |
| Workspace symbol resolution | Planned | 2 |
| Push diagnostics; document/workspace pull diagnostics and refresh | Planned; negotiate client model | 3 |
| Completion and resolution; signature help | Planned | 3 |
| Document/range/on-type formatting | Planned; each mode needs its own gate | 3 |
| Semantic tokens full/range/delta and refresh | Planned | 4 |
| Inlay hints, resolution and refresh | Planned | 4 |
| Folding/selection ranges, document links and resolution | Planned | 4 |
| Prepare rename, rename, versioned workspace edits and apply-edit flow | Planned | 5 |
| Code actions, resolution and commands | Planned | 5 |
| Call hierarchy and type hierarchy | Planned | 6 |
| CodeLens, resolution and refresh | Planned | 6 |
| Window messages, show-document, configuration requests, tracing and telemetry policy | Planned; client-capability aware | 1, 6 |
| Linked editing, document colors and color presentations | Planned | 7 |
| Monikers for external symbol identity | Planned | 7 |
| Notebook synchronization, inline values and inline completion | Planned | 7 |
| Workspace document-content requests and refresh | Planned | 7 |
| Dynamic registration, partial results and refresh notifications | Planned where applicable to supported families | 1–7 |

The method inventory follows the
[official LSP specification](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.18/specification/).
The final milestone includes optional protocol features as a product requirement.
Unsupported behavior remains an explicit gap until implemented; narrowing this scope
requires a new user decision. Legitimate empty results must reflect the input, not
stand in for an unimplemented feature.

## Validation and performance evidence

Use small semantic fixtures for correctness, then pinned **Tokio** and
**rust-analyzer** workspaces for scale. Record revision, toolchain, hardware, build
profile, target, feature selection, command lines, and cache state. Pin the
**rust-analyzer server version** separately from the rust-analyzer source corpus.
Keep these six measurements separate:

| Measurement | Required evidence |
| --- | --- |
| Initial indexing | Wall time against equivalent plain Cargo check; extraction overhead and process-tree peak memory. |
| Warm no-op | Startup and fingerprint costs; number of compile units and shards actually revisited. |
| Incremental change | One-file and public-API edits through compilation, shard publication, and visible LSP update. |
| Open and first query | Manifest/shard loading, first page faults, and first complete editor response; distinguish cold process from cold filesystem cache. |
| Warm requests | End-to-end LSP p50/p95/p99 and result correctness, including serialization and position conversion, not just index lookup. |
| Storage and memory | Local/shared cache sizes, duplicate objects, idle/active/peak RSS, mapped pages, live buffers, semantic caches, and eviction behavior. |

Also measure switching feature variants, cross-project cache reuse, failed builds,
high-fanout references, and unsaved edits. The comparison harness must:

- Replay identical LSP requests and edit sequences against RIDX and rust-analyzer,
  with matching source snapshots, Cargo features, target, toolchain, and capabilities.
- Compare result correctness and feature coverage before latency. Use curated
  expected answers to investigate disagreements; rust-analyzer is a baseline, not
  an infallible oracle. Mark methods it does not support explicitly.
- Measure both servers through the same local transport and client, with equivalent
  warm/cold conditions. Exclude prod-code or network wrapper timings from the main
  comparison; raw RIDX lookup timings are supplementary measurements only.
- Measure startup to usable results, edit/save to fresh answers, end-to-end request
  latency, and actual memory for both servers. Report RIDX indexing cost separately
  against plain Cargo check and compare overall readiness with rust-analyzer.
- Record repetitions and latency distributions, timeouts, cancellations, errors,
  and cache misses. Do not calculate a speedup for unequal or incorrect results.

Use SIMD-capable byte/string search primitives for suitable hot paths, including
bounded NUL scans and repeated substring searches. Preserve portable fallback and
measure short and long strings in realistic pools; do not infer end-to-end speed
from an isolated scan. Optimization does not relax snapshot or bounds correctness.

Define latency and memory budgets from reproducible baselines before each milestone.
Do not use zero-RSS, zero-startup, or universal speedup claims as acceptance criteria.

## Immediate implementation order

1. Close snapshot, compilation-unit identity, and cache repair correctness gaps.
2. Establish parser-backed document state and protocol lifecycle coverage.
3. Complete navigation and connect versioned compiler diagnostics.
4. Validate the live semantic approach with completion and signature-help fixtures.
5. Deliver the daily-use milestone, then complete presentation, safe refactorings,
   relationships, and every remaining LSP capability with rust-analyzer comparisons.

This document records scope and acceptance criteria. It does not mark planned
features as implemented or prescribe dates before the semantic-backend evaluation.
