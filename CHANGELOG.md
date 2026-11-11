# Changelog

All notable changes to `rust-intelligence` (`ridx`) are documented in this file.

## [0.3.0] - 2026-11-11

Live type inference behind the LSP: dependencies and the standard library come from the rustc-built index, while the workspace's own code is parsed, name-resolved and type-inferred by `ridx` itself, on saved and on edited buffers alike. Figures below are from the 38-method, 7-corpus content-parity suite against rust-analyzer (`serde`, `ripgrep`, `tokio`, `rust-analyzer`, `bevy`, `clap`, `regex`), before (the last state without live inference in the LSP) and after (`602f4a19`), on the same index.

### Added
- **`textDocument/typeDefinition` from the inferred type**: every type the inferred type names (`Vec<Foo>` leads to `Vec`, `Foo` and the allocator). Parity `89.22%` → `93.43%`.
- **`textDocument/completion` after `.` from the receiver's inferred type**: fields and methods of the full receiver type with autoderef, traits in scope, bounds of type parameters and of a trait's `Self`, `#[doc(hidden)]` items of other crates hidden, postfix snippets (`let`, `letm`, `for`, `match`, `ifl`, `lete`, `while`, `if`, `not`, `break`, `drop`) chosen by the receiver's type and the expression's position as rust-analyzer does, and `await` / `await.member` on futures. Parity `78.36%` → `95.52%` (`1920/2010`); exact label-set matches `292` → `1326`; median latency `2.90 ms` → `1.76 ms`.
- **`#[cfg(test)]` code answered from the unit that compiles it** (a library's test harness), for type queries and completion.

### Changed
- **`textDocument/references`**: `Self` inside an impl counts as a reference to its struct, enum or union. Parity `80.37%` → `82.20%`.
- **`textDocument/inlayHint`**: recorded types are merged from every unit compiling the file, so test-module bindings get hints. Parity `94.31%` → `95.08%`.
- **Trait solving**: bounds that land on a type parameter are checked against that parameter's declared bounds; compiler-implemented traits without an indexed definition (`Destruct`) no longer fail bound checks; type references no longer resolve to a same-path derive macro instead of the trait.

### Notes
- Refactoring assists stay compile-safe: `Inline into all callers` is not offered where the inlined body would reference private items from another module or crate, although rust-analyzer offers it there (`code_action` parity stays at `58%`).
- Latency: `typeDefinition` median `0.54 ms` (`0.16 ms` before; the inferred answer costs a parse and a function inference on the first query in a file). Peak server memory on `bevy` is unchanged at about 1 GB.

## [0.2.0] - 2026-10-30

First tagged release (`git tag` had no prior tags; entries below summarize changes relative to the initial `0.1.0` workspace baseline and the 5-corpus parity program `3fce845..d7fbd78`).

### Added
- **Drop-in `rust-analyzer` CLI & Editor Configs (`e9f468c`)**: Invoking `ridx` with no subcommand starts the LSP server on `stdio`. Added `rust-analyzer`-compatible CLI flags (`-v`/`--verbose`, `-q`/`--quiet`, `--log-file <path>`, `--print-config-schema`), configuration templates in [`docs/editors/vscode-settings.json`](docs/editors/vscode-settings.json), [`docs/editors/zed-settings.json`](docs/editors/zed-settings.json), and [`docs/editors/neovim-lspconfig.lua`](docs/editors/neovim-lspconfig.lua), and the headless drop-in session verifier ([`bench/dropin_editor_session.py`](bench/dropin_editor_session.py)).
- **Enriched `textDocument/hover` with Provenance & Blast Radius (`c2aaec7`)**: Hover responses now include full multi-line item declaration signatures (`where` clauses and bounds), source doc comments (`///`, `#[doc = "..."]`, `#[cfg_attr(..., doc = "...")]`), canonical crate/file **Provenance**, and precomputed $O(1)$ **Blast Radius** counters (`BlastRadiusIndex`) deduplicated by normalized workspace file path and Cargo package.
- **`.ridx` v3 Format (`DefEntry::owner_hash` & `DerefEntry`) (`d2cbf33`, `3af1467`)**: Extended binary index format (`.ridx` v3) with container `owner_hash` for associated items (`d2cbf33`) and `DerefEntry` receiver chains for `Deref` and supertrait method resolution (`3af1467`).
- **5-Corpus Content-Parity Sidecars (`7715510`, `462fe99`, `d7fbd78`)**: Published 5-corpus content-parity sidecars (`bench/published/position_ab_parity_*_vs_ra_ram9_*.differing.json`) across `serde`, `ripgrep`, `tokio`, `rust-analyzer`, and `bevy`.
- **Preparation for Sysroot Indexing**: Standard library path mapping (`std_path_candidates` mapping `std::` queries to canonical `core::` and `alloc::` paths in `find_definition_by_path` and `search_symbols`).
- **Rename Correctness Oracle**: Automated oracle verification against real-world crates (e.g. Tokio), validating that generated `WorkspaceEdit`s compile cleanly with zero errors (`bench/published/rename_oracle.json`).

### Changed
- **`textDocument/references` Precision & Container Scoping (`7748b2d`, `2a58115`)**: Filtered trait/impl sibling associated items by `DefEntry::owner_hash` and rejected multi-line and whole-item spans in cursor/reference resolution (`2a58115`), raising 5-corpus workspace line precision from `57.0%` (`030b224`) to `90.4%` (`152,976 / 169,270` lines, `83.5%` line recall, `968/1352 = 71.6%` full-set parity in `d7fbd78`).
- **`textDocument/definition` & `textDocument/hover` Parity (`75c8a64`, `7748b2d`, `c2aaec7`, `d7fbd78`)**: Reached `94.8%` (`1293/1364`) on `definition` and `96.1%` (`1332/1386`) on `hover` across the 5-corpus benchmark suite (`d7fbd78`), while avoiding redundant `type_at` scans when a resolved `DefEntry` already carries a signature (`7748b2d`).
- **`textDocument/completion` Type-Directed Receiver Lookup (`d2cbf33`, `3af1467`)**: Added owner-hash receiver member lookup (`d2cbf33`) and `Deref`/supertrait receiver chain traversal (`3af1467`), achieving `1108/1443 = 76.8%` target label recall (`878/1443 = 60.8%` in top-20, `37/1443 = 2.6%` exact full-set parity in `d7fbd78`).
- **selectionRange chain**: The innermost node is the token under the cursor (identifier, keyword, `r#raw`, lifetime, or the whole comment or string/char literal the cursor is inside); the `fn` body `{..}` and the `fn` item (including its outer attributes and doc comments, like `rust-analyzer`'s `FN` node) are separate nodes.
- **Canonical Path Extraction**: Wrapped reference and definition path generation in `ridx-driver` with `with_no_visible_paths!` and `with_no_trimmed_paths!`, eliminating re-export path divergences across multi-crate boundaries.
- **Re-export Alias Isolation**: Re-export alias names are strictly isolated from canonical references and used only for unmerged alias safety refusals during rename.
- **Notice on Reindexing**: Upgrades affecting driver path generation or `.ridx` v3 layout require reindexing workspaces (`ridx index .` or `ridx index . --all-features`) to replace cached CAS shards.
