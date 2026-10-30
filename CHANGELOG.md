# Changelog

All notable changes to `rust-intelligence` (`ridx`) are documented in this file.

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
