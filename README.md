<p align="center">
  <img src="site/assets/logo-minimal.svg" width="96" height="96" alt="ridx logo">
</p>

<h1 align="center">Rust Intelligence (<code>ridx</code>)</h1>

<p align="center">
  <b>Rust code intelligence at a tenth of the memory.</b><br>
  A language server for Rust that answers from what rustc already worked out.
</p>

<p align="center">
  <a href="https://rust-intelligence.com"><img src="https://img.shields.io/badge/site-rust--intelligence.com-f74c00" alt="Website"></a>
  <img src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-c9ced4" alt="License: MIT OR Apache-2.0">
  <img src="https://img.shields.io/badge/platform-Linux%20%7C%20macOS-1b1d20" alt="Platforms: Linux, macOS">
  <img src="https://img.shields.io/badge/protocol-LSP-1b1d20" alt="Language Server Protocol">
  <a href="https://doi.org/10.5281/zenodo.24379816"><img src="https://zenodo.org/badge/DOI/10.5281/zenodo.24379816.svg" alt="DOI: 10.5281/zenodo.24379816"></a>
</p>

<p align="center">
  <b>English</b> ·
  <a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.ru.md">Русский</a> ·
  <a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.zh-CN.md">简体中文</a> ·
  <a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.es.md">Español</a> ·
  <a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.pt-BR.md">Português (Brasil)</a> ·
  <a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.ja.md">日本語</a> ·
  <a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.de.md">Deutsch</a> ·
  <a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.fr.md">Français</a> ·
  <a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.ko.md">한국어</a> ·
  <a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.ar.md">العربية</a>
</p>

`ridx` records definitions, references and types once, during `cargo check`, into a binary index, and
reads that index straight from disk. Your editor gets navigation, hover, completion, references and rename
without a second compiler held in memory.

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

Linux (x86_64, arm64) and macOS on Apple Silicon, with [rustup](https://rustup.rs):

```bash
curl -fsSL https://rust-intelligence.com/install.sh | sh
```

The installer adds Rust 1.98.1, the toolchain `ridx-driver` runs inside (it links to that compiler's
libraries; your own projects keep any toolchain, ridx runs the index build with the right one), downloads
the release archive for your platform from GitHub, checks its SHA-256 and puts `ridx` and `ridx-driver`
into `~/.local/bin` (`RIDX_INSTALL_DIR` changes it, `RIDX_VERSION=v0.3.7` pins a release).

To build from source instead:

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
| Refactoring | `prepareRename`, `rename`, `codeAction` (compile-safe assists), `executeCommand` |
| Formatting | `formatting`, `rangeFormatting` |
| Symbols | `documentSymbol`, `workspace/symbol` |
| Diagnostics | `publishDiagnostics` from a background `cargo check` on save, pull diagnostics |
| Workspaces | `didChangeWorkspaceFolders`, `didChangeWatchedFiles`, multi-root |

## How it works

<p align="center"><img src="site/assets/architecture.svg" width="900" alt="How ridx works: at build time cargo check runs ridx-driver inside rustc and writes .ridx shards to disk; at edit time ridx lsp maps them and answers the editor"></p>

rustc already resolves every name, expands every macro and infers every type on each `cargo check`. `ridx`
keeps those answers instead of computing them again.

- **The compiler writes the index.** `ridx index` runs `cargo check` with `ridx-driver` as the rustc wrapper.
  After analysis the driver walks the HIR and rustc's type-check results and writes one `.ridx` shard per
  crate: definitions, every resolved reference (method calls and fields as rustc resolved them, through
  auto-deref and trait selection), types, impls, fields, generics, macros and `use` items. Compilation then
  goes on as usual, so the check itself is unchanged.
- **Dependencies are indexed once per machine.** A dependency's shard goes to a content-addressed store,
  keyed by crate, version, target, cfg flags and the digests of what it was compiled against; when a valid
  shard is there, the driver does not extract the crate again. Projects and worktrees share these shards,
  and `std`, `core` and `alloc` are indexed once per toolchain.
- **Reading is mapping, not parsing.** The v5 shard is little-endian, with fixed-size records in sections
  aligned to 8 bytes: a string pool, files (16 B), definitions (40 B) with an FNV-1a hash index, references
  sorted by span (48 B) with a target hash index, and type, impl, field and generic tables. A lookup is a
  hash probe, a binary search or a scan over mapped pages. Pages nobody asks for are never read, and the
  kernel can drop the others under memory pressure.
- **Rebuilds are incremental and safe.** After a save the server reindexes in the background; when only
  workspace sources changed, it calls the driver on the changed crates directly, without Cargo. A crate
  that fails to build keeps its last good shard, and another feature set or target gets its own variant
  next to the first.
- **Code typed between builds** is mapped onto the compiled text line by line, and the edited code is
  analyzed by a small compiler-free front end ([`crates/ridx-syntax`](crates/ridx-syntax): lexer, parser,
  name resolution, type inference) that reads every other type and item from the index.

The whole design, with the shard layout and where each part lives in the code:
[`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md).

## Known limitations

- **Answers come from the last build.** Code typed since then is answered from the open buffer and catches
  up when the project builds again; a crate that fails to build keeps its last good index.
- **One cfg.** Like `cargo check`, the index sees one target and one feature set; code behind other
  `#[cfg]`s or features is not indexed. Choose them with `ridx index --all-features` / `--target`.
- **Windows** is not supported yet.
- **Standard library**: navigation and hover into `std`, `core` and `alloc` work; references to them are
  searched in the workspace only, and they cannot be renamed.
- `vec![]` counts as a reference to `Vec`; rust-analyzer does not count it.

---

## Development

Every source file stays at or under 400 lines.

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## Citing

If you use ridx in research or documentation, please cite the archived release on Zenodo:
[10.5281/zenodo.24379816](https://doi.org/10.5281/zenodo.24379816) for the project (it resolves to the latest
version), [10.5281/zenodo.24379817](https://doi.org/10.5281/zenodo.24379817) for exactly 0.3.7.
[`CITATION.cff`](CITATION.cff) holds the metadata GitHub's "Cite this repository" uses.

```bibtex
@software{panasenko_2026_24379817,
  author    = {Panasenko, Alexander},
  title     = {Rust Intelligence (ridx) — a Rust language server answering from a compiler-built on-disk index},
  month     = nov,
  year      = 2026,
  publisher = {Zenodo},
  version   = {0.3.7},
  doi       = {10.5281/zenodo.24379817},
  url       = {https://doi.org/10.5281/zenodo.24379817},
}
```

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.
