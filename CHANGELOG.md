# Changelog

All notable changes to `rust-intelligence` (`ridx`) are documented in this file.

## [Unreleased]

### Added
- **Standard Library Path Mapping**: Queries starting with `std::` (such as `std::vec::Vec` or `std::option::Option`) are automatically mapped to underlying canonical crates `core::` and `alloc::` in `find_definition_by_path` and `search_symbols`.
- **Rename Correctness Oracle**: Automated oracle verification against real-world crates (e.g. Tokio), validating that generated `WorkspaceEdit`s compile cleanly with zero errors. Published results stored in `bench/published/rename_oracle.json`.

### Changed
- **Canonical Path Extraction**: Wrapped reference and definition path generation in `ridx-driver` with `with_no_visible_paths!` and `with_no_trimmed_paths!`, eliminating re-export path divergences across multi-crate boundaries.
- **Re-export Alias Isolation**: Re-export alias names are strictly isolated from canonical references and used only for unmerged alias safety refusals during rename.
- **Notice on Reindexing**: Upgrades affecting driver path generation require reindexing workspaces (`ridx index .` or `ridx index . --all-features`) to replace cached CAS shards.
