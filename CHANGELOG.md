# Changelog

All notable changes to `rust-intelligence` (`ridx`) are documented in this file.

## [Unreleased]

### Added
- **Preparation for Sysroot Indexing**: Foundation for standard library path mapping (`std_path_candidates` mapping `std::` queries to canonical `core::` and `alloc::` paths in `find_definition_by_path` and `search_symbols`).
- **Rename Correctness Oracle**: Automated oracle verification against real-world crates (e.g. Tokio), validating that generated `WorkspaceEdit`s compile cleanly with zero errors. Published results stored in `bench/published/rename_oracle.json`.

### Changed
- **selectionRange chain**: the innermost node is the token under the cursor (identifier, keyword, `r#raw`, lifetime, or the whole comment or string/char literal the cursor is inside); the `fn` body `{..}` and the `fn` item (including its outer attributes and doc comments, like rust-analyzer's `FN` node) are separate nodes.
- **Canonical Path Extraction**: Wrapped reference and definition path generation in `ridx-driver` with `with_no_visible_paths!` and `with_no_trimmed_paths!`, eliminating re-export path divergences across multi-crate boundaries.
- **Re-export Alias Isolation**: Re-export alias names are strictly isolated from canonical references and used only for unmerged alias safety refusals during rename.
- **Notice on Reindexing**: Upgrades affecting driver path generation require reindexing workspaces (`ridx index .` or `ridx index . --all-features`) to replace cached CAS shards.
