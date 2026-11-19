# Rust Intelligence (`ridx`)

A language server for Rust that answers from what rustc already worked out. `ridx` records definitions,
references and types once, during `cargo check`, into a binary index, and reads that index straight from
disk. Your editor gets navigation, hover, completion, references and rename without a second compiler
held in memory.

Linux and macOS. Not affiliated with the Rust Project or the Rust Foundation.

---

## Why

- **rust-analyzer keeps its own analysis of the whole workspace in memory**: 1.5 GB on serde and
  11.3 GB on Bevy in the run below. It repeats name resolution, macro expansion and type inference that
  rustc performs anyway on every `cargo check`.
- **Most of what is analyzed is dependencies and std**, code nobody edits. `ridx` indexes each
  dependency once and shares it between every project and worktree on the machine.
- **What you are typing still works**: unsaved edits are answered from an in-memory overlay on top of the
  index until the project builds again.

## Measured

The same editor session against both servers: 40 files of each repository opened one by one, with
document symbols, semantic tokens, folding, inlay hints, then hover, definition, type definition, document
highlight, references and completion at sampled positions, and an edit. Memory includes child processes.

| Repository | Peak memory, ridx / rust-analyzer | CPU for the session | Start until ready |
|---|---:|---:|---:|
| serde | 72 MB / 1.5 GB | 2.34 s / 6.97 s | 0.06 s / 3.27 s |
| tokio | 221 MB / 3.2 GB | 1.90 s / 35.9 s | 0.24 s / 4.77 s |
| rust-analyzer | 434 MB / 4.6 GB | 4.44 s / 66.7 s | 0.64 s / 5.22 s |
| bevy | 917 MB / 11.3 GB | 5.38 s / 170.5 s | 1.26 s / 15.5 s |

- ridx `f7105fcc`, 2026-11-14; one run (n = 1) on Linux 6.8, AMD Ryzen 9 7950X, 64 GB. Raw results:
  [`bench/published/resources_f7105fcc.jsonl`](bench/published/resources_f7105fcc.jsonl), script:
  [`bench/measure_resources.py`](bench/measure_resources.py); per-method latency is in the raw results and
  on the site.
- **Not counted**: ridx answers from an index built beforehand (`ridx index`, a `cargo check` with the
  ridx driver), which takes minutes on a large workspace the first time. rust-analyzer analyzes the
  workspace each time it starts, and that time is in its numbers.
- **"Ready"** is the reply to `initialize` for ridx (caches keep warming in the background after it) and,
  for rust-analyzer, the end of its analysis plus 0.5 s without messages.
- **Memory**: ridx's figure includes index pages mapped from disk, which the kernel can drop under
  pressure and read again. rust-analyzer's is its own heap.
- rust-analyzer replied with an error to 26–31 requests per repository, ridx to none; the kinds were not
  recorded. Errored requests are included in the latency figures.

Agreement of the answers with rust-analyzer per LSP method is measured with the suite in [`bench/`](bench/)
and recorded per release in [`CHANGELOG.md`](CHANGELOG.md). The full 78-method matrix of `v0.2.0` is in
[`docs/BENCHMARK_MATRIX.md`](docs/BENCHMARK_MATRIX.md) and
[`LSP_BENCHMARK_COMPARISON_REPORT.md`](LSP_BENCHMARK_COMPARISON_REPORT.md).

---

## Install

`ridx-driver` runs inside rustc, so both binaries are built with the toolchain pinned in
[`rust-toolchain.toml`](rust-toolchain.toml), and that toolchain stays installed: the driver links to its
compiler libraries. Your own projects can use any toolchain; ridx runs the index build with the right one.

```bash
rustup toolchain install 1.98.1 --component rustc-dev,llvm-tools-preview,rust-src,rustfmt,clippy
git clone https://github.com/alex09x/rust-intelligence
cd rust-intelligence
RUSTC_BOOTSTRAP=1 cargo +1.98.1 install --path crates/ridx --locked
RUSTC_BOOTSTRAP=1 cargo +1.98.1 install --path crates/ridx-driver --locked
```

Then open a Cargo workspace in your editor. With no index yet, `ridx` builds one on first use and reports
progress to the editor; it reloads the index when the build finishes and reindexes in the background after
saves. `ridx index .` builds it from the shell (`--all-features`, `--target <triple>` choose the cfg).

