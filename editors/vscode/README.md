# Rust Intelligence for VS Code

Runs [`ridx lsp`](https://github.com/alex09x/rust-intelligence) as the language server of Rust files. ridx answers
from an index the compiler built (`ridx-driver` runs inside `rustc`), so types, paths and macro expansions are the
compiler's own. Completion, go to definition, references, hover, signature help, inlay hints, semantic highlighting,
diagnostics, formatting, rename and code actions all come from ridx over LSP; the first index of a workspace shows its
progress in the status bar.

## Install

Install the extension, open a Cargo workspace. If `ridx` is not found (the `rustIntelligence.server.path` setting,
then `PATH`, `~/.cargo/bin`, `~/.local/bin`), the extension offers to install it: it asks first, then downloads the
latest GitHub release for your platform, checks it against the release's SHA-256 and runs
`rustup toolchain install 1.98.1 --profile minimal --component rust-src` (the compiler `ridx-driver` links to; your
projects keep their own toolchain).

Prebuilt binaries exist for Linux on x86_64 and arm64, macOS on Apple Silicon and Intel, and Windows on x86_64. On
other platforms install from crates.io and restart the server:

```sh
cargo install ridx ridx-driver --locked
```

## With the rust-analyzer extension

Both extensions start a server for every Rust file, so completions and diagnostics appear twice. Disable the
rust-analyzer extension in workspaces where you use Rust Intelligence; the extension points you to it when both are
enabled. rust-analyzer's own features that are not LSP (run and debug lenses, Cargo tasks, syntax tree and macro
expansion views) are not part of this extension.

## Commands and settings

- **Rust Intelligence: Restart Server**
- **Rust Intelligence: Install or Update ridx** — installs the latest release into the extension's storage.
- `rustIntelligence.server.path` — a `ridx` of your own; `ridx-driver` must sit next to it or on `PATH`.
- `rustIntelligence.server.checkForUpdates` — at start, offer to update a `ridx` older than the latest release.
- `rustIntelligence.trace.server` — log the LSP traffic to the Rust Intelligence output.

Issues: https://github.com/alex09x/rust-intelligence/issues