If a desktop editor cannot find `ridx`, give it the full path from `which ridx`.

## Editors

`ridx` with no subcommand behaves as `rust-analyzer` on the command line (`--version`, `--stdio`,
`--log-file`, `--print-config-schema`), so an editor set up for rust-analyzer can start it instead.

- **Zed**: the [`editors/zed`](editors/zed) extension registers `ridx lsp` as the `rust-intelligence`
  language server (command palette: `zed: install dev extension`), and
  [`docs/editors/zed-settings.json`](docs/editors/zed-settings.json) selects it for Rust.
- **VS Code**: [`docs/editors/vscode-settings.json`](docs/editors/vscode-settings.json) points the
  rust-analyzer extension at `ridx` (`"rust-analyzer.server.path": "ridx"`).
- **Neovim**: [`docs/editors/neovim-lspconfig.lua`](docs/editors/neovim-lspconfig.lua) (`cmd = { "ridx" }`,
  root found by `Cargo.lock` first, so a workspace member opens the workspace's index).
- **Helix**: a `[language-server.ridx]` with `command = "ridx"`, `args = ["lsp"]` in `languages.toml`.

## From the shell

```bash
ridx index .                     # build or refresh the index
ridx def . my_crate::run         # go to definition, by path or file:line:col
ridx refs . src/main.rs:12:15    # references across the workspace and its dependencies
ridx hover . src/main.rs:12:15
ridx type-at . src/main.rs:12:15
ridx symbols . Config
ridx inspect .                   # shards, crates and record counts
ridx compare -c all . my_crate::run   # side by side with rust-analyzer
```

---

## LSP methods

| Category | Methods |
|---|---|
| Navigation | `definition`, `declaration`, `implementation`, `typeDefinition` |
| References | `references`, `documentHighlight` |
| Intelligence | `hover`, `completion` (with auto-import), `signatureHelp`, `inlayHint`, `semanticTokens` |
| Hierarchies | `prepareCallHierarchy`, `incomingCalls`, `outgoingCalls`, `prepareTypeHierarchy`, `supertypes`, `subtypes` |
| Refactoring | `prepareRename`, `rename`, `codeAction` (compile-checked assists), `executeCommand` |
| Formatting | `formatting`, `rangeFormatting` |
| Symbols | `documentSymbol`, `workspace/symbol` |
| Diagnostics | `publishDiagnostics` from a background `cargo check` on save, pull diagnostics |
| Workspaces | `didChangeWorkspaceFolders`, `didChangeWatchedFiles`, multi-root |

## How it works

```
 editor ──JSON-RPC (stdio)──► ridx lsp
                               ├─ OverlayManager (RAM): open buffers, live edits
                               ├─ WorkspaceIndex (mmap): shards of this workspace
                               └─ flycheck thread: cargo check on save
                                        │
              ┌─────────────────────────┴─────────────────────────┐
   ~/.cache/ridx/v5/objects/                          target/ridx/variants/<id>/
   dependencies, once per source hash and target      the workspace's own crates
   shared by every project                            + a manifest scoped to Cargo.lock
                                        ▲
                         ridx-driver (rustc wrapper inside cargo check)
```

The `.ridx` v5 shard is little-endian with 8-byte aligned, fixed-size records: a header, a string pool, the
file table (16 B per entry), definitions (40 B) with an FNV-1a hash index, owner and deref tables for
receiver chains, references sorted by span (48 B) with a target hash index, and type, impl, field and
generic tables. Lookups are binary searches over mapped memory; nothing is parsed into the heap, and the
shard files are closed once mapped.

## Known limitations

- **Answers come from the last build.** Code typed since then is answered from the open buffer and catches
  up when the project builds again; a crate that fails to build keeps its last good index.
- **One cfg.** Like `cargo check`, the index sees one target and one feature set; code behind other
  `#[cfg]`s or features is not indexed. Choose them with `ridx index --all-features` / `--target`.
- **Windows** is not supported yet.
- **Standard library**: navigation and hover into `std`, `core` and `alloc` work; references to them are
  searched in the workspace only, and they cannot be renamed.
- **Incoming calls** list a caller where the function is called (`name(` or `name::<…>(`); rust-analyzer
  also lists functions passed as values, such as a system given to `add_systems`.
- `vec![]` counts as a reference to `Vec`; rust-analyzer does not count it.

---

## Development

Every source file stays at or under 400 lines.

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.
